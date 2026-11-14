-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the access log keeps a chain of its own, apart from the records
-- outside it (#5573).
--
-- Until now audit_event and access_log_record shared one cursor and one head
-- (0027), so an EHR's span of the access log held the positions of the
-- records outside the log written between its entries (an authentication, a
-- template, party or application operation) and its export was refused. The
-- access log holds only the accesses to EHR data, so it now forms one chain
-- per system with no gaps (docs/plans/access-log.md §Integrity, "One chain
-- per system"), and audit_event, the system chain, keeps its own.
--
-- The split, done once here:
--
-- - The system chain lets go of the access-log records in place. Every run
--   of positions an access-log record held becomes an audit_chain_gap whose
--   link is the digest of the run's last record, which is what the next
--   audit_event record links to, so no audit_event record is rewritten and
--   the system chain verifies as before. Adjacent gaps merge as reaping
--   merges them, and the verification cursor restarts from the genesis.
-- - The access log is renumbered from 1 in its order and linked again from
--   its own genesis. A link signature and a checkpoint signed a digest of
--   the shared chain, so the local link signatures are cleared and the local
--   checkpoints removed; the next scheduled seal signs the new head, and the
--   links written from now on are signed as they are written. The logs
--   imported from other systems keep their own chains untouched.
-- - The FHIR Feed's outbox names a record by its chain and position, under
--   an id of its own the feed delivers by.
--
-- Each chain has its head, its lock, its verification and its signing: the
-- access log its link signatures and ACCESS_LOG_CHECKPOINTs, the system
-- chain its seals (audit_chain_seal). Writers of one chain serialise on its
-- head row (SELECT ... FOR UPDATE), which holds across replicas.
--
-- No openEHR spec governs audit storage, chaining or signing: our own
-- design/extension.
--
-- Runs with search_path = audit, ext, public.

-- ── the FHIR Feed's outbox names the chain ───────────────────────────────────

CREATE TABLE fhir_feed_owed (
    id        bigint NOT NULL GENERATED ALWAYS AS IDENTITY,
    chain     text   NOT NULL,
    chain_seq bigint NOT NULL,
    CONSTRAINT pk_fhir_feed_owed PRIMARY KEY (id),
    CONSTRAINT uq_fhir_feed_owed_record UNIQUE (chain, chain_seq),
    CONSTRAINT ck_fhir_feed_owed_chain CHECK (chain IN ('system', 'access_log'))
);

INSERT INTO fhir_feed_owed (chain, chain_seq)
SELECT CASE WHEN EXISTS (SELECT 1 FROM access_log_record r WHERE r.chain_seq = o.chain_seq)
            THEN 'access_log' ELSE 'system' END,
       o.chain_seq
  FROM fhir_feed_outbox o
 ORDER BY o.chain_seq;

DROP TABLE fhir_feed_outbox;
ALTER TABLE fhir_feed_owed RENAME TO fhir_feed_outbox;
ALTER TABLE fhir_feed_outbox RENAME CONSTRAINT pk_fhir_feed_owed TO pk_fhir_feed_outbox;
ALTER TABLE fhir_feed_outbox RENAME CONSTRAINT uq_fhir_feed_owed_record TO uq_fhir_feed_outbox_record;
ALTER TABLE fhir_feed_outbox RENAME CONSTRAINT ck_fhir_feed_owed_chain TO ck_fhir_feed_outbox_chain;

COMMENT ON TABLE fhir_feed_outbox IS
    'The records the FHIR Feed still owes, each by its chain and position, in the order they were written: written with the record while the feed is configured, removed when the repository accepted it. The feed delivers by id.';
COMMENT ON COLUMN fhir_feed_outbox.chain IS
    'The chain the record is in: system (audit_event) or access_log (access_log_record).';

CREATE OR REPLACE FUNCTION audit.enqueue_fhir_feed_backlog() RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    through bigint;
    moved   bigint;
BEGIN
    SELECT b.through_seq INTO through
      FROM audit.fhir_feed_backlog b
     WHERE b.singleton AND b.enqueued_at IS NULL
       FOR UPDATE;
    IF NOT FOUND THEN
        RETURN 0;
    END IF;
    INSERT INTO audit.fhir_feed_outbox (chain, chain_seq)
    SELECT 'system', e.chain_seq FROM audit.audit_event e
     WHERE e.chain_seq <= through AND e.delivered_fhir_feed_at IS NULL
     ORDER BY e.chain_seq
    ON CONFLICT DO NOTHING;
    GET DIAGNOSTICS moved = ROW_COUNT;
    UPDATE audit.fhir_feed_backlog SET enqueued_at = now() WHERE singleton;
    RETURN moved;
END;
$$;

CREATE OR REPLACE FUNCTION audit.audit_event_leave_outbox() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    DELETE FROM audit.fhir_feed_outbox o
     WHERE o.chain = 'system' AND o.chain_seq = OLD.chain_seq;
    RETURN OLD;
END;
$$;

CREATE OR REPLACE FUNCTION audit.access_log_record_unindex() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF NEW.record_type = 'ACCESS_LOG_TOMBSTONE' THEN
        DELETE FROM audit.access_log_search s WHERE s.chain_seq = NEW.chain_seq;
        DELETE FROM audit.fhir_feed_outbox o
         WHERE o.chain = 'access_log' AND o.chain_seq = NEW.chain_seq;
    END IF;
    RETURN NEW;
END;
$$;

-- ── the system chain lets go of the access-log records ───────────────────────

INSERT INTO audit_chain_gap (from_seq, to_seq, link_hash)
SELECT min(run.chain_seq), max(run.chain_seq),
       decode((array_agg(run.digest ORDER BY run.chain_seq DESC))[1], 'hex')
  FROM (SELECT r.chain_seq, r.digest,
               r.chain_seq - row_number() OVER (ORDER BY r.chain_seq) AS island
          FROM access_log_record r) AS run
 GROUP BY run.island;

DO $merge$
DECLARE
    left_from  bigint;
    right_from bigint;
BEGIN
    LOOP
        SELECT g1.from_seq, g2.from_seq INTO left_from, right_from
          FROM audit.audit_chain_gap g1
          JOIN audit.audit_chain_gap g2 ON g2.from_seq = g1.to_seq + 1
         LIMIT 1;
        EXIT WHEN left_from IS NULL;
        UPDATE audit.audit_chain_gap merged
           SET to_seq = absorbed.to_seq, link_hash = absorbed.link_hash
          FROM audit.audit_chain_gap absorbed
         WHERE merged.from_seq = left_from AND absorbed.from_seq = right_from;
        DELETE FROM audit.audit_chain_gap WHERE from_seq = right_from;
    END LOOP;
END
$merge$;

DELETE FROM audit_chain_checkpoint;

DROP TRIGGER access_log_record_chain_begin ON access_log_record;
DROP TRIGGER access_log_record_chain_commit ON access_log_record;
DROP FUNCTION audit.audit_chain_reserve();
DROP FUNCTION audit.audit_chain_link_at(bigint);

-- The verification of the system chain over audit_event alone, as before the
-- access log joined it (0017).
CREATE OR REPLACE FUNCTION audit.verify_audit_chain_after(p_after_seq bigint, p_after_hash bytea)
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
    'The verification of the system chain (audit_event) from one verified link on: the body of both the full and the incremental verification.';

CREATE OR REPLACE FUNCTION audit.verify_audit_chain_incremental(p_full boolean DEFAULT false)
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

DROP FUNCTION audit.audit_chain_newest();

-- ── the system chain's seals ─────────────────────────────────────────────────

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

COMMENT ON TABLE audit_chain_seal IS
    'The signed seals of the system chain (audit_event), one per sealed position, append-only.';
COMMENT ON COLUMN audit_chain_seal.content IS
    'The seal''s JSON form, proof included.';

CREATE TRIGGER audit_chain_seal_append_only
    BEFORE UPDATE OR DELETE ON audit_chain_seal
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
CREATE TRIGGER audit_chain_seal_no_truncate
    BEFORE TRUNCATE ON audit_chain_seal
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_imported_append_only();

-- The link digest of the system chain at a position, in lower-case
-- hexadecimal: the record's row_hash, or the link of the retention gap that
-- ends there; NULL when neither is there.
CREATE FUNCTION audit.audit_chain_link_at(p_sequence bigint) RETURNS text
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT coalesce(
        (SELECT encode(e.row_hash, 'hex') FROM audit.audit_event e WHERE e.chain_seq = p_sequence),
        (SELECT encode(g.link_hash, 'hex') FROM audit.audit_chain_gap g WHERE g.to_seq = p_sequence))
$$;

COMMENT ON FUNCTION audit.audit_chain_link_at(bigint) IS
    'The link digest of the system chain at a position: what a seal of that position fixes.';

-- ── the access log's own chain ───────────────────────────────────────────────

CREATE FUNCTION audit.access_log_genesis() RETURNS text
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT encode(sha256(convert_to('ferroehr.access_log.v1', 'UTF8')), 'hex') $$;

COMMENT ON FUNCTION audit.access_log_genesis() IS
    'ACCESS_LOG_LINK.previous_digest of the first record of this system''s access log.';

CREATE TABLE access_log_chain_state (
    singleton   boolean     NOT NULL DEFAULT true,
    head_seq    bigint      NOT NULL DEFAULT 0,
    head_digest text        NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_log_chain_state PRIMARY KEY (singleton),
    CONSTRAINT ck_access_log_chain_state_singleton CHECK (singleton),
    CONSTRAINT ck_access_log_chain_state_digest CHECK (head_digest ~ '^[0-9a-f]{64}$')
);

COMMENT ON TABLE access_log_chain_state IS
    'The head of this system''s access log: one row, written only by the SECURITY DEFINER chain triggers; its row lock serialises the writers of the log.';

CREATE TABLE access_log_chain_verified (
    singleton        boolean     NOT NULL DEFAULT true,
    through_seq      bigint      NOT NULL,
    through_digest   text        NOT NULL,
    verified_at      timestamptz NOT NULL,
    full_verified_at timestamptz,
    CONSTRAINT pk_access_log_chain_verified PRIMARY KEY (singleton),
    CONSTRAINT ck_access_log_chain_verified_singleton CHECK (singleton)
);

COMMENT ON TABLE access_log_chain_verified IS
    'The newest access-log link a verification found intact, with every link before it: where an incremental verification of the access log starts. One row, written only by audit.verify_access_log_chain_incremental().';

-- The renumbering: every record keeps its order, takes the next position
-- from 1 and is linked again from the genesis.
ALTER TABLE access_log_search DROP CONSTRAINT fk_access_log_search_record;
ALTER TABLE access_log_record DISABLE TRIGGER access_log_record_tombstone_only;
ALTER TABLE access_log_record DISABLE TRIGGER access_log_record_unindex;
ALTER TABLE access_log_checkpoint DISABLE TRIGGER access_log_checkpoint_append_only;

CREATE TEMPORARY TABLE access_log_renumbered AS
SELECT r.chain_seq AS old_seq, row_number() OVER (ORDER BY r.chain_seq) AS new_seq
  FROM access_log_record r;

UPDATE access_log_record r SET chain_seq = -m.new_seq
  FROM access_log_renumbered m WHERE r.chain_seq = m.old_seq;
UPDATE access_log_record SET chain_seq = -chain_seq;
UPDATE access_log_search s SET chain_seq = -m.new_seq
  FROM access_log_renumbered m WHERE s.chain_seq = m.old_seq;
UPDATE access_log_search SET chain_seq = -chain_seq;
UPDATE fhir_feed_outbox o SET chain_seq = -m.new_seq
  FROM access_log_renumbered m WHERE o.chain = 'access_log' AND o.chain_seq = m.old_seq;
UPDATE fhir_feed_outbox SET chain_seq = -chain_seq WHERE chain_seq < 0;
DROP TABLE access_log_renumbered;

DO $relink$
DECLARE
    previous text := audit.access_log_genesis();
    last_seq bigint := 0;
    held     record;
BEGIN
    FOR held IN SELECT r.chain_seq, r.content_digest FROM audit.access_log_record r
                 ORDER BY r.chain_seq
    LOOP
        UPDATE audit.access_log_record
           SET previous_digest = previous,
               digest = audit.access_log_link_digest(previous, held.chain_seq, held.content_digest),
               signature = NULL
         WHERE chain_seq = held.chain_seq
        RETURNING digest INTO previous;
        last_seq := held.chain_seq;
    END LOOP;
    INSERT INTO audit.access_log_chain_state (singleton, head_seq, head_digest)
    VALUES (true, last_seq, previous);
END
$relink$;

DELETE FROM access_log_checkpoint;

ALTER TABLE access_log_checkpoint ENABLE TRIGGER access_log_checkpoint_append_only;
ALTER TABLE access_log_record ENABLE TRIGGER access_log_record_unindex;
ALTER TABLE access_log_record ENABLE TRIGGER access_log_record_tombstone_only;
ALTER TABLE access_log_search ADD CONSTRAINT fk_access_log_search_record
    FOREIGN KEY (chain_seq) REFERENCES access_log_record (chain_seq);

COMMENT ON TABLE access_log_record IS
    'The openEHR access log (draft org.openehr.rm.access_log 0.1.0): one row per ACCESS_LOG_ENTRY or ACCESS_LOG_TOMBSTONE, in this system''s own chain, apart from the records outside the log. Strictly outside the EHR content (BASE architecture_overview/master07-security.adoc §Access logging); append-only except the replacement of an entry by its tombstone.';
COMMENT ON COLUMN access_log_record.chain_seq IS
    'The position in this system''s access log, ACCESS_LOG_LINK.sequence, set by the insert trigger: from 1, with no gaps.';

-- ── the chain writer of the access log ───────────────────────────────────────

-- As the system chain's writer (0003): the head row is locked and read once
-- per statement, carried per row in a transaction-local setting the statement
-- trigger overwrites, and written once at the statement's end.
CREATE FUNCTION audit.access_log_chain_begin() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    head_position bigint;
    head_link     text;
BEGIN
    SELECT s.head_seq, s.head_digest INTO head_position, head_link
      FROM audit.access_log_chain_state s
     WHERE s.singleton
       FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'the access-log chain state is missing: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    PERFORM set_config('ferroehr.access_log_chain_head', head_position || ':' || head_link, true);
    RETURN NULL;
END;
$$;

CREATE OR REPLACE FUNCTION audit.access_log_record_chain_link() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.access_log_chain_head', true);
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RAISE EXCEPTION 'the access-log chain cursor is unset: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW := audit.access_log_record_derive(NEW);
    IF NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_ENTRY' THEN
        RAISE EXCEPTION 'an access-log record is written as an ACCESS_LOG_ENTRY, not %', NEW.record_type
            USING ERRCODE = 'check_violation';
    END IF;
    NEW.chain_seq := split_part(cursor_value, ':', 1)::bigint + 1;
    NEW.previous_digest := split_part(cursor_value, ':', 2);
    NEW.algorithm := 'SHA-256';
    NEW.content_digest := audit.access_log_content_digest(NEW.content);
    NEW.digest := audit.access_log_link_digest(NEW.previous_digest, NEW.chain_seq, NEW.content_digest);
    NEW.stored_at := now();
    PERFORM set_config('ferroehr.access_log_chain_head', NEW.chain_seq || ':' || NEW.digest, true);
    RETURN NEW;
END;
$$;

CREATE FUNCTION audit.access_log_chain_commit() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.access_log_chain_head', true);
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RETURN NULL;
    END IF;
    UPDATE audit.access_log_chain_state
       SET head_seq = split_part(cursor_value, ':', 1)::bigint,
           head_digest = split_part(cursor_value, ':', 2),
           updated_at = now()
     WHERE singleton;
    RETURN NULL;
END;
$$;

CREATE TRIGGER access_log_record_chain_begin
    BEFORE INSERT ON access_log_record
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_chain_begin();
CREATE TRIGGER access_log_record_chain_commit
    AFTER INSERT ON access_log_record
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_chain_commit();

-- The head of the access log, locked until the transaction ends: the
-- position and link digest the next record follows, so a writer signs the
-- links its next rows will get (0029).
CREATE FUNCTION audit.access_log_chain_reserve()
    RETURNS TABLE (head_seq bigint, head_digest text)
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RETURN QUERY
        SELECT s.head_seq, s.head_digest
          FROM audit.access_log_chain_state s
         WHERE s.singleton
           FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'the access-log chain state is missing: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
END;
$$;

COMMENT ON FUNCTION audit.access_log_chain_reserve() IS
    'Lock the head of the access log until the transaction ends and return its position and link digest, so a writer can sign the links its next rows will get.';

-- ── the verification of the access log ───────────────────────────────────────

-- The access log from the link (p_after_seq, p_after_digest) on: an entry's
-- content digest is recomputed from its content and a tombstone's taken as
-- recorded, as the draft's verifier does; every link is recomputed and
-- matched to the one before it; and the newest record is matched to the
-- head. The log has no gaps, so a missing position is a deleted record.
CREATE FUNCTION audit.verify_access_log_chain_after(p_after_seq bigint, p_after_digest text)
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
    WITH linked AS (
        SELECT r.chain_seq,
               CASE WHEN r.uid ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                    THEN r.uid::uuid END AS id,
               r.time_recorded, r.previous_digest, r.digest,
               coalesce(lag(r.digest) OVER (ORDER BY r.chain_seq), p_after_digest) AS before_digest,
               coalesce(lag(r.chain_seq) OVER (ORDER BY r.chain_seq), p_after_seq) AS before_seq,
               r.digest IS DISTINCT FROM audit.access_log_link_digest(
                   r.previous_digest, r.chain_seq, r.content_digest)
                 OR (r.record_type = 'ACCESS_LOG_ENTRY'
                     AND r.content_digest IS DISTINCT FROM audit.access_log_content_digest(r.content))
                 OR audit.access_log_record_derive(r) IS DISTINCT FROM r AS modified
          FROM audit.access_log_record r
         WHERE r.chain_seq > p_after_seq
    )
    SELECT l.chain_seq, l.id, l.time_recorded,
           CASE
             WHEN l.modified
               THEN 'record content was modified after it was written'
             WHEN l.chain_seq <> l.before_seq + 1
               THEN 'record(s) deleted between chain position ' || l.before_seq
                    || ' and this one'
             ELSE 'the link to the preceding record does not match'
           END::text
      FROM linked l
     WHERE l.modified
        OR l.chain_seq <> l.before_seq + 1
        OR l.previous_digest IS DISTINCT FROM l.before_digest;

    SELECT r.chain_seq, r.digest INTO highest, highest_link
      FROM audit.access_log_record r ORDER BY r.chain_seq DESC LIMIT 1;
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

COMMENT ON FUNCTION audit.verify_access_log_chain_after(bigint, text) IS
    'The verification of the access log from one verified link on: the body of both the full and the incremental verification.';

CREATE FUNCTION audit.verify_access_log_chain()
    RETURNS TABLE (chain_seq bigint, record_id uuid, recorded_at timestamptz, finding text)
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT f.chain_seq, f.record_id, f.recorded_at, f.finding
      FROM audit.verify_access_log_chain_after(0, audit.access_log_genesis()) f
$$;

COMMENT ON FUNCTION audit.verify_access_log_chain() IS
    'Tamper check over the whole access log: one row per damaged record or boundary, and nothing at all when the log is intact.';

CREATE FUNCTION audit.verify_access_log_chain_incremental(p_full boolean DEFAULT false)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    from_seq    bigint := 0;
    from_digest text := audit.access_log_genesis();
    damage      audit.audit_chain_finding[];
    top_seq     bigint;
    top_digest  text;
BEGIN
    IF NOT coalesce(p_full, false) THEN
        SELECT c.through_seq, c.through_digest INTO from_seq, from_digest
          FROM audit.access_log_chain_verified c WHERE c.singleton;
        IF NOT FOUND THEN
            from_seq := 0;
            from_digest := audit.access_log_genesis();
        END IF;
    END IF;

    SELECT (SELECT coalesce(array_agg(f ORDER BY f.chain_seq NULLS FIRST), '{}')
              FROM audit.verify_access_log_chain_after(from_seq, from_digest) f),
           newest.chain_seq, newest.digest
      INTO damage, top_seq, top_digest
      FROM (SELECT 1) AS one
      LEFT JOIN LATERAL (
          SELECT r.chain_seq, r.digest
            FROM audit.access_log_record r ORDER BY r.chain_seq DESC LIMIT 1
      ) AS newest ON true;

    IF cardinality(damage) = 0 AND top_seq IS NOT NULL THEN
        INSERT INTO audit.access_log_chain_verified AS c
               (through_seq, through_digest, verified_at, full_verified_at)
        VALUES (top_seq, top_digest, now(), CASE WHEN p_full THEN now() END)
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

COMMENT ON FUNCTION audit.verify_access_log_chain_incremental(boolean) IS
    'Verifies the access-log records written after the verification cursor (every record when p_full) and the head, and moves the cursor to the newest record when the run finds nothing.';

-- ── the relations the search reads name the chain ────────────────────────────

CREATE OR REPLACE VIEW audit_record AS
    SELECT e.chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.operation,
           e.principal, e.organisation, e.patient_id, e.resource_class, e.resource_id,
           e.client_ip, e.token_id, e.domain, e.purpose, e.legal_basis, e.result_count,
           e.request_id, e.roles, e.origins, e.origin_count, e.category_map_digest,
           e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id,
           e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id,
           e.subject_namespace, e.subject_lookup_failed, e.category_content,
           e.render_policy, e.token_attributes, e.search_tokens,
           NULL::text AS content, NULL::text AS algorithm, NULL::text AS previous_digest,
           NULL::text AS content_digest, NULL::text AS digest, NULL::text AS signature,
           'system'::text AS chain
      FROM audit_event e
    UNION ALL
    SELECT s.chain_seq, s.recorded_at, r.stored_at, s.action, s.outcome, NULL::text,
           s.principal, NULL::text, s.patient_id, NULL::text, s.resource_id,
           s.client_ip, NULL::text, NULL::text, NULL::text, NULL::text, NULL::bigint,
           s.request_id, NULL::jsonb, NULL::jsonb, NULL::bigint, NULL::text,
           NULL::text, NULL::text, NULL::text, NULL::text,
           false, NULL::text, NULL::text, NULL::text, s.ehr_id,
           NULL::text, false, NULL::jsonb,
           s.render_policy, NULL::jsonb, s.search_tokens,
           r.content, r.algorithm, r.previous_digest, r.content_digest, r.digest, r.signature,
           'access_log'::text
      FROM access_log_search s
      JOIN access_log_record r ON r.chain_seq = s.chain_seq;


CREATE OR REPLACE VIEW access_trail AS
    SELECT e.chain_seq, e.id, e.recorded_at, e.stored_at, e.action, e.outcome,
           e.event_code, e.operation, e.principal, e.organisation, e.patient_id,
           e.resource_class, e.resource_id, e.client_ip, e.token_id, e.domain, e.purpose,
           e.legal_basis, e.result_count, e.request_id, e.roles, e.origins, e.origin_count,
           e.categories, e.category_basis, e.category_evidence, e.category_map_digest,
           e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id,
           e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id,
           e.subject_namespace, e.subject_lookup_failed, e.category_content,
           e.binding, e.guest, e.party, e.party_groups, e.provider, e.representation,
           e.external_fields, e.agreement, e.actor_chain, e.facilities, e.binding_detail,
           e.summary, e.token_attributes,
           NULL::text AS entry_uid, 'system'::text AS chain
      FROM audit_event e
    UNION ALL
    SELECT t.chain_seq, NULL::uuid, t.time_recorded, t.stored_at,
           audit.access_detail(t.doc, 'at0001') ->> 'value',
           (audit.access_detail(t.doc, 'at0003') ->> 'magnitude')::smallint,
           audit.access_event_code(audit.access_detail(t.doc, 'at0002') ->> 'value',
                                   audit.access_detail(t.doc, 'at0001') ->> 'value'),
           coalesce(audit.access_detail(t.doc, 'at0029') ->> 'value',
                    t.doc #>> '{operation,defining_code,code_string}'),
           audit.access_detail(t.doc, 'at0004') ->> 'value',
           audit.access_detail(t.doc, 'at0008') ->> 'value',
           audit.access_detail(t.doc, 'at0012') ->> 'id',
           audit.access_detail(t.doc, 'at0002') ->> 'value',
           audit.access_detail(t.doc, 'at0014') ->> 'value',
           t.doc #>> '{channel,client_address}',
           audit.access_detail(t.doc, 'at0007') ->> 'value',
           coalesce(audit.access_detail(t.doc, 'at0010') ->> 'value', 'clinical'),
           audit.access_detail(t.doc, 'at0015') ->> 'value',
           audit.access_detail(t.doc, 'at0016') ->> 'value',
           (audit.access_detail(t.doc, 'at0017') ->> 'magnitude')::bigint,
           t.doc #>> '{channel,request_id}',
           CASE WHEN audit.access_detail(t.doc, 'at0004') IS NULL
                     AND audit.access_details(t.doc, 'at0009') IS NULL THEN NULL
                ELSE coalesce(audit.access_details(t.doc, 'at0009'), '[]'::jsonb) END,
           audit.access_details(t.doc, 'at0018'),
           (audit.access_detail(t.doc, 'at0019') ->> 'magnitude')::bigint,
           NULL::jsonb, NULL::text, NULL::jsonb,
           audit.access_detail(t.doc, 'at0020') ->> 'value',
           audit.access_detail(t.doc, 'at0022') ->> 'value',
           audit.access_detail(t.doc, 'at0023') ->> 'value',
           audit.access_detail(t.doc, 'at0024') ->> 'value',
           audit.access_detail(t.doc, 'at0025') ->> 'value',
           coalesce((t.doc ->> 'is_emergency')::boolean, false),
           (SELECT a #>> '{reference,id}'
              FROM jsonb_array_elements(coalesce(t.doc -> 'authorisations', '[]'::jsonb)) a
             WHERE a #>> '{kind,defining_code,code_string}' = 'permit' LIMIT 1),
           audit.access_detail(t.doc, 'at0005') ->> 'value',
           audit.access_detail(t.doc, 'at0006') ->> 'value',
           audit.access_detail(t.doc, 'at0011') ->> 'value',
           audit.access_detail(t.doc, 'at0012') ->> 'issuer',
           coalesce((audit.access_detail(t.doc, 'at0013') ->> 'value')::boolean, false),
           (audit.access_detail(t.doc, 'at0021') ->> 'value')::jsonb,
           t.binding ->> 'code',
           coalesce((t.binding ->> 'guest')::boolean, false),
           nullif(t.binding -> 'party', 'null'::jsonb),
           nullif(t.binding -> 'groups', '[]'::jsonb),
           nullif(t.binding -> 'provider', 'null'::jsonb),
           nullif(t.binding -> 'representation', 'null'::jsonb),
           nullif(t.binding -> 'external', '{}'::jsonb),
           nullif(t.binding -> 'agreement', 'null'::jsonb),
           nullif(t.binding -> 'actor_chain', '[]'::jsonb),
           nullif(t.binding -> 'facilities', '[]'::jsonb),
           nullif(t.binding -> 'detail', '{}'::jsonb),
           (audit.access_detail(t.doc, 'at0028') ->> 'value')::jsonb,
           (audit.access_detail(t.doc, 'at0026') ->> 'value')::jsonb,
           t.uid, 'access_log'::text
      FROM (SELECT r.chain_seq, r.uid, r.time_recorded, r.stored_at, r.content::jsonb AS doc,
                   (audit.access_detail(r.content::jsonb, 'at0027') ->> 'value')::jsonb AS binding
              FROM access_log_record r
             WHERE r.record_type = 'ACCESS_LOG_ENTRY') t;

COMMENT ON VIEW audit_record IS
    'Every record the ITI-81 search, the person''s own trail and the FHIR Feed read: an audit_event record of the system chain by its columns, an access-log entry by its content and link, which its projection is rendered from. A record is named by its chain and position.';
COMMENT ON VIEW access_trail IS
    'The access trail in SQL: every audit_event record and every access-log entry with the facts it states, in the audit vocabulary, each named by its chain and position. An entry''s categories are computed by its projection and are no column here.';

-- ── grants ───────────────────────────────────────────────────────────────────

REVOKE ALL ON FUNCTION audit.access_log_chain_begin() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_chain_commit() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_chain_reserve() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.verify_access_log_chain_after(bigint, text) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.verify_access_log_chain_incremental(boolean) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.audit_chain_link_at(bigint) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON fhir_feed_outbox, access_log_chain_state, access_log_chain_verified,
            audit_chain_seal FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT, INSERT, DELETE ON fhir_feed_outbox TO ferroehr_clinical;
        GRANT SELECT ON access_log_chain_state, access_log_chain_verified
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON audit_chain_seal TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT INSERT ON audit_chain_seal TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.access_log_chain_reserve() TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.verify_access_log_chain()
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.verify_access_log_chain_after(bigint, text)
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.verify_access_log_chain_incremental(boolean)
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.audit_chain_link_at(bigint)
            TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the access-log chain grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
