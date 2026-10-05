-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the tamper-evidence hash chain over the record.
--
-- Every record carries its position in a gapless chain and a SHA-256 digest
-- over its predecessor's digest plus its own immutable content, so a record
-- that is altered or removed is detectable rather than merely forbidden. IHE
-- ITI TF-2 ATNA requires the repository to protect its records; the chain is
-- how this one does it. No openEHR spec governs audit storage mechanics: our
-- own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    ADD COLUMN chain_seq bigint NOT NULL,
    ADD COLUMN prev_hash bytea  NOT NULL,
    ADD COLUMN row_hash  bytea  NOT NULL,
    ADD CONSTRAINT uq_audit_event_chain_seq UNIQUE (chain_seq);

COMMENT ON COLUMN audit_event.chain_seq IS
    'Position in the tamper-evidence hash chain, assigned by the insert trigger under the chain-state row lock: gapless, so a missing number is a deleted record.';
COMMENT ON COLUMN audit_event.prev_hash IS
    'The row_hash of this record''s predecessor in the chain (the genesis digest for the first record).';
COMMENT ON COLUMN audit_event.row_hash IS
    'SHA-256 over prev_hash plus this record''s immutable content (audit.audit_event_digest); recomputed by audit.verify_audit_chain().';

-- ── chain state: the head and the retention low-water mark ───────────────────

CREATE TABLE audit_chain_state (
    -- One row, forever: the CHECK plus the primary key make a second row
    -- unrepresentable.
    singleton  boolean     NOT NULL DEFAULT true,
    -- The chain position and digest of the most recently written record, so a
    -- deletion at the END of the chain — where no successor would notice — is
    -- still detected.
    head_seq   bigint      NOT NULL DEFAULT 0,
    head_hash  bytea       NOT NULL,
    updated_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_audit_chain_state PRIMARY KEY (singleton),
    CONSTRAINT ck_audit_chain_state_singleton CHECK (singleton)
);

COMMENT ON TABLE audit_chain_state IS
    'The audit hash chain''s head: one row, written only by the SECURITY DEFINER chain trigger.';

-- Retention removes records, which by itself is indistinguishable from an
-- attacker removing them. A reaped span therefore leaves a tombstone recording
-- WHICH chain positions went and what digest the next surviving record must
-- link to — so an intact repository has a recorded reason for every gap, and an
-- unrecorded gap is tampering. Spans, not per-record rows, so ordinary
-- oldest-first reaping keeps exactly one tombstone however long the system
-- runs; a tombstone carries a position and a hash, never any record content.
CREATE TABLE audit_chain_gap (
    from_seq  bigint      NOT NULL,
    to_seq    bigint      NOT NULL,
    -- The row_hash of the LAST record in the span: what the next surviving
    -- record's prev_hash must equal.
    link_hash bytea       NOT NULL,
    reaped_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_audit_chain_gap PRIMARY KEY (from_seq),
    CONSTRAINT ck_audit_chain_gap_span CHECK (to_seq >= from_seq)
);

COMMENT ON TABLE audit_chain_gap IS
    'One row per span of chain positions removed by retention reaping — the recorded reason a gap in the chain is legitimate.';

-- ── the digest ───────────────────────────────────────────────────────────────

CREATE FUNCTION audit.audit_chain_genesis() RETURNS bytea
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT sha256(convert_to('ferroehr.audit_chain.v1', 'UTF8')) $$;

COMMENT ON FUNCTION audit.audit_chain_genesis() IS
    'The fixed digest the first record in the chain links back to.';

