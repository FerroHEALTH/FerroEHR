-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the verification of both hash chains walks the chain in bounded
-- steps, each its own statement that moves the cursor (#5779).
--
-- audit.verify_access_log_chain_incremental() and
-- audit.verify_audit_chain_incremental() verified every record after the
-- cursor in one statement and moved the cursor only when that statement
-- finished. A backlog that took longer than the statement timeout was
-- cancelled every run, left the cursor where it was, and grew. A run now
-- takes the chain head when it starts and walks to it one window at a time:
-- the *_verification_step_end() functions name where a window of at most a
-- given number of records ends, and the verify_*_chain_step() functions
-- verify that window from the link before it and, when the window and every
-- window before it in the run are intact, moves the cursor to its end. A run
-- that finds damage keeps walking from the stored link so it reports every
-- finding up to the head, and moves the cursor no more.
--
-- No openEHR spec governs audit tamper detection: our own design.
--
-- Runs with search_path = audit, ext, public.

-- ── the window ───────────────────────────────────────────────────────────────

-- The verification of the elements whose last position is in
-- (p_after_seq, p_upto], linked from p_after_digest; with p_check_head, the
-- head check of 0039 as well. A window ends at a record or at the head, so it
-- never splits a gap.
CREATE FUNCTION audit.verify_access_log_chain_window(
    p_after_seq bigint, p_after_digest text, p_upto bigint, p_check_head boolean)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql STABLE
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    head_position bigint;
    head_link     text;
    highest       bigint;
    highest_link  text;
BEGIN
    SELECT s.head_seq, s.head_digest INTO head_position, head_link
      FROM audit.access_log_chain_state s WHERE s.singleton;
    IF NOT FOUND THEN
        RETURN QUERY SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
            'the access-log chain state row is missing: the log cannot be verified'::text;
        RETURN;
    END IF;

    RETURN QUERY
    WITH elements AS (
        SELECT r.chain_seq AS first_seq, r.chain_seq AS last_seq,
               r.uid AS id,
               r.time_recorded, encode(r.previous_digest, 'hex') AS previous_digest,
               encode(r.digest, 'hex') AS digest, false AS gap,
               encode(r.digest, 'hex') IS DISTINCT FROM audit.access_log_link_digest(
                   encode(r.previous_digest, 'hex'), r.chain_seq, encode(r.content_digest, 'hex'))
                 OR audit.access_log_record_derive(r) IS DISTINCT FROM r AS modified
          FROM audit.access_log_record r
         WHERE r.chain_seq > p_after_seq
           AND (p_upto IS NULL OR r.chain_seq <= p_upto)
        UNION ALL
        SELECT g.first_seq, g.last_seq, NULL::uuid, NULL::timestamptz, NULL::text, g.link,
               true, false
          FROM audit.access_log_chain_gap g
         WHERE g.last_seq > p_after_seq
           AND (p_upto IS NULL OR g.last_seq <= p_upto)
    ),
    linked AS (
        SELECT e.*,
               coalesce(lag(e.digest) OVER w, p_after_digest) AS before_digest,
               coalesce(lag(e.last_seq) OVER w, p_after_seq) AS before_seq,
               lag(e.last_seq) OVER w IS NULL AS opening
          FROM elements e
        WINDOW w AS (ORDER BY e.last_seq, e.first_seq)
    ),
    judged AS (
        SELECT l.*,
               l.first_seq = l.before_seq + 1
                 OR (l.gap AND l.opening AND l.first_seq <= p_after_seq + 1) AS adjacent
          FROM linked l
    )
    SELECT j.first_seq, j.id, j.time_recorded,
           CASE
             WHEN j.modified
               THEN 'record content was modified after it was written'
             WHEN NOT j.adjacent
               THEN 'record(s) deleted between chain position ' || j.before_seq
                    || ' and this one, with no gap for the removal'
             ELSE 'the link to the preceding record does not match'
           END::text
      FROM judged j
     WHERE j.modified
        OR NOT j.adjacent
        OR (NOT j.gap AND j.previous_digest IS DISTINCT FROM j.before_digest);

    IF NOT p_check_head THEN
        RETURN;
    END IF;

    SELECT n.last_seq, n.link INTO highest, highest_link
      FROM (
          (SELECT r.chain_seq AS last_seq, encode(r.digest, 'hex') AS link
             FROM audit.access_log_record r ORDER BY r.chain_seq DESC LIMIT 1)
          UNION ALL
          (SELECT g.last_seq, g.link
             FROM audit.access_log_chain_gap g ORDER BY g.last_seq DESC LIMIT 1)
      ) AS n
     ORDER BY n.last_seq DESC LIMIT 1;
    highest := coalesce(highest, 0);

    IF highest <> head_position THEN
        RETURN QUERY SELECT highest, NULL::uuid, NULL::timestamptz,
            'record(s) deleted from the end of the access log'::text;
    ELSIF highest > 0 AND highest_link IS DISTINCT FROM head_link THEN
        RETURN QUERY SELECT highest, NULL::uuid, NULL::timestamptz,
            'the newest record does not match the recorded head of the access log'::text;
    END IF;
END;
$$;

COMMENT ON FUNCTION audit.verify_access_log_chain_window(bigint, text, bigint, boolean) IS
    'The verification of the access log from one link on, up to and including position p_upto (to the end when NULL), with the head check when p_check_head: the body of the full verification and of each verification step.';

CREATE OR REPLACE FUNCTION audit.verify_access_log_chain_after(p_after_seq bigint, p_after_digest text)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT * FROM audit.verify_access_log_chain_window(p_after_seq, p_after_digest, NULL, true)
$$;

COMMENT ON FUNCTION audit.verify_access_log_chain_after(bigint, text) IS
    'The verification of the access log from one verified link to the end, with the head check: the body of audit.verify_access_log_chain().';

-- ── the walk ─────────────────────────────────────────────────────────────────

-- Where a walk starts: the chain head it walks to, and the link it starts
-- from, the genesis for a full walk and the cursor otherwise. The head is
-- NULL when the chain state row is missing.
CREATE FUNCTION audit.access_log_verification_start(p_full boolean)
    RETURNS TABLE (head_seq bigint, after_seq bigint, after_digest text)
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT (SELECT s.head_seq FROM audit.access_log_chain_state s WHERE s.singleton),
           coalesce(c.through_seq, 0),
           coalesce(c.through_digest, audit.access_log_genesis())
      FROM (SELECT 1) AS one
      LEFT JOIN audit.access_log_chain_verified c ON c.singleton AND NOT p_full
$$;

COMMENT ON FUNCTION audit.access_log_verification_start(boolean) IS
    'Where a verification walk of the access log starts: the chain head it walks to, and the verified link it starts from (the genesis when p_full).';

-- The end of the window after p_after: the position of the p_records-th
-- record after it, or p_head when fewer records are left before it. A record
-- position is never inside a gap, so the window holds whole gaps.
CREATE FUNCTION audit.access_log_verification_step_end(
    p_after bigint, p_records integer, p_head bigint)
    RETURNS bigint
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT greatest(p_after, coalesce(
        (SELECT r.chain_seq FROM audit.access_log_record r
          WHERE r.chain_seq > p_after AND r.chain_seq < p_head
          ORDER BY r.chain_seq OFFSET greatest(p_records, 1) - 1 LIMIT 1),
        p_head))
$$;

COMMENT ON FUNCTION audit.access_log_verification_step_end(bigint, integer, bigint) IS
    'Where the verification window after p_after ends: its p_records-th record, or p_head when fewer are left.';

-- One step of a walk: the window (p_after, p_upto] linked from p_after_digest,
-- with the entries the server found modified (p_modified) and, on the last
-- step (p_last), the head check. When the step finds nothing and p_advance,
-- the cursor moves to p_upto, never backwards; a full walk that reaches its
-- last step intact also records when the whole chain was verified.
CREATE FUNCTION audit.verify_access_log_chain_step(
    p_after bigint, p_after_digest text, p_upto bigint, p_modified bigint[],
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
      FROM (
          SELECT * FROM audit.verify_access_log_chain_window(p_after, p_after_digest, p_upto, p_last)
          UNION ALL
          SELECT r.chain_seq, r.uid, r.time_recorded,
                 'record content was modified after it was written'::text
            FROM audit.access_log_record r
           WHERE r.chain_seq = ANY (coalesce(p_modified, '{}'))
             AND r.chain_seq > p_after AND r.chain_seq <= p_upto
      ) AS f;

    link := audit.access_log_link_at(p_upto);
    IF cardinality(damage) = 0 AND p_advance AND link IS NOT NULL THEN
        INSERT INTO audit.access_log_chain_verified AS c
               (through_seq, through_digest, verified_at, full_verified_at)
        VALUES (p_upto, link, now(), CASE WHEN p_full AND p_last THEN now() END)
        ON CONFLICT (singleton) DO UPDATE
           SET through_seq = excluded.through_seq,
               through_digest = excluded.through_digest,
               verified_at = excluded.verified_at,
               full_verified_at = coalesce(excluded.full_verified_at, c.full_verified_at)
         WHERE excluded.through_seq >= c.through_seq;
    END IF;

    RETURN QUERY SELECT * FROM unnest(damage);
END;
$$;

COMMENT ON FUNCTION audit.verify_access_log_chain_step(bigint, text, bigint, bigint[], boolean, boolean, boolean) IS
    'One bounded step of a verification walk of the access log: verifies (p_after, p_upto] from p_after_digest, the head on the last step, and moves the cursor to p_upto when the step is intact and p_advance.';

-- The link the next step starts from: the stored link at the end of the
-- window just verified.
CREATE FUNCTION audit.access_log_verification_link(p_sequence bigint)
    RETURNS text
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT CASE WHEN p_sequence = 0 THEN audit.access_log_genesis()
                ELSE audit.access_log_link_at(p_sequence) END
$$;

COMMENT ON FUNCTION audit.access_log_verification_link(bigint) IS
    'The stored link at p_sequence, the genesis at 0: where the next step of a verification walk starts.';

DROP FUNCTION audit.verify_access_log_chain_incremental(boolean, bigint, bigint[]);

COMMENT ON TABLE access_log_chain_verified IS
    'The newest access-log link a verification found intact, with every link before it: where an incremental verification of the access log starts. One row, written only by audit.verify_access_log_chain_step().';

-- ── the system chain ─────────────────────────────────────────────────────────

-- The verification of the records of audit_event in (p_after_seq, p_upto],
-- linked from p_after_hash; with p_check_head, the whole-table checks of
-- the render policies and search tokens and the head check of 0031 as well.
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
           AND (p_upto IS NULL OR e.chain_seq <= p_upto)
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

COMMENT ON FUNCTION audit.verify_audit_chain_window(bigint, bytea, bigint, boolean) IS
    'The verification of the system chain (audit_event) from one link on, up to and including position p_upto (to the end when NULL), with the whole-table and head checks when p_check_head: the body of the full verification and of each verification step.';

CREATE OR REPLACE FUNCTION audit.verify_audit_chain_after(p_after_seq bigint, p_after_hash bytea)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT * FROM audit.verify_audit_chain_window(p_after_seq, p_after_hash, NULL, true)
$$;

COMMENT ON FUNCTION audit.verify_audit_chain_after(bigint, bytea) IS
    'The verification of the system chain (audit_event) from one verified link to the end, with the whole-table and head checks: the body of audit.verify_audit_chain().';

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

COMMENT ON FUNCTION audit.audit_chain_verification_start(boolean) IS
    'Where a verification walk of the system chain starts: the chain head it walks to, and the verified link it starts from (the genesis when p_full), in hexadecimal.';

CREATE FUNCTION audit.audit_chain_verification_step_end(
    p_after bigint, p_records integer, p_head bigint)
    RETURNS bigint
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT greatest(p_after, coalesce(
        (SELECT e.chain_seq FROM audit.audit_event e
          WHERE e.chain_seq > p_after AND e.chain_seq < p_head
          ORDER BY e.chain_seq OFFSET greatest(p_records, 1) - 1 LIMIT 1),
        p_head))
$$;

COMMENT ON FUNCTION audit.audit_chain_verification_step_end(bigint, integer, bigint) IS
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

COMMENT ON FUNCTION audit.verify_audit_chain_step(bigint, text, bigint, boolean, boolean, boolean) IS
    'One bounded step of a verification walk of the system chain: verifies (p_after, p_upto] from p_after_digest, the whole-table and head checks on the last step, and moves the cursor to p_upto when the step is intact and p_advance.';

CREATE FUNCTION audit.audit_chain_verification_link(p_sequence bigint)
    RETURNS text
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT CASE WHEN p_sequence = 0 THEN encode(audit.audit_chain_genesis(), 'hex')
                ELSE audit.audit_chain_link_at(p_sequence) END
$$;

COMMENT ON FUNCTION audit.audit_chain_verification_link(bigint) IS
    'The link of the system chain at p_sequence, the genesis at 0: where the next step of a verification walk starts.';

DROP FUNCTION audit.verify_audit_chain_incremental(boolean);

COMMENT ON TABLE audit_chain_checkpoint IS
    'The newest chain link a verification found intact, with every link before it: where an incremental verification starts. One row, written only by audit.verify_audit_chain_step().';

-- ── the lag ──────────────────────────────────────────────────────────────────

-- How far a chain's verification cursor is behind its head: the positions
-- past it and when the oldest record past it was recorded.
CREATE FUNCTION audit.chain_verification_lag(p_chain text)
    RETURNS TABLE (entries bigint, oldest timestamptz)
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT greatest(coalesce(s.head_seq, 0) - coalesce(c.through_seq, 0), 0),
           (SELECT r.time_recorded FROM audit.access_log_record r
             WHERE r.chain_seq > coalesce(c.through_seq, 0)
             ORDER BY r.chain_seq LIMIT 1)
      FROM (SELECT 1) AS one
      LEFT JOIN audit.access_log_chain_state s ON s.singleton
      LEFT JOIN audit.access_log_chain_verified c ON c.singleton
     WHERE p_chain = 'access_log'
    UNION ALL
    SELECT greatest(coalesce(s.head_seq, 0) - coalesce(c.through_seq, 0), 0),
           (SELECT e.recorded_at FROM audit.audit_event e
             WHERE e.chain_seq > coalesce(c.through_seq, 0)
             ORDER BY e.chain_seq LIMIT 1)
      FROM (SELECT 1) AS one
      LEFT JOIN audit.audit_chain_state s ON s.singleton
      LEFT JOIN audit.audit_chain_checkpoint c ON c.singleton
     WHERE p_chain = 'system'
$$;

COMMENT ON FUNCTION audit.chain_verification_lag(text) IS
    'How far the verification cursor of the chain p_chain (system or access_log) is behind its head: the positions past it, and when the oldest record past it was recorded.';

-- ── grants ───────────────────────────────────────────────────────────────────

REVOKE ALL ON FUNCTION audit.verify_access_log_chain_window(bigint, text, bigint, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_verification_start(boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_verification_step_end(bigint, integer, bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.verify_access_log_chain_step(bigint, text, bigint, bigint[], boolean, boolean, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_verification_link(bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.verify_audit_chain_window(bigint, bytea, bigint, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.audit_chain_verification_start(boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.audit_chain_verification_step_end(bigint, integer, bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.verify_audit_chain_step(bigint, text, bigint, boolean, boolean, boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.audit_chain_verification_link(bigint) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.chain_verification_lag(text) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT EXECUTE ON FUNCTION audit.verify_access_log_chain_window(bigint, text, bigint, boolean),
            audit.verify_audit_chain_window(bigint, bytea, bigint, boolean)
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.access_log_verification_start(boolean),
            audit.access_log_verification_step_end(bigint, integer, bigint),
            audit.verify_access_log_chain_step(bigint, text, bigint, bigint[], boolean, boolean, boolean),
            audit.access_log_verification_link(bigint),
            audit.audit_chain_verification_start(boolean),
            audit.audit_chain_verification_step_end(bigint, integer, bigint),
            audit.verify_audit_chain_step(bigint, text, bigint, boolean, boolean, boolean),
            audit.audit_chain_verification_link(bigint),
            audit.chain_verification_lag(text)
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the access-log verification step grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
