-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: a versioned chain digest that covers every column of the record.
--
-- The digest of the tamper chain (audit.audit_event_digest) takes a fixed list
-- of the columns the table had when the chain was introduced. Every column
-- added since (the access fields, the categories, the authentication fields,
-- the emergency mark) was protected by the append-only trigger only, which the
-- table owner can disable, so the verifier could not see a change to them.
--
-- Each record now carries the digest version it was written under. Version 1
-- is the original digest, kept unchanged, so every record written before this
-- migration verifies exactly as it did. Version 2 hashes every column except
-- row_hash itself and the two delivery stamps, the one permitted change to a
-- stored row. A column added by a later migration is outside version 2: that
-- migration adds version 3 (a new branch in audit.audit_event_digest_columns
-- and a new audit.audit_chain_digest_version), and the integration test
-- audit_chain::the_current_digest_covers_every_column fails until it does.
--
-- Version 2 names its columns explicitly rather than hashing the whole row, so
-- adding a column never changes the digest of a record already written.
--
-- No openEHR spec governs audit storage mechanics: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

-- Existing records take 1 through the stored default (PostgreSQL 18 docs,
-- ALTER TABLE, "Notes": a non-volatile default is recorded once and returned
-- for existing rows, https://www.postgresql.org/docs/18/sql-altertable.html);
-- dropping the default afterwards keeps that value for them. New records get
-- their version from the chain trigger, never from the caller.
ALTER TABLE audit_event
    ADD COLUMN chain_digest_version smallint NOT NULL DEFAULT 1;
ALTER TABLE audit_event
    ALTER COLUMN chain_digest_version DROP DEFAULT;

COMMENT ON COLUMN audit_event.chain_digest_version IS
    'The chain digest version this record was hashed under (1: the original column list; 2: every column but row_hash and the delivery stamps), set by the insert trigger.';

COMMENT ON FUNCTION audit.audit_event_digest(bytea, bigint, uuid, timestamptz, timestamptz, text, smallint, text, text, text, text, text, text, text, text, jsonb) IS
    'The version 1 chain digest: SHA-256 over the predecessor link plus the columns the table had when the chain was introduced. Records with chain_digest_version = 1 are verified with it.';

-- ── the digest versions ──────────────────────────────────────────────────────

CREATE FUNCTION audit.audit_chain_digest_version() RETURNS smallint
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT 2::smallint $$;

COMMENT ON FUNCTION audit.audit_chain_digest_version() IS
    'The digest version the insert trigger writes new records under.';

-- Version 1 has no entry: its digest is positional and formats its own
-- timestamps, so it stays in audit.audit_event_digest.
CREATE FUNCTION audit.audit_event_digest_columns(p_version smallint) RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE p_version
        WHEN 2 THEN ARRAY[
            'id', 'recorded_at', 'stored_at', 'action', 'outcome', 'event_code',
            'operation', 'principal', 'patient_id', 'resource_class',
            'resource_id', 'client_ip', 'token_id', 'fhir',
            'chain_seq', 'prev_hash', 'chain_digest_version',
            'domain', 'purpose', 'legal_basis', 'result_count', 'request_id',
            'organisation', 'roles', 'origins', 'origin_count',
            'categories', 'category_basis', 'category_evidence',
            'category_map_digest',
            'acting_mode', 'assurance_level', 'assurance_value',
            'professional_id',
            'emergency_access'
        ]
    END
$$;

COMMENT ON FUNCTION audit.audit_event_digest_columns(smallint) IS
    'The columns a column-list chain digest version (2 and later) hashes; NULL for version 1 and for an unknown version.';

-- The hashed payload is a jsonb object of the listed columns rendered to text:
-- jsonb output is canonical (sorted keys, normalized numbers), and the SET
-- clauses fix the two settings a column's JSON rendering depends on, TimeZone
-- for timestamptz and bytea_output for bytea (PostgreSQL 18 docs, "JSON
-- Functions and Operators" and "Client Connection Defaults",
-- https://www.postgresql.org/docs/18/functions-json.html,
-- https://www.postgresql.org/docs/18/runtime-config-client.html). An unknown
-- version yields NULL, which the verifier reports as a modified record.
CREATE FUNCTION audit.audit_event_row_digest(p_row audit.audit_event) RETURNS bytea
    LANGUAGE sql STABLE PARALLEL SAFE
    SET search_path = pg_catalog
    SET TimeZone = 'UTC'
    SET bytea_output = 'hex'
    AS $$
    SELECT CASE
        WHEN p_row.chain_digest_version = 1 THEN audit.audit_event_digest(
            p_row.prev_hash, p_row.chain_seq, p_row.id, p_row.recorded_at,
            p_row.stored_at, p_row.action, p_row.outcome, p_row.event_code,
            p_row.operation, p_row.principal, p_row.patient_id,
            p_row.resource_class, p_row.resource_id, p_row.client_ip,
            p_row.token_id, p_row.fhir)
        ELSE (
            SELECT CASE WHEN count(*) = 0 THEN NULL ELSE
                sha256(convert_to(jsonb_build_array(
                    'ferroehr.audit_chain'::text,
                    p_row.chain_digest_version,
                    jsonb_object_agg(listed.name, whole.doc -> listed.name)
                )::text, 'UTF8'))
            END
              FROM to_jsonb(p_row) AS whole(doc),
                   unnest(audit.audit_event_digest_columns(p_row.chain_digest_version))
                   AS listed(name)
        )
    END
$$;

COMMENT ON FUNCTION audit.audit_event_row_digest(audit.audit_event) IS
    'The chain digest of one stored record under the version it records: the single definition the insert trigger and the verifier both call.';

-- ── the chain writer ─────────────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION audit.audit_event_chain_link() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.audit_chain_head', true);
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RAISE EXCEPTION 'the audit chain cursor is unset: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW.chain_seq := split_part(cursor_value, ':', 1)::bigint + 1;
    NEW.prev_hash := decode(split_part(cursor_value, ':', 2), 'hex');
    NEW.chain_digest_version := audit.audit_chain_digest_version();
    NEW.row_hash := audit.audit_event_row_digest(NEW);
    PERFORM set_config('ferroehr.audit_chain_head',
                       NEW.chain_seq || ':' || encode(NEW.row_hash, 'hex'), true);
    RETURN NEW;
END;
$$;

-- ── verification ─────────────────────────────────────────────────────────────

-- As before, plus one finding: a record whose digest version is older than its
-- predecessor's. Versions only rise along the chain, so a lower one is a
-- record moved to a digest that covers fewer columns.
CREATE OR REPLACE FUNCTION audit.verify_audit_chain()
    RETURNS TABLE (chain_seq bigint, record_id uuid, recorded_at timestamptz, finding text)
    LANGUAGE plpgsql STABLE
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    head_position bigint;
    head_digest   bytea;
    highest       bigint;
    highest_row   bytea;
BEGIN
    SELECT s.head_seq, s.head_hash INTO head_position, head_digest
      FROM audit.audit_chain_state s WHERE s.singleton;
    IF NOT FOUND THEN
        RETURN QUERY SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
            'the audit chain state row is missing: the chain cannot be verified'::text;
        RETURN;
    END IF;

    RETURN QUERY
    WITH linked AS (
        SELECT e.chain_seq, e.id, e.recorded_at, e.prev_hash, e.row_hash,
               e.chain_digest_version,
               lag(e.row_hash)  OVER (ORDER BY e.chain_seq) AS before_hash,
               coalesce(lag(e.chain_seq) OVER (ORDER BY e.chain_seq), 0) AS before_seq,
               lag(e.chain_digest_version) OVER (ORDER BY e.chain_seq) AS before_version,
               audit.audit_event_row_digest(e) AS recomputed
          FROM audit.audit_event e
    ),
    judged AS (
        SELECT l.*,
               l.row_hash IS DISTINCT FROM l.recomputed AS modified,
               coalesce(l.before_version > l.chain_digest_version, false) AS downgraded,
               l.chain_seq = l.before_seq + 1 AS adjacent,
               EXISTS (
                   SELECT 1 FROM audit.audit_chain_gap g
                    WHERE g.from_seq = l.before_seq + 1
                      AND g.to_seq = l.chain_seq - 1
                      AND g.link_hash = l.prev_hash
               ) AS gap_accounted
          FROM linked l
    )
    SELECT j.chain_seq, j.id, j.recorded_at,
           CASE
             WHEN j.modified
               THEN 'record content was modified after it was written'
             WHEN j.downgraded
               THEN 'record carries an older digest version than the record before it'
             WHEN NOT j.adjacent
               THEN 'record(s) deleted between chain position ' || j.before_seq
                    || ' and this one, with no retention record for the removal'
             ELSE 'the link to the preceding record does not match'
           END::text
      FROM judged j
     WHERE j.modified
        OR j.downgraded
        OR (NOT j.adjacent AND NOT j.gap_accounted)
        OR (j.adjacent AND j.prev_hash IS DISTINCT FROM
              coalesce(j.before_hash, audit.audit_chain_genesis()));

    SELECT e.chain_seq, e.row_hash INTO highest, highest_row
      FROM audit.audit_event e ORDER BY e.chain_seq DESC LIMIT 1;
    highest := coalesce(highest, 0);

    IF highest = head_position THEN
        IF highest > 0 AND highest_row IS DISTINCT FROM head_digest THEN
            RETURN QUERY SELECT highest, NULL::uuid, NULL::timestamptz,
                'the newest record does not match the recorded chain head'::text;
        END IF;
    ELSIF NOT EXISTS (
        SELECT 1 FROM audit.audit_chain_gap g
         WHERE g.from_seq = highest + 1 AND g.to_seq = head_position
           AND g.link_hash = head_digest
    ) THEN
        RETURN QUERY SELECT highest, NULL::uuid, NULL::timestamptz,
            'record(s) deleted from the end of the chain, with no retention record for the removal'::text;
    END IF;
END;
$$;