-- The hashed payload is a jsonb array rendered to text: jsonb output is
-- canonical (normalized numbers, sorted keys, no insignificant whitespace) and
-- distinguishes a JSON null from an empty string, so no field boundary is
-- ambiguous. Timestamps are formatted explicitly in UTC with a numeric-only
-- pattern, so neither the session TimeZone nor lc_time can change the digest
-- (PostgreSQL 18 docs, "Data Type Formatting Functions":
-- https://www.postgresql.org/docs/18/functions-formatting.html).
--
-- The mutable delivery stamps are deliberately NOT hashed: stamping a record as
-- forwarded is the one permitted change to a stored row.
CREATE FUNCTION audit.audit_event_digest(
    p_prev_hash      bytea,
    p_chain_seq      bigint,
    p_id             uuid,
    p_recorded_at    timestamptz,
    p_stored_at      timestamptz,
    p_action         text,
    p_outcome        smallint,
    p_event_code     text,
    p_operation      text,
    p_principal      text,
    p_patient_id     text,
    p_resource_class text,
    p_resource_id    text,
    p_client_ip      text,
    p_token_id       text,
    p_fhir           jsonb
) RETURNS bytea
    LANGUAGE sql STABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT sha256(convert_to(jsonb_build_array(
        encode(p_prev_hash, 'hex'),
        p_chain_seq,
        p_id,
        to_char(p_recorded_at AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
        to_char(p_stored_at   AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS.US"Z"'),
        p_action,
        p_outcome,
        p_event_code,
        p_operation,
        p_principal,
        p_patient_id,
        p_resource_class,
        p_resource_id,
        p_client_ip,
        p_token_id,
        p_fhir
    )::text, 'UTF8'))
$$;

COMMENT ON FUNCTION audit.audit_event_digest(bytea, bigint, uuid, timestamptz, timestamptz, text, smallint, text, text, text, text, text, text, text, text, jsonb) IS
    'SHA-256 over the predecessor link plus one record''s immutable content; the single definition the insert trigger and the verifier both call.';

-- ── seed ─────────────────────────────────────────────────────────────────────

INSERT INTO audit_chain_state (singleton, head_hash)
VALUES (true, audit.audit_chain_genesis());

-- ── the chain writer ─────────────────────────────────────────────────────────

-- The chain is written by three triggers, and the split is a throughput
-- decision rather than a stylistic one. Advancing the head by UPDATEing the
-- single state row PER RECORD costs quadratic time inside one transaction: each
-- update leaves another tuple version on the same page, and every subsequent
-- read walks the whole chain of them. Measured on PostgreSQL 18 over a 2000
-- record batch, that one statement was ~60% of the total insert time. So the
-- head is carried per STATEMENT in a transaction-local setting, and the state
-- row is read once at the start of a statement and written once at its end.
--
-- The setting is a cache, never an input: the BEFORE STATEMENT trigger
-- OVERWRITES it from the state row every time, so a caller who sets it by hand
-- has no effect on the chain a subsequent insert builds.
--
-- The FOR UPDATE on the state row is what makes the chain a total order:
-- concurrent inserting transactions serialize on it, and a rolled-back
-- transaction rolls its head advance back too, so chain_seq is gapless by
-- construction and a missing number means a deleted record.
--
-- SECURITY DEFINER on the two statement triggers so the runtime role can insert
-- records without holding any write privilege on the chain state (PostgreSQL 18
-- docs, CREATE FUNCTION §"Writing SECURITY DEFINER Functions Safely":
-- https://www.postgresql.org/docs/18/sql-createfunction.html).
CREATE FUNCTION audit.audit_event_chain_begin() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    head_position bigint;
    head_digest   bytea;
BEGIN
    SELECT head_seq, head_hash INTO head_position, head_digest
      FROM audit.audit_chain_state
     WHERE singleton
       FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'audit chain state is missing: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    PERFORM set_config('ferroehr.audit_chain_head',
                       head_position || ':' || encode(head_digest, 'hex'), true);
    RETURN NULL;
END;
$$;

CREATE TRIGGER audit_event_chain_begin
    BEFORE INSERT ON audit_event
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_event_chain_begin();

CREATE FUNCTION audit.audit_event_chain_link() RETURNS trigger
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
    NEW.row_hash := audit.audit_event_digest(
        NEW.prev_hash, NEW.chain_seq, NEW.id, NEW.recorded_at, NEW.stored_at,
        NEW.action, NEW.outcome, NEW.event_code, NEW.operation, NEW.principal,
        NEW.patient_id, NEW.resource_class, NEW.resource_id, NEW.client_ip,
        NEW.token_id, NEW.fhir);
    PERFORM set_config('ferroehr.audit_chain_head',
                       NEW.chain_seq || ':' || encode(NEW.row_hash, 'hex'), true);
    RETURN NEW;
END;
$$;

CREATE TRIGGER audit_event_chain_link
    BEFORE INSERT ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit.audit_event_chain_link();

CREATE FUNCTION audit.audit_event_chain_commit() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.audit_chain_head', true);
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RETURN NULL;
    END IF;
    UPDATE audit.audit_chain_state
       SET head_seq = split_part(cursor_value, ':', 1)::bigint,
           head_hash = decode(split_part(cursor_value, ':', 2), 'hex'),
           updated_at = now()
     WHERE singleton;
    RETURN NULL;
END;
$$;

CREATE TRIGGER audit_event_chain_commit
    AFTER INSERT ON audit_event
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_event_chain_commit();

-- ── append-only enforcement ──────────────────────────────────────────────────

-- Comparing the whole row minus the two delivery stamps fails CLOSED: a column
-- added later is immutable unless this exclusion list says otherwise.
CREATE FUNCTION audit.audit_event_reject_mutation() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF to_jsonb(NEW) - 'delivered_syslog_at' - 'delivered_fhir_feed_at'
       IS DISTINCT FROM
       to_jsonb(OLD) - 'delivered_syslog_at' - 'delivered_fhir_feed_at'
    THEN
        RAISE EXCEPTION
            'audit.audit_event is append-only: only delivered_syslog_at and delivered_fhir_feed_at may change (record %)',
            OLD.id
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE TRIGGER audit_event_reject_mutation
    BEFORE UPDATE ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit.audit_event_reject_mutation();

-- Deletion is legitimate only as retention reaping, which is prefix-only and
-- records its low-water mark. The transaction-local flag is set exclusively by
-- audit.reap_audit_events().
CREATE FUNCTION audit.audit_event_reject_deletion() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF coalesce(current_setting('ferroehr.audit_reaping', true), 'off') <> 'on' THEN
        RAISE EXCEPTION
            'audit.audit_event records are deleted only by audit.reap_audit_events() (record %)',
            OLD.id
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;

CREATE TRIGGER audit_event_reject_deletion
    BEFORE DELETE ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit.audit_event_reject_deletion();

-- TRUNCATE bypasses row triggers entirely, so it needs its own statement-level
-- refusal (PostgreSQL 18 docs, CREATE TRIGGER:
-- https://www.postgresql.org/docs/18/sql-createtrigger.html).
CREATE FUNCTION audit.audit_event_reject_truncate() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.audit_event cannot be truncated: it is the tamper-evident audit trail'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER audit_event_reject_truncate
    BEFORE TRUNCATE ON audit_event
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_event_reject_truncate();

-- ── verification ─────────────────────────────────────────────────────────────

-- The operator-runnable check. Empty result = the repository is intact; every
-- returned row names one record (or one boundary) and what is wrong with it.
-- Runnable straight from psql: SELECT * FROM audit.verify_audit_chain();
CREATE FUNCTION audit.verify_audit_chain()
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

    -- Every surviving record, against its own digest and against whatever
    -- precedes it: the record before it, a recorded reaping tombstone, or (at
    -- position 1) the genesis digest. A gap with no tombstone is a deletion
    -- nobody recorded.
    RETURN QUERY
    WITH linked AS (
        SELECT e.chain_seq, e.id, e.recorded_at, e.prev_hash, e.row_hash,
               lag(e.row_hash)  OVER (ORDER BY e.chain_seq) AS before_hash,
               coalesce(lag(e.chain_seq) OVER (ORDER BY e.chain_seq), 0) AS before_seq,
               audit.audit_event_digest(
                   e.prev_hash, e.chain_seq, e.id, e.recorded_at, e.stored_at,
                   e.action, e.outcome, e.event_code, e.operation, e.principal,
                   e.patient_id, e.resource_class, e.resource_id, e.client_ip,
                   e.token_id, e.fhir) AS recomputed
          FROM audit.audit_event e
    ),
    judged AS (
        SELECT l.*,
               l.row_hash IS DISTINCT FROM l.recomputed AS modified,
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
             WHEN NOT j.adjacent
               THEN 'record(s) deleted between chain position ' || j.before_seq
                    || ' and this one, with no retention record for the removal'
             ELSE 'the link to the preceding record does not match'
           END::text
      FROM judged j
     WHERE j.modified
        OR (NOT j.adjacent AND NOT j.gap_accounted)
        OR (j.adjacent AND j.prev_hash IS DISTINCT FROM
              coalesce(j.before_hash, audit.audit_chain_genesis()));

    -- The end of the chain has no successor to notice a deletion, so it is
    -- checked against the recorded head: either the newest surviving record IS
    -- the head, or a tombstone accounts for everything between them.
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

COMMENT ON FUNCTION audit.verify_audit_chain() IS
    'Tamper check over the whole repository: returns one row per damaged record or boundary, and nothing at all when the trail is intact.';
