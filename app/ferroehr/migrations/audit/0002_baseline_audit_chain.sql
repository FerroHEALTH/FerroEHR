-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the tamper-evidence hash chain of the system records.
--
-- The head of the chain, the gaps retention reaping leaves, the checkpoint an
-- incremental verification starts from, the signed seals, and the
-- verification functions. A later link commits to every earlier one, so a
-- change to a stored record breaks every link after it.
--
-- No openEHR spec governs audit tamper detection: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

-- ── the digest ───────────────────────────────────────────────────────────────
CREATE FUNCTION audit.audit_chain_genesis() RETURNS bytea
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT sha256(convert_to('ferroehr.audit_chain.v1', 'UTF8')) $$;
COMMENT ON FUNCTION audit_chain_genesis() IS
    'The fixed digest the first record in the chain links back to.';

-- The links and checkpoints an import brought never change, and a record
-- changes only by its replacement with its own tombstone.
CREATE FUNCTION audit.access_log_imported_append_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION '% is append-only', TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- ── chain state: the head and the retention low-water mark ───────────────────
-- Every update of the head row stays HOT with room on the page, and the
-- vacuum of a table that only ever holds one live row runs after a fixed
-- number of updates, so the row does not bloat.
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
) WITH (fillfactor = 10, autovacuum_vacuum_scale_factor = 0,
       autovacuum_vacuum_threshold = 1000);
INSERT INTO audit_chain_state (singleton, head_hash)
VALUES (true, audit.audit_chain_genesis());
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

-- ── audit_chain_checkpoint ───────────────────────────────────────────────────
-- The chain makes a later link commit to every earlier one, so a verification
-- run that starts from a link an earlier run verified only recomputes the
-- records written since. This row keeps that link: its position and its
-- digest. A change to a record before the checkpoint is outside such a run,
-- so the full pass (audit.verify_audit_chain()) stays. No openEHR spec
-- governs audit tamper detection: our own design/extension.
CREATE TABLE audit_chain_checkpoint (
    singleton        boolean     NOT NULL DEFAULT true,
    through_seq      bigint      NOT NULL,
    through_hash     bytea       NOT NULL,
    verified_at      timestamptz NOT NULL,
    full_verified_at timestamptz,
    CONSTRAINT pk_audit_chain_checkpoint PRIMARY KEY (singleton),
    CONSTRAINT ck_audit_chain_checkpoint_singleton CHECK (singleton)
);
COMMENT ON TABLE audit_chain_checkpoint IS
    'The newest chain link a verification found intact, with every link before it: where an incremental verification starts. One row, written only by audit.verify_audit_chain_step().';

-- A seal fixes the head of the system chain as an ACCESS_LOG_CHECKPOINT fixes
-- the access log's: its position and link digest, signed by the system's
-- key. The record a seal names keeps its row_hash, or, once retention reaped
-- it, the gap that ends at it keeps the same digest as its link.
CREATE TABLE audit_chain_seal (
    sequence    bigint      NOT NULL,
    digest      text        NOT NULL,
    time_sealed timestamptz NOT NULL,
    content     text        NOT NULL,
    CONSTRAINT pk_audit_chain_seal PRIMARY KEY (sequence),
    CONSTRAINT ck_audit_chain_seal_digest CHECK (digest ~ '^[0-9a-f]{64}$')
);
CREATE TRIGGER audit_chain_seal_append_only
    BEFORE UPDATE OR DELETE ON audit_chain_seal
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
CREATE TRIGGER audit_chain_seal_no_truncate
    BEFORE TRUNCATE ON audit_chain_seal
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_imported_append_only();
COMMENT ON TABLE audit_chain_seal IS
    'The signed seals of the system chain (audit_event), one per sealed position, append-only.';
COMMENT ON COLUMN audit_chain_seal.content IS
    'The seal''s JSON form, proof included.';

CREATE TYPE audit_chain_finding AS (
    chain_seq   bigint,
    record_id   uuid,
    recorded_at timestamptz,
    finding     text
);

-- Link the records of up to p_limit released intake rows, oldest first, in
-- the order each row names them; the link's triggers chain them. Rows another
-- replica holds are skipped, and each row leaves in the same statement.
CREATE FUNCTION audit.audit_event_chain_move(p_limit integer, p_fhir_feed boolean, p_syslog boolean)
    RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    moved bigint;
