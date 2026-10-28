-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the record keeps its facts in columns, and the FHIR AuditEvent is
-- rendered from them when a record is read or fed.
--
-- Until now every record stored the full FHIR R4 AuditEvent in the fhir
-- column, about 2.7 KB of the 3.2 KB a record took on disk, repeating facts
-- the columns already held. The column goes. What the rendering needs beside
-- the columns the table had is added: the token's issuer and client, the EHR
-- a classified access names, the subject's namespace and whether its lookup
-- failed, and what the category classification read (category_content). What the
-- rendering takes from the configuration (the audit source, the store horizon,
-- the category map and the emergency purposes) is kept once per distinct
-- configuration in audit_render_policy, content-addressed by its SHA-256, and
-- each record names the policy it was written under (render_policy).
--
-- The chain digest is one digest over every column but row_hash and the two
-- delivery stamps. The versioned digests of 0003, 0011, 0013 and 0016 are
-- retired with the records they covered: no installation holds records this
-- migration has to keep verifying (owner decision 2026-10-28, #4300), and it
-- refuses a store that does.
--
-- IHE BALP and ITI-20 define the content of the AuditEvent, not how a
-- repository stores it: storing columns and rendering on read is our own
-- design. No openEHR spec governs audit storage mechanics.
--
-- Runs with search_path = audit, ext, public.

DO $empty$
BEGIN
    IF EXISTS (SELECT 1 FROM audit_event) THEN
        RAISE EXCEPTION 'audit.audit_event holds records written before migration 0017, whose stored AuditEvent this migration drops'
            USING ERRCODE = 'object_not_in_prerequisite_state',
                  HINT = 'FerroEHR 5.6.0 starts on an empty audit store: export what you need over ITI-81 with the earlier release, then drop the audit schema and start again.';
    END IF;
END
$empty$;

-- ── the render policy ────────────────────────────────────────────────────────

CREATE TABLE audit_render_policy (
    -- SHA-256 over the policy's jsonb text, which jsonb writes canonically
    -- (sorted keys, no insignificant whitespace).
    digest    bytea       NOT NULL,
    policy    jsonb       NOT NULL,
    stored_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_audit_render_policy PRIMARY KEY (digest),
    CONSTRAINT ck_audit_render_policy_digest
        CHECK (digest = sha256(convert_to(policy::text, 'UTF8')))
);

COMMENT ON TABLE audit_render_policy IS
    'The configuration the FHIR AuditEvent of a record is rendered under (audit source, store horizon, category map, emergency purposes), one row per distinct configuration, keyed by the SHA-256 of its text; append-only.';

-- The runtime role registers the policy it writes under through this function
-- and holds no INSERT on the table, so it can add a policy and never alter one.
CREATE FUNCTION audit.register_render_policy(p_policy jsonb) RETURNS bytea
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    named bytea := sha256(convert_to(p_policy::text, 'UTF8'));
BEGIN
    INSERT INTO audit.audit_render_policy (digest, policy)
    VALUES (named, p_policy)
    ON CONFLICT (digest) DO NOTHING;
    RETURN named;
END;
$$;

COMMENT ON FUNCTION audit.register_render_policy(jsonb) IS
    'Stores a render policy unless it is already stored, and returns its digest.';

CREATE FUNCTION audit.audit_render_policy_reject_change() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.audit_render_policy is append-only: the records that name a policy are rendered under it'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER audit_render_policy_reject_change
    BEFORE UPDATE OR DELETE ON audit_render_policy
    FOR EACH ROW EXECUTE FUNCTION audit.audit_render_policy_reject_change();

CREATE TRIGGER audit_render_policy_reject_truncate
    BEFORE TRUNCATE ON audit_render_policy
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_render_policy_reject_change();

-- ── the record ───────────────────────────────────────────────────────────────

ALTER TABLE audit_event
    DROP COLUMN fhir,
    DROP COLUMN chain_digest_version,
    ADD COLUMN issuer                text,
    ADD COLUMN client_id             text,
    ADD COLUMN ehr_id                text,
    ADD COLUMN subject_namespace     text,
    ADD COLUMN subject_lookup_failed boolean NOT NULL DEFAULT false,
    ADD COLUMN category_content      jsonb,
    ADD COLUMN render_policy         bytea   NOT NULL,
    ADD CONSTRAINT fk_audit_event_render_policy
        FOREIGN KEY (render_policy) REFERENCES audit_render_policy (digest),
    ADD CONSTRAINT ck_audit_event_category_content
        CHECK ((category_content IS NULL) = (categories IS NULL));

COMMENT ON TABLE audit_event IS
    'The local IHE ATNA Audit Record Repository: one row per audit record, whose FHIR R4 AuditEvent (IHE BALP shape) is rendered from the columns under the record''s render policy. Strictly outside the EHR content (BASE architecture_overview/master07-security.adoc §Access logging); append-only except per-sink delivery stamps and retention reaping.';
COMMENT ON COLUMN audit_event.issuer IS
    'The issuer (iss) of the bearer token that authenticated the caller; NULL for a Basic-authenticated or anonymous caller.';
COMMENT ON COLUMN audit_event.client_id IS
    'The client application the bearer token was issued to (client_id or azp); NULL when the token names none.';
COMMENT ON COLUMN audit_event.ehr_id IS
    'The EHR a classified access reached, which its EHDS access record names; NULL on every other record, so a linkage record never pairs a party with its EHR.';
COMMENT ON COLUMN audit_event.subject_namespace IS
    'The namespace of the subject id in patient_id, as EHR_STATUS.subject names it.';
COMMENT ON COLUMN audit_event.subject_lookup_failed IS
    'Whether the subject of ehr_id could not be looked up when the record was written.';
COMMENT ON COLUMN audit_event.category_content IS
    'What the category classification read (the objects with their template and archetype ids, or the query constraints), from which the classification is computed again under the render policy''s category map; NULL exactly when categories is NULL.';
COMMENT ON COLUMN audit_event.render_policy IS
    'The digest of the audit_render_policy row the record is rendered under.';
COMMENT ON COLUMN audit_event.row_hash IS
    'SHA-256 over every column of the record but row_hash and the delivery stamps (audit.audit_event_row_digest); recomputed by audit.verify_audit_chain().';

-- ── the digest ───────────────────────────────────────────────────────────────

DROP FUNCTION audit.audit_event_digest(bytea, bigint, uuid, timestamptz, timestamptz, text, smallint, text, text, text, text, text, text, text, text, jsonb);
DROP FUNCTION audit.audit_event_digest_columns(smallint);
DROP FUNCTION audit.audit_chain_digest_version();

-- Every column but row_hash itself and the two delivery stamps, the one
-- permitted change to a stored row. A column added later is outside the
-- digest until it is listed here, and the integration test
-- audit_chain::the_digest_covers_every_column fails until it is.
CREATE FUNCTION audit.audit_event_digest_columns() RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT ARRAY[
        'id', 'recorded_at', 'stored_at', 'action', 'outcome', 'event_code',
        'operation', 'principal', 'patient_id', 'resource_class',
        'resource_id', 'client_ip', 'token_id',
        'chain_seq', 'prev_hash',
        'domain', 'purpose', 'legal_basis', 'result_count', 'request_id',
        'organisation', 'roles', 'origins', 'origin_count',
        'categories', 'category_basis', 'category_evidence',
        'category_map_digest',
        'acting_mode', 'assurance_level', 'assurance_value',
        'professional_id',
        'emergency_access',
        'retention_ends', 'retention_stated',
        'permit',
        'issuer', 'client_id', 'ehr_id', 'subject_namespace',
        'subject_lookup_failed', 'category_content', 'render_policy'
    ]
$$;

COMMENT ON FUNCTION audit.audit_event_digest_columns() IS
    'The columns the chain digest hashes: every column but row_hash and the delivery stamps.';

-- The hashed payload is a jsonb object of the listed columns rendered to text:
-- jsonb output is canonical (sorted keys, normalized numbers), and the SET
-- clauses fix the two settings a column's JSON rendering depends on, TimeZone
-- for timestamptz and bytea_output for bytea (PostgreSQL 18 docs, "JSON
-- Functions and Operators" and "Client Connection Defaults",
-- https://www.postgresql.org/docs/18/functions-json.html,
-- https://www.postgresql.org/docs/18/runtime-config-client.html).
CREATE OR REPLACE FUNCTION audit.audit_event_row_digest(p_row audit.audit_event) RETURNS bytea
    LANGUAGE sql STABLE PARALLEL SAFE
    SET search_path = pg_catalog
    SET TimeZone = 'UTC'
    SET bytea_output = 'hex'
    AS $$
    SELECT sha256(convert_to(jsonb_build_array(
               'ferroehr.audit_chain'::text,
               jsonb_object_agg(listed.name, whole.doc -> listed.name)
           )::text, 'UTF8'))
      FROM to_jsonb(p_row) AS whole(doc),
           unnest(audit.audit_event_digest_columns()) AS listed(name)
$$;

COMMENT ON FUNCTION audit.audit_event_row_digest(audit.audit_event) IS
    'The chain digest of one stored record: the single definition the insert trigger and the verifier both call.';

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
    NEW.row_hash := audit.audit_event_row_digest(NEW);
    PERFORM set_config('ferroehr.audit_chain_head',
                       NEW.chain_seq || ':' || encode(NEW.row_hash, 'hex'), true);
    RETURN NEW;
END;
$$;

-- ── verification ─────────────────────────────────────────────────────────────

ALTER TABLE audit_chain_checkpoint DROP COLUMN through_version;

DROP FUNCTION audit.verify_audit_chain_incremental(boolean);
DROP FUNCTION audit.verify_audit_chain();
DROP FUNCTION audit.verify_audit_chain_after(bigint, bytea, smallint);

-- The verification from the link (p_after_seq, p_after_hash) on: every record
-- after it is recomputed and re-linked, both ends are checked against the
-- chain state, and every render policy against its digest. From (0, genesis)
-- it is the full pass. STABLE, so every statement reads the snapshot of the
-- calling query (PostgreSQL 18 docs, "Function Volatility Categories",
-- https://www.postgresql.org/docs/18/xfunc-volatility.html).
CREATE FUNCTION audit.verify_audit_chain_after(p_after_seq bigint, p_after_hash bytea)
    RETURNS SETOF audit.audit_chain_finding
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

    -- A record is rendered under its policy, so a changed policy changes what
    -- every record naming it says.
    RETURN QUERY
    SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
           ('the render policy ' || encode(p.digest, 'hex')
            || ' was modified after it was written')::text
      FROM audit.audit_render_policy p
     WHERE p.digest IS DISTINCT FROM sha256(convert_to(p.policy::text, 'UTF8'))
     ORDER BY p.digest;

    RETURN QUERY
    WITH linked AS (
        SELECT e.chain_seq, e.id, e.recorded_at, e.prev_hash, e.row_hash,
               coalesce(lag(e.row_hash) OVER (ORDER BY e.chain_seq), p_after_hash)
                   AS before_hash,
               coalesce(lag(e.chain_seq) OVER (ORDER BY e.chain_seq), p_after_seq)
                   AS before_seq,
               audit.audit_event_row_digest(e) AS recomputed,
               EXISTS (SELECT 1 FROM audit.audit_render_policy p
                        WHERE p.digest = e.render_policy) AS policy_present
          FROM audit.audit_event e
         WHERE e.chain_seq > p_after_seq
    ),
    judged AS (
        SELECT l.*,
               l.row_hash IS DISTINCT FROM l.recomputed AS modified,
               l.chain_seq = l.before_seq + 1 AS adjacent,
               -- Reaping merges adjacent tombstones, so once the checkpoint
               -- record itself is reaped the span after it starts earlier.
               EXISTS (
                   SELECT 1 FROM audit.audit_chain_gap g
                    WHERE (g.from_seq = l.before_seq + 1
                           OR (l.before_seq = p_after_seq AND p_after_seq > 0
                               AND g.from_seq <= l.before_seq + 1))
                      AND g.to_seq = l.chain_seq - 1
                      AND g.link_hash = l.prev_hash
               ) AS gap_accounted
          FROM linked l
    )
    SELECT j.chain_seq, j.id, j.recorded_at,
           CASE
             WHEN j.modified
               THEN 'record content was modified after it was written'
             WHEN NOT j.policy_present
               THEN 'the render policy the record names is missing'
             WHEN NOT j.adjacent
               THEN 'record(s) deleted between chain position ' || j.before_seq
                    || ' and this one, with no retention record for the removal'
             ELSE 'the link to the preceding record does not match'
           END::text
      FROM judged j
     WHERE j.modified
        OR NOT j.policy_present
        OR (NOT j.adjacent AND NOT j.gap_accounted)
        OR (j.adjacent AND j.prev_hash IS DISTINCT FROM j.before_hash);

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

COMMENT ON FUNCTION audit.verify_audit_chain_after(bigint, bytea) IS
    'The chain verification from one verified link on: the body of both the full and the incremental verification.';

CREATE FUNCTION audit.verify_audit_chain()
    RETURNS TABLE (chain_seq bigint, record_id uuid, recorded_at timestamptz, finding text)
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT f.chain_seq, f.record_id, f.recorded_at, f.finding
      FROM audit.verify_audit_chain_after(0, audit.audit_chain_genesis()) f
$$;

COMMENT ON FUNCTION audit.verify_audit_chain() IS
    'Tamper check over the whole repository: returns one row per damaged record, boundary or render policy, and nothing at all when the trail is intact.';

-- One statement reads the findings and the newest record, so both come from
-- one snapshot and the checkpoint never passes a record the run did not see.
-- Concurrent runs (one per replica) only ever move the checkpoint forward.
CREATE FUNCTION audit.verify_audit_chain_incremental(p_full boolean DEFAULT false)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    from_seq  bigint := 0;
    from_hash bytea := audit.audit_chain_genesis();
    damage    audit.audit_chain_finding[];
    top_seq   bigint;
    top_hash  bytea;
BEGIN
    IF NOT coalesce(p_full, false) THEN
        SELECT c.through_seq, c.through_hash INTO from_seq, from_hash
          FROM audit.audit_chain_checkpoint c WHERE c.singleton;
        IF NOT FOUND THEN
            from_seq := 0;
            from_hash := audit.audit_chain_genesis();
        END IF;
    END IF;

    SELECT (SELECT coalesce(array_agg(f ORDER BY f.chain_seq NULLS FIRST), '{}')
              FROM audit.verify_audit_chain_after(from_seq, from_hash) f),
           newest.chain_seq, newest.row_hash
      INTO damage, top_seq, top_hash
      FROM (SELECT 1) AS one
      LEFT JOIN LATERAL (
          SELECT e.chain_seq, e.row_hash
            FROM audit.audit_event e ORDER BY e.chain_seq DESC LIMIT 1
      ) AS newest ON true;

    IF cardinality(damage) = 0 AND top_seq IS NOT NULL THEN
        INSERT INTO audit.audit_chain_checkpoint AS c
               (through_seq, through_hash, verified_at, full_verified_at)
        VALUES (top_seq, top_hash, now(), CASE WHEN p_full THEN now() END)
        ON CONFLICT (singleton) DO UPDATE
           SET through_seq = excluded.through_seq,
               through_hash = excluded.through_hash,
               verified_at = excluded.verified_at,
               full_verified_at = coalesce(excluded.full_verified_at, c.full_verified_at)
         WHERE excluded.through_seq >= c.through_seq;
    END IF;

    RETURN QUERY SELECT * FROM unnest(damage);
END;
$$;

COMMENT ON FUNCTION audit.verify_audit_chain_incremental(boolean) IS
    'Verifies the records written after the checkpoint (every record when p_full), the chain head and the render policies, and moves the checkpoint to the newest record when the run finds nothing; returns the findings as audit.verify_audit_chain() does.';

-- ── grants ───────────────────────────────────────────────────────────────────

-- Functions are executable by PUBLIC unless revoked (PostgreSQL 18, GRANT,
-- "Notes", https://www.postgresql.org/docs/18/sql-grant.html).
REVOKE ALL ON FUNCTION audit.register_render_policy(jsonb) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.verify_audit_chain_incremental(boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.verify_audit_chain_after(bigint, bytea) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON audit_render_policy
            FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON audit_render_policy
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.register_render_policy(jsonb)
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.verify_audit_chain()
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.verify_audit_chain_after(bigint, bytea)
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.verify_audit_chain_incremental(boolean)
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping audit render policy grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
