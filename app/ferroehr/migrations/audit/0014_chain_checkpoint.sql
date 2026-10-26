-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: incremental chain verification from the last verified link.
--
-- audit.verify_audit_chain() recomputes the digest of every record, so its
-- cost grows with the trail. The chain makes a later link commit to every
-- earlier one, so a run that starts from a link an earlier run verified only
-- has to recompute the records written since. audit_chain_checkpoint keeps
-- that link: its position, its digest and its digest version.
-- audit.verify_audit_chain_incremental() verifies the records after it, and
-- the chain head, and moves the checkpoint to the newest record when it finds
-- nothing. A change to a record before the checkpoint is outside an
-- incremental run, so the full pass stays: audit.verify_audit_chain()
-- unchanged on demand, and audit.verify_audit_chain_incremental(true) as the
-- scheduled full pass.
--
-- No openEHR spec governs audit tamper detection: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

CREATE TABLE audit_chain_checkpoint (
    singleton        boolean     NOT NULL DEFAULT true,
    through_seq      bigint      NOT NULL,
    through_hash     bytea       NOT NULL,
    through_version  smallint    NOT NULL,
    verified_at      timestamptz NOT NULL,
    full_verified_at timestamptz,
    CONSTRAINT pk_audit_chain_checkpoint PRIMARY KEY (singleton),
    CONSTRAINT ck_audit_chain_checkpoint_singleton CHECK (singleton)
);

COMMENT ON TABLE audit_chain_checkpoint IS
    'The newest chain link a verification found intact, with every link before it: where an incremental verification starts. One row, written only by audit.verify_audit_chain_incremental().';

CREATE TYPE audit_chain_finding AS (
    chain_seq   bigint,
    record_id   uuid,
    recorded_at timestamptz,
    finding     text
);

-- The verification body, from the link (p_after_seq, p_after_hash,
-- p_after_version) on: every record after it is recomputed and re-linked, and
-- both ends are checked against the chain state. From (0, genesis, NULL) it is
-- the full pass. STABLE, so every statement reads the snapshot of the calling
-- query (PostgreSQL 18 docs, "Function Volatility Categories",
-- https://www.postgresql.org/docs/18/xfunc-volatility.html) and a record
-- committed during the run cannot look like a deletion at the end.
CREATE FUNCTION audit.verify_audit_chain_after(
        p_after_seq bigint, p_after_hash bytea, p_after_version smallint)
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

    RETURN QUERY
    WITH linked AS (
        SELECT e.chain_seq, e.id, e.recorded_at, e.prev_hash, e.row_hash,
               e.chain_digest_version,
               coalesce(lag(e.row_hash) OVER (ORDER BY e.chain_seq), p_after_hash)
                   AS before_hash,
               coalesce(lag(e.chain_seq) OVER (ORDER BY e.chain_seq), p_after_seq)
                   AS before_seq,
               coalesce(lag(e.chain_digest_version) OVER (ORDER BY e.chain_seq),
                        p_after_version) AS before_version,
               audit.audit_event_row_digest(e) AS recomputed
          FROM audit.audit_event e
         WHERE e.chain_seq > p_after_seq
    ),
    judged AS (
        SELECT l.*,
               l.row_hash IS DISTINCT FROM l.recomputed AS modified,
               coalesce(l.before_version > l.chain_digest_version, false) AS downgraded,
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

COMMENT ON FUNCTION audit.verify_audit_chain_after(bigint, bytea, smallint) IS
    'The chain verification from one verified link on: the body of both the full and the incremental verification.';

-- The full pass, with the same columns and findings as before.
CREATE OR REPLACE FUNCTION audit.verify_audit_chain()
    RETURNS TABLE (chain_seq bigint, record_id uuid, recorded_at timestamptz, finding text)
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT f.chain_seq, f.record_id, f.recorded_at, f.finding
      FROM audit.verify_audit_chain_after(0, audit.audit_chain_genesis(), NULL) f
$$;

-- One statement reads the findings and the newest record, so both come from
-- one snapshot and the checkpoint never passes a record the run did not see.
-- Concurrent runs (one per replica) only ever move the checkpoint forward.
CREATE FUNCTION audit.verify_audit_chain_incremental(p_full boolean DEFAULT false)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    from_seq     bigint := 0;
    from_hash    bytea := audit.audit_chain_genesis();
    from_version smallint;
    damage       audit.audit_chain_finding[];
    top_seq      bigint;
    top_hash     bytea;
    top_version  smallint;
BEGIN
    IF NOT coalesce(p_full, false) THEN
        SELECT c.through_seq, c.through_hash, c.through_version
          INTO from_seq, from_hash, from_version
          FROM audit.audit_chain_checkpoint c WHERE c.singleton;
        IF NOT FOUND THEN
            from_seq := 0;
            from_hash := audit.audit_chain_genesis();
            from_version := NULL;
        END IF;
    END IF;

    SELECT (SELECT coalesce(array_agg(f ORDER BY f.chain_seq NULLS FIRST), '{}')
              FROM audit.verify_audit_chain_after(from_seq, from_hash, from_version) f),
           newest.chain_seq, newest.row_hash, newest.chain_digest_version
      INTO damage, top_seq, top_hash, top_version
      FROM (SELECT 1) AS one
      LEFT JOIN LATERAL (
          SELECT e.chain_seq, e.row_hash, e.chain_digest_version
            FROM audit.audit_event e ORDER BY e.chain_seq DESC LIMIT 1
      ) AS newest ON true;

    IF cardinality(damage) = 0 AND top_seq IS NOT NULL THEN
        INSERT INTO audit.audit_chain_checkpoint AS c
               (through_seq, through_hash, through_version, verified_at, full_verified_at)
        VALUES (top_seq, top_hash, top_version, now(),
                CASE WHEN p_full THEN now() END)
        ON CONFLICT (singleton) DO UPDATE
           SET through_seq = excluded.through_seq,
               through_hash = excluded.through_hash,
               through_version = excluded.through_version,
               verified_at = excluded.verified_at,
               full_verified_at = coalesce(excluded.full_verified_at, c.full_verified_at)
         WHERE excluded.through_seq >= c.through_seq;
    END IF;

    RETURN QUERY SELECT * FROM unnest(damage);
END;
$$;

COMMENT ON FUNCTION audit.verify_audit_chain_incremental(boolean) IS
    'Verifies the records written after the checkpoint (every record when p_full) and the chain head, and moves the checkpoint to the newest record when the run finds nothing; returns the findings as audit.verify_audit_chain() does.';

-- Functions are executable by PUBLIC unless revoked (PostgreSQL 18, GRANT,
-- "Notes", https://www.postgresql.org/docs/18/sql-grant.html).
REVOKE ALL ON FUNCTION audit.verify_audit_chain_incremental(boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.verify_audit_chain_after(bigint, bytea, smallint) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON audit_chain_checkpoint
            FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON audit_chain_checkpoint
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.verify_audit_chain_after(bigint, bytea, smallint)
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.verify_audit_chain_incremental(boolean)
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping audit chain checkpoint grants (roles absent — see the ext role block NOTICE)';
    END IF;
END
$grants$;