BEGIN
    PERFORM set_config('ferroehr.audit_chaining', 'on', true);
    WITH taken AS (
        DELETE FROM audit.audit_event_intake i
         WHERE i.id IN (SELECT t.id FROM audit.audit_event_intake t
                         WHERE t.reserved_by IS NULL
                         ORDER BY t.id LIMIT p_limit FOR UPDATE SKIP LOCKED)
        RETURNING i.id, i.events
    ),
    linked AS (
        INSERT INTO audit.audit_event_link (event_id)
        SELECT u.event_id
          FROM taken t, unnest(t.events) WITH ORDINALITY AS u(event_id, n)
         ORDER BY t.id, u.n
        RETURNING chain_seq
    ),
    fed AS (
        INSERT INTO audit.fhir_feed_outbox (chain, chain_seq)
        SELECT 'system', l.chain_seq FROM linked l WHERE p_fhir_feed ORDER BY l.chain_seq
        RETURNING 1
    ),
    logged AS (
        INSERT INTO audit.syslog_outbox (chain, chain_seq)
        SELECT 'system', l.chain_seq FROM linked l WHERE p_syslog ORDER BY l.chain_seq
        RETURNING 1
    )
    SELECT count(*) INTO moved FROM linked;
    PERFORM set_config('ferroehr.audit_chaining', 'off', true);
    RETURN moved;
END;
$$;
COMMENT ON FUNCTION audit_event_chain_move(integer, boolean, boolean) IS
    'The one path a record of no access-log entry enters the system chain by: links the records of up to p_limit released intake rows, oldest first, with their outbox rows; the intake rows leave in the same statement.';

-- The link digest of the system chain at a position, in lower-case
-- hexadecimal: the record's row_hash, or the link of the retention gap that
-- ends there; NULL when neither is there.
CREATE FUNCTION audit.audit_chain_link_at(p_sequence bigint) RETURNS text
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT coalesce(
        (SELECT encode(l.row_hash, 'hex') FROM audit.audit_event_link l WHERE l.chain_seq = p_sequence),
        (SELECT encode(g.link_hash, 'hex') FROM audit.audit_chain_gap g WHERE g.to_seq = p_sequence))
$$;
COMMENT ON FUNCTION audit_chain_link_at(bigint) IS
    'The link digest of the system chain at a position: what a seal of that position fixes.';

CREATE FUNCTION audit.audit_chain_verification_link(p_sequence bigint)
    RETURNS text
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT CASE WHEN p_sequence = 0 THEN encode(audit.audit_chain_genesis(), 'hex')
                ELSE audit.audit_chain_link_at(p_sequence) END
$$;
COMMENT ON FUNCTION audit_chain_verification_link(bigint) IS
    'The link of the system chain at p_sequence, the genesis at 0: where the next step of a verification walk starts.';

CREATE FUNCTION audit.audit_chain_verification_start(p_full boolean)
    RETURNS TABLE (head_seq bigint, after_seq bigint, after_digest text)
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT (SELECT s.head_seq FROM audit.audit_chain_state s WHERE s.singleton),
           coalesce(c.through_seq, 0),
           encode(coalesce(c.through_hash, audit.audit_chain_genesis()), 'hex')
      FROM (SELECT 1) AS one
      LEFT JOIN audit.audit_chain_checkpoint c ON c.singleton AND NOT p_full
$$;
COMMENT ON FUNCTION audit_chain_verification_start(boolean) IS
    'Where a verification walk of the system chain starts: the chain head it walks to, and the verified link it starts from (the genesis when p_full), in hexadecimal.';

CREATE FUNCTION audit.audit_chain_verification_step_end(
    p_after bigint, p_records integer, p_head bigint)
    RETURNS bigint
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT greatest(p_after, coalesce(
        (SELECT l.chain_seq FROM audit.audit_event_link l
          WHERE l.chain_seq > p_after AND l.chain_seq < p_head
          ORDER BY l.chain_seq OFFSET greatest(p_records, 1) - 1 LIMIT 1),
        p_head))
$$;
COMMENT ON FUNCTION audit_chain_verification_step_end(bigint, integer, bigint) IS
    'Where the verification window of the system chain after p_after ends: its p_records-th record, or p_head when fewer are left.';

CREATE FUNCTION audit.verify_audit_chain_step(
    p_after bigint, p_after_digest text, p_upto bigint,
    p_advance boolean, p_last boolean, p_full boolean)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    damage audit.audit_chain_finding[];
    link   text;
