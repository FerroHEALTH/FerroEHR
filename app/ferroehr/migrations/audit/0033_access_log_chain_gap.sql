-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: a run of expired tombstones folds into one gap of the access log's
-- chain (#5619).
--
-- Retention replaces an expired entry by its ACCESS_LOG_TOMBSTONE in place
-- (0027), so the records after it still verify. A tombstone was kept for
-- ever, so the log never stopped growing. A tombstone is now kept one more
-- retention horizon, its entry's own: as long after its removal as the entry
-- was kept before it (fold_after). After that, each run of consecutive
-- tombstones that are all past it is removed through one sanctioned path,
-- audit.fold_access_log_tombstones(), which writes the run as one
-- access_log_chain_gap: the positions it held, the link digest of its last
-- record (what the record after it links to), that link's signature, and the
-- grounds its tombstones named. The gap row is the record of the fold,
-- append-only. A run never spans a checkpoint's position except as its last,
-- so every checkpoint still names a link the log holds.
--
-- The verification walks a gap as one record whose digest is its link, and
-- a missing position no gap accounts for is still a deletion. An erasure a
-- law orders is a tombstone like any other and folds the same way.
--
-- An import keeps the gaps an export carries apart from this system's chain,
-- as it keeps the links (0028).
--
-- No openEHR spec governs audit storage, chaining or retention: our own
-- design/extension.
--
-- Runs with search_path = audit, ext, public.

-- ── how long a tombstone is kept ─────────────────────────────────────────────

ALTER TABLE access_log_record ADD COLUMN fold_after timestamptz;

COMMENT ON COLUMN access_log_record.fold_after IS
    'A tombstone''s end: time_removed plus the time its entry was kept (time_removed minus time_recorded), derived from content; NULL on an entry. After it the tombstone folds into a gap.';

CREATE OR REPLACE FUNCTION audit.access_log_record_derive(p_row audit.access_log_record)
    RETURNS audit.access_log_record
    LANGUAGE plpgsql STABLE
    SET search_path = pg_catalog
    SET TimeZone = 'UTC'
    AS $$
DECLARE
    doc jsonb := p_row.content::jsonb;
BEGIN
    IF jsonb_typeof(doc) IS DISTINCT FROM 'object' OR doc ? 'link' THEN
        RAISE EXCEPTION 'an access-log record is stored as its canonical form without its link'
            USING ERRCODE = 'check_violation';
    END IF;
    p_row.record_type := doc ->> '_type';
    p_row.uid := doc #>> '{uid,value}';
    p_row.system_id := doc ->> 'system_id';
    p_row.time_recorded := (doc #>> '{time_recorded,value}')::timestamptz;
    p_row.ehr_id := doc #>> '{ehr_id,value}';
    p_row.action := doc #>> '{action,value}';
    p_row.result := doc #>> '{outcome,result,value}';
    p_row.party_id := doc #>> '{accessor,party,external_ref,id,value}';
    p_row.request_id := doc #>> '{channel,request_id}';
    p_row.retain_until := (doc #>> '{retention,retain_until,value}')::date;
    p_row.time_removed := (doc #>> '{time_removed,value}')::timestamptz;
    p_row.fold_after := p_row.time_removed + (p_row.time_removed - p_row.time_recorded);
    RETURN p_row;
END;
$$;

ALTER TABLE access_log_record DISABLE TRIGGER access_log_record_tombstone_only;
UPDATE access_log_record
   SET fold_after = time_removed + (time_removed - time_recorded)
 WHERE record_type = 'ACCESS_LOG_TOMBSTONE';
ALTER TABLE access_log_record ENABLE TRIGGER access_log_record_tombstone_only;

CREATE INDEX ix_access_log_record_fold_after ON access_log_record (fold_after)
    WHERE fold_after IS NOT NULL;

-- ── the gaps ─────────────────────────────────────────────────────────────────

CREATE TABLE access_log_chain_gap (
    first_seq   bigint      NOT NULL,
    last_seq    bigint      NOT NULL,
    link        text        NOT NULL,
    signature   text,
    grounds     jsonb       NOT NULL,
    time_folded timestamptz NOT NULL,
    CONSTRAINT pk_access_log_chain_gap PRIMARY KEY (first_seq),
    CONSTRAINT uq_access_log_chain_gap_last UNIQUE (last_seq),
    CONSTRAINT ck_access_log_chain_gap_span CHECK (last_seq >= first_seq AND first_seq >= 1),
    CONSTRAINT ck_access_log_chain_gap_link CHECK (link ~ '^[0-9a-f]{64}$'),
    CONSTRAINT ck_access_log_chain_gap_grounds
        CHECK (jsonb_typeof(grounds) = 'array' AND jsonb_array_length(grounds) > 0)
);

COMMENT ON TABLE access_log_chain_gap IS
    'The runs of expired tombstones removed from this system''s access log, one row per fold, append-only: the positions the run held and the link of its last record, which the record after it links to.';
COMMENT ON COLUMN access_log_chain_gap.link IS
    'ACCESS_LOG_LINK.digest of the run''s last record.';
COMMENT ON COLUMN access_log_chain_gap.signature IS
    'ACCESS_LOG_LINK.signature of the run''s last record, the signature of link; NULL when that link was not signed.';
COMMENT ON COLUMN access_log_chain_gap.grounds IS
    'Every distinct ground the run''s tombstones named, as DV_CODED_TEXT ITS-JSON, ordered.';

CREATE FUNCTION audit.access_log_chain_gap_append_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.access_log_chain_gap is append-only: a fold is never undone or rewritten'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER access_log_chain_gap_append_only
    BEFORE UPDATE OR DELETE ON access_log_chain_gap
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_chain_gap_append_only();
CREATE TRIGGER access_log_chain_gap_no_truncate
    BEFORE TRUNCATE ON access_log_chain_gap
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_chain_gap_append_only();

-- A record leaves the log only through the fold, and only as a tombstone.
CREATE OR REPLACE FUNCTION audit.access_log_record_reject_removal() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND OLD.record_type = 'ACCESS_LOG_TOMBSTONE'
       AND coalesce(current_setting('ferroehr.access_log_folding', true), 'off') = 'on'
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'audit.access_log_record is append-only: a record is replaced by its tombstone, and a tombstone leaves only by the fold into a gap'
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- The link digest of the access log at a position, in lower-case
-- hexadecimal: the record's, or the link of the gap that ends there; NULL
-- when neither is there.
CREATE FUNCTION audit.access_log_link_at(p_sequence bigint) RETURNS text
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT coalesce(
        (SELECT r.digest FROM audit.access_log_record r WHERE r.chain_seq = p_sequence),
        (SELECT g.link FROM audit.access_log_chain_gap g WHERE g.last_seq = p_sequence))
$$;

COMMENT ON FUNCTION audit.access_log_link_at(bigint) IS
    'The link digest of the access log at a position: what a checkpoint of that position seals.';

-- ── the fold ─────────────────────────────────────────────────────────────────

-- Up to p_limit tombstones past fold_after at p_now, oldest end first, fold
-- at p_now into one gap per run of consecutive positions, cut after each checkpoint's
-- position. Replicas serialise on a transaction-scoped advisory lock, so two
-- runs never fold the same tombstone. Returns the tombstones removed and the
-- gaps written.
CREATE FUNCTION audit.fold_access_log_tombstones(p_now timestamptz, p_limit integer)
    RETURNS TABLE (folded bigint, gaps bigint)
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF p_now IS NULL OR coalesce(p_limit, 0) < 1 THEN
        RETURN QUERY SELECT 0::bigint, 0::bigint;
        RETURN;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtext('audit.access_log_chain_gap'), 0);
    PERFORM set_config('ferroehr.access_log_folding', 'on', true);
    RETURN QUERY
    WITH due AS (
        SELECT r.chain_seq, r.digest, r.signature, r.content
          FROM audit.access_log_record r
         WHERE r.fold_after <= p_now
         ORDER BY r.fold_after, r.chain_seq
         LIMIT p_limit
    ),
    islanded AS (
        SELECT d.*,
               d.chain_seq - row_number() OVER (ORDER BY d.chain_seq) AS island,
               EXISTS (SELECT 1 FROM audit.access_log_checkpoint c
                        WHERE c.sequence = d.chain_seq) AS sealed
          FROM due d
    ),
    parted AS (
        SELECT i.*,
               coalesce(sum(i.sealed::int) OVER (
                   PARTITION BY i.island ORDER BY i.chain_seq
                   ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING), 0) AS part
          FROM islanded i
    ),
    grounded AS (
        SELECT p.island, p.part, jsonb_agg(DISTINCT g.ground ORDER BY g.ground) AS grounds
          FROM parted p
          CROSS JOIN LATERAL jsonb_array_elements(p.content::jsonb -> 'grounds') AS g(ground)
         GROUP BY p.island, p.part
    ),
    spans AS (
        SELECT min(p.chain_seq) AS first_seq, max(p.chain_seq) AS last_seq,
               (array_agg(p.digest ORDER BY p.chain_seq DESC))[1] AS link,
               (array_agg(p.signature ORDER BY p.chain_seq DESC))[1] AS signature,
               p.island, p.part
          FROM parted p
         GROUP BY p.island, p.part
    ),
    written AS (
        INSERT INTO audit.access_log_chain_gap
               (first_seq, last_seq, link, signature, grounds, time_folded)
        SELECT s.first_seq, s.last_seq, s.link, s.signature, g.grounds, p_now
          FROM spans s
          JOIN grounded g ON g.island = s.island AND g.part = s.part
         ORDER BY s.first_seq
        RETURNING 1
    ),
    removed AS (
        DELETE FROM audit.access_log_record r
         USING due d
         WHERE r.chain_seq = d.chain_seq
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM removed), (SELECT count(*) FROM written);
    PERFORM set_config('ferroehr.access_log_folding', 'off', true);
END;
$$;

COMMENT ON FUNCTION audit.fold_access_log_tombstones(timestamptz, integer) IS
    'The one path a tombstone leaves the access log by: up to p_limit tombstones past their end fold into one access_log_chain_gap per run, cut after each checkpoint''s position.';

-- ── the verification walks the gaps ──────────────────────────────────────────

-- The access log from the link (p_after_seq, p_after_digest) on: the records
-- and the gaps in order, a gap as one record whose digest is its link. A
-- record's content digest is recomputed from an entry's content and taken as
-- recorded from a tombstone; every record's link is recomputed and matched to
-- the element before it; a gap's start follows the element before it, and a
-- gap may cover the starting link. A missing position no gap accounts for is
-- a deleted record. The newest element is matched to the head.
CREATE OR REPLACE FUNCTION audit.verify_access_log_chain_after(p_after_seq bigint, p_after_digest text)
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
               CASE WHEN r.uid ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                    THEN r.uid::uuid END AS id,
               r.time_recorded, r.previous_digest, r.digest, false AS gap,
               r.digest IS DISTINCT FROM audit.access_log_link_digest(
                   r.previous_digest, r.chain_seq, r.content_digest)
                 OR (r.record_type = 'ACCESS_LOG_ENTRY'
                     AND r.content_digest IS DISTINCT FROM audit.access_log_content_digest(r.content))
                 OR audit.access_log_record_derive(r) IS DISTINCT FROM r AS modified
          FROM audit.access_log_record r
         WHERE r.chain_seq > p_after_seq
        UNION ALL
        SELECT g.first_seq, g.last_seq, NULL::uuid, NULL::timestamptz, NULL::text, g.link,
               true, false
          FROM audit.access_log_chain_gap g
         WHERE g.last_seq > p_after_seq
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

    SELECT n.last_seq, n.link INTO highest, highest_link
      FROM (
          (SELECT r.chain_seq AS last_seq, r.digest AS link
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

CREATE OR REPLACE FUNCTION audit.verify_access_log_chain_incremental(p_full boolean DEFAULT false)
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
           newest.last_seq, newest.link
      INTO damage, top_seq, top_digest
      FROM (SELECT 1) AS one
      LEFT JOIN LATERAL (
          SELECT n.last_seq, n.link
            FROM (
                (SELECT r.chain_seq AS last_seq, r.digest AS link
                   FROM audit.access_log_record r ORDER BY r.chain_seq DESC LIMIT 1)
                UNION ALL
                (SELECT g.last_seq, g.link
                   FROM audit.access_log_chain_gap g ORDER BY g.last_seq DESC LIMIT 1)
            ) AS n
           ORDER BY n.last_seq DESC LIMIT 1
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

-- ── the gaps an import brought ───────────────────────────────────────────────

CREATE TABLE access_log_imported_gap (
    system_id   text        NOT NULL,
    first_seq   bigint      NOT NULL,
    last_seq    bigint      NOT NULL,
    link        text        NOT NULL,
    signature   text,
    grounds     jsonb       NOT NULL,
    time_folded timestamptz NOT NULL,
    imported_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_log_imported_gap PRIMARY KEY (system_id, first_seq),
    CONSTRAINT ck_access_log_imported_gap_span CHECK (last_seq >= first_seq AND first_seq >= 1),
    CONSTRAINT ck_access_log_imported_gap_link CHECK (link ~ '^[0-9a-f]{64}$'),
    CONSTRAINT ck_access_log_imported_gap_grounds CHECK (jsonb_typeof(grounds) = 'array')
);

COMMENT ON TABLE access_log_imported_gap IS
    'The gaps of another system''s chain that imported logs brought, so the imported records stay verifiable against their checkpoint across a fold.';

CREATE TRIGGER access_log_imported_gap_append_only
    BEFORE UPDATE OR DELETE ON access_log_imported_gap
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();

-- ── grants ───────────────────────────────────────────────────────────────────

REVOKE ALL ON FUNCTION audit.fold_access_log_tombstones(timestamptz, integer) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_link_at(bigint) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON access_log_chain_gap, access_log_imported_gap
            FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON access_log_chain_gap, access_log_imported_gap
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT INSERT ON access_log_imported_gap TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.fold_access_log_tombstones(timestamptz, integer)
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.access_log_link_at(bigint)
            TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the access-log gap grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