BEGIN
    SELECT coalesce(array_agg(f ORDER BY f.chain_seq NULLS FIRST), '{}')
      INTO damage
      FROM audit.verify_audit_chain_window(p_after, decode(p_after_digest, 'hex'), p_upto, p_last) f;

    link := audit.audit_chain_link_at(p_upto);
    IF cardinality(damage) = 0 AND p_advance AND link IS NOT NULL THEN
        INSERT INTO audit.audit_chain_checkpoint AS c
               (through_seq, through_hash, verified_at, full_verified_at)
        VALUES (p_upto, decode(link, 'hex'), now(), CASE WHEN p_full AND p_last THEN now() END)
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
COMMENT ON FUNCTION verify_audit_chain_step(bigint, text, bigint, boolean, boolean, boolean) IS
    'One bounded step of a verification walk of the system chain: verifies (p_after, p_upto] from p_after_digest, the whole-table and head checks on the last step, and moves the cursor to p_upto when the step is intact and p_advance.';

-- The verification of the links of audit_event_link in (p_after_seq, p_upto],
-- each with the record it names (a missing record reads as modified),
-- linked from p_after_hash; with p_check_head, the whole-table checks of
-- the render policies and search tokens and the head check as well.
CREATE FUNCTION audit.verify_audit_chain_window(
    p_after_seq bigint, p_after_hash bytea, p_upto bigint, p_check_head boolean)
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

    IF p_check_head THEN
        RETURN QUERY
        SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
               ('the render policy ' || encode(p.digest, 'hex')
                || ' was modified after it was written')::text
          FROM audit.audit_render_policy p
         WHERE p.digest IS DISTINCT FROM sha256(convert_to(p.policy::text, 'UTF8'))
         ORDER BY p.digest;

        RETURN QUERY
        SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
               ('the search token ' || t.id || ' was modified after it was written')::text
          FROM audit.audit_search_token t
         WHERE t.digest IS DISTINCT FROM audit.search_token_digest(t.parameter, t.system, t.code)
         ORDER BY t.id;
        RETURN QUERY
        SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
               ('the search set ' || s.id || ' was modified after it was written')::text
          FROM audit.audit_search_set s
         WHERE s.digest IS DISTINCT FROM audit.search_set_digest(s.tokens)
         ORDER BY s.id;
    END IF;

    RETURN QUERY
    WITH linked AS (
        SELECT l.chain_seq, l.event_id AS id, e.recorded_at, l.prev_hash, l.row_hash,
               coalesce(lag(l.row_hash) OVER (ORDER BY l.chain_seq), p_after_hash)
                   AS before_hash,
               coalesce(lag(l.chain_seq) OVER (ORDER BY l.chain_seq), p_after_seq)
                   AS before_seq,
               audit.audit_event_row_digest(e, l.chain_seq, l.prev_hash) AS recomputed,
               e.id IS NULL
                   OR EXISTS (SELECT 1 FROM audit.audit_render_policy p
                               WHERE p.digest = e.render_policy) AS policy_present
          FROM audit.audit_event_link l
          LEFT JOIN audit.audit_event e ON e.id = l.event_id
         WHERE l.chain_seq > p_after_seq
           AND (p_upto IS NULL OR l.chain_seq <= p_upto)
    ),
    judged AS (
        SELECT l.*,
               l.row_hash IS DISTINCT FROM l.recomputed AS modified,
               l.chain_seq = l.before_seq + 1 AS adjacent,
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

    IF NOT p_check_head THEN
        RETURN;
    END IF;

    SELECT l.chain_seq, l.row_hash INTO highest, highest_row
      FROM audit.audit_event_link l ORDER BY l.chain_seq DESC LIMIT 1;
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
COMMENT ON FUNCTION verify_audit_chain_window(bigint, bytea, bigint, boolean) IS
    'The verification of the system chain (audit_event) from one link on, up to and including position p_upto (to the end when NULL), with the whole-table and head checks when p_check_head: the body of the full verification and of each verification step.';

CREATE FUNCTION audit.verify_audit_chain_after(p_after_seq bigint, p_after_hash bytea)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT * FROM audit.verify_audit_chain_window(p_after_seq, p_after_hash, NULL, true)
$$;
COMMENT ON FUNCTION verify_audit_chain_after(bigint, bytea) IS
    'The verification of the system chain (audit_event) from one verified link to the end, with the whole-table and head checks: the body of audit.verify_audit_chain().';

CREATE FUNCTION audit.verify_audit_chain()
    RETURNS TABLE (chain_seq bigint, record_id uuid, recorded_at timestamptz, finding text)
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT f.chain_seq, f.record_id, f.recorded_at, f.finding
      FROM audit.verify_audit_chain_after(0, audit.audit_chain_genesis()) f
$$;
COMMENT ON FUNCTION verify_audit_chain() IS
    'Tamper check over the whole repository: returns one row per damaged record, boundary or render policy, and nothing at all when the trail is intact.';
