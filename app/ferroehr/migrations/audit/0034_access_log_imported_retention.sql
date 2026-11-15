-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the retention of the access logs imported from another system
-- (#5664).
--
-- An imported entry states its own retain_until, which the importing system
-- keeps (docs/plans/access-log.md §Retention), and the retention run
-- replaces it by its tombstone once that day has passed, as a local one is
-- (0028). Its tombstone is kept as long again, as a local one is (0033):
-- fold_after, derived from its content.
--
-- An imported log usually holds one checkpoint, at its end, and the links of
-- other EHRs' records between the moved EHR's own (bare links). A gap does
-- not commit to the link before it, so a record kept before a gap would be
-- tied to its checkpoint only by its own link signature. Within each span
-- between two imported checkpoints of a system, only the positions before
-- the first record still kept fold: tombstones past their end and bare links
-- alike, as one access_log_imported_gap per run that holds a tombstone. Every
-- kept imported record stays hash-chained to the checkpoint after it whether
-- or not the exporter signed its links.
--
-- A gap is now keyed by its whole span, so a gap this system folded and one
-- an exporter folded over the same first position are both kept, and an
-- import refuses a link or gap that ends where a stored one ends with
-- another digest.
--
-- No openEHR spec governs audit storage, chaining or retention: our own
-- design/extension.
--
-- Runs with search_path = audit, ext, public.

-- ── how long an imported tombstone is kept ───────────────────────────────────

ALTER TABLE access_log_imported_record ADD COLUMN fold_after timestamptz;

COMMENT ON COLUMN access_log_imported_record.fold_after IS
    'An imported tombstone''s end: time_removed plus the time its entry was kept (time_removed minus time_recorded), derived from content; NULL on an entry.';

-- The end of the tombstone whose canonical form is p_content; NULL for an
-- entry.
CREATE FUNCTION audit.access_log_tombstone_end(p_content text) RETURNS timestamptz
    LANGUAGE plpgsql STABLE
    SET search_path = pg_catalog
    SET TimeZone = 'UTC'
    AS $$
DECLARE
    doc      jsonb := p_content::jsonb;
    removed  timestamptz;
    recorded timestamptz;
BEGIN
    IF doc ->> '_type' IS DISTINCT FROM 'ACCESS_LOG_TOMBSTONE' THEN
        RETURN NULL;
    END IF;
    removed := (doc #>> '{time_removed,value}')::timestamptz;
    recorded := (doc #>> '{time_recorded,value}')::timestamptz;
    RETURN removed + (removed - recorded);
END;
$$;

CREATE OR REPLACE FUNCTION audit.access_log_imported_tombstone_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    doc jsonb := NEW.content::jsonb;
BEGIN
    IF OLD.record_type <> 'ACCESS_LOG_ENTRY'
       OR doc ->> '_type' IS DISTINCT FROM 'ACCESS_LOG_TOMBSTONE'
       OR doc #>> '{uid,value}' IS DISTINCT FROM OLD.uid
       OR doc ->> 'system_id' IS DISTINCT FROM OLD.system_id
       OR (NEW.system_id, NEW.sequence, NEW.uid, NEW.imported_at)
          IS DISTINCT FROM (OLD.system_id, OLD.sequence, OLD.uid, OLD.imported_at)
    THEN
        RAISE EXCEPTION
            'audit.access_log_imported_record is append-only: an entry is only replaced by its own tombstone (record %:%)',
            OLD.system_id, OLD.sequence
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW.record_type := 'ACCESS_LOG_TOMBSTONE';
    NEW.ehr_id := NULL;
    NEW.retain_until := NULL;
    NEW.fold_after := audit.access_log_tombstone_end(NEW.content);
    RETURN NEW;
END;
$$;

-- A record imported as a tombstone carries its end from the start.
CREATE FUNCTION audit.access_log_imported_record_end() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    NEW.fold_after := audit.access_log_tombstone_end(NEW.content);
    RETURN NEW;
END;
$$;

CREATE TRIGGER access_log_imported_record_end
    BEFORE INSERT ON access_log_imported_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_record_end();

ALTER TABLE access_log_imported_record DISABLE TRIGGER access_log_imported_record_tombstone_only;
UPDATE access_log_imported_record
   SET fold_after = audit.access_log_tombstone_end(content)
 WHERE record_type = 'ACCESS_LOG_TOMBSTONE';
ALTER TABLE access_log_imported_record ENABLE TRIGGER access_log_imported_record_tombstone_only;

-- ── a gap is keyed by its span ───────────────────────────────────────────────

ALTER TABLE access_log_imported_gap DROP CONSTRAINT pk_access_log_imported_gap;
ALTER TABLE access_log_imported_gap
    ADD CONSTRAINT pk_access_log_imported_gap PRIMARY KEY (system_id, first_seq, last_seq);
CREATE INDEX ix_access_log_imported_gap_last ON access_log_imported_gap (system_id, last_seq);

COMMENT ON TABLE access_log_imported_gap IS
    'The gaps of another system''s chain: those its exports brought and those this system''s retention folded, so the imported records stay verifiable against their checkpoint across a fold.';

-- ── the one way an imported record or link leaves ────────────────────────────

-- An imported tombstone and a link leave only inside the fold.
CREATE FUNCTION audit.access_log_imported_reject_removal() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF coalesce(current_setting('ferroehr.access_log_imported_folding', true), 'off') = 'on' THEN
        IF TG_TABLE_NAME = 'access_log_imported_link' THEN
            RETURN OLD;
        END IF;
        IF to_jsonb(OLD) ->> 'record_type' = 'ACCESS_LOG_TOMBSTONE' THEN
            RETURN OLD;
        END IF;
    END IF;
    RAISE EXCEPTION '% is append-only: an imported tombstone and its link leave only by the fold into a gap', TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
END;
$$;

DROP TRIGGER access_log_imported_record_no_delete ON access_log_imported_record;
CREATE TRIGGER access_log_imported_record_no_delete
    BEFORE DELETE ON access_log_imported_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_reject_removal();

DROP TRIGGER access_log_imported_link_append_only ON access_log_imported_link;
CREATE TRIGGER access_log_imported_link_append_only
    BEFORE UPDATE ON access_log_imported_link
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
CREATE TRIGGER access_log_imported_link_no_delete
    BEFORE DELETE ON access_log_imported_link
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_reject_removal();

-- ── the fold ─────────────────────────────────────────────────────────────────

-- Fold at p_now, per system, the positions before the first record still
-- kept in each span between two imported checkpoints: tombstones past their
-- end and bare links, one gap per run of consecutive links that holds a
-- tombstone. Whole runs fold, oldest first, until p_limit positions are
-- reached; the first run always folds. Replicas serialise on a
-- transaction-scoped advisory lock. Returns the tombstones and positions
-- removed and the gaps written; a caller repeats the call until no gap is
-- written.
CREATE FUNCTION audit.fold_access_log_imported(p_now timestamptz, p_limit integer)
    RETURNS TABLE (tombstones bigint, positions bigint, gaps bigint)
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF p_now IS NULL OR coalesce(p_limit, 0) < 1 THEN
        RETURN QUERY SELECT 0::bigint, 0::bigint, 0::bigint;
        RETURN;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtext('audit.access_log_imported_gap'), 0);
    PERFORM set_config('ferroehr.access_log_imported_folding', 'on', true);
    RETURN QUERY
    WITH segment AS (
        SELECT c.system_id, c.sequence AS last_seq,
               coalesce(lag(c.sequence) OVER (PARTITION BY c.system_id ORDER BY c.sequence), 0)
                   AS after_seq
          FROM audit.access_log_imported_checkpoint c
    ),
    bounded AS (
        SELECT s.system_id, s.after_seq, s.last_seq,
               coalesce((SELECT min(r.sequence) FROM audit.access_log_imported_record r
                          WHERE r.system_id = s.system_id
                            AND r.sequence > s.after_seq AND r.sequence <= s.last_seq
                            AND (r.fold_after IS NULL OR r.fold_after > p_now)),
                        s.last_seq + 1) AS first_kept
          FROM segment s
    ),
    due AS (
        SELECT l.system_id, l.sequence, l.digest, l.signature, b.last_seq AS segment,
               r.content AS tombstone
          FROM bounded b
          JOIN audit.access_log_imported_link l
            ON l.system_id = b.system_id
           AND l.sequence > b.after_seq AND l.sequence < b.first_kept
          LEFT JOIN audit.access_log_imported_record r
            ON r.system_id = l.system_id AND r.sequence = l.sequence
    ),
    islanded AS (
        SELECT d.*,
               d.sequence - row_number() OVER (
                   PARTITION BY d.system_id, d.segment ORDER BY d.sequence) AS island
          FROM due d
    ),
    runs AS (
        SELECT i.system_id, i.segment, i.island,
               min(i.sequence) AS first_seq, max(i.sequence) AS last_seq,
               (array_agg(i.digest ORDER BY i.sequence DESC))[1] AS link,
               (array_agg(i.signature ORDER BY i.sequence DESC))[1] AS signature,
               count(i.tombstone) AS held
          FROM islanded i
         GROUP BY i.system_id, i.segment, i.island
        HAVING count(i.tombstone) > 0
    ),
    chosen AS (
        SELECT r.*
          FROM (SELECT r.*,
                       sum(r.last_seq - r.first_seq + 1) OVER (
                           ORDER BY r.system_id, r.first_seq
                           ROWS BETWEEN UNBOUNDED PRECEDING AND 1 PRECEDING) AS before
                  FROM runs r) AS r
         WHERE coalesce(r.before, 0) < p_limit
    ),
    grounded AS (
        SELECT c.system_id, c.first_seq,
               jsonb_agg(DISTINCT g.ground ORDER BY g.ground) AS grounds
          FROM chosen c
          JOIN islanded i
            ON i.system_id = c.system_id AND i.sequence BETWEEN c.first_seq AND c.last_seq
          CROSS JOIN LATERAL jsonb_array_elements(i.tombstone::jsonb -> 'grounds') AS g(ground)
         GROUP BY c.system_id, c.first_seq
    ),
    written AS (
        INSERT INTO audit.access_log_imported_gap
               (system_id, first_seq, last_seq, link, signature, grounds, time_folded)
        SELECT c.system_id, c.first_seq, c.last_seq, c.link, c.signature, g.grounds, p_now
          FROM chosen c
          JOIN grounded g ON g.system_id = c.system_id AND g.first_seq = c.first_seq
         ORDER BY c.system_id, c.first_seq
        ON CONFLICT (system_id, first_seq, last_seq) DO NOTHING
        RETURNING 1
    ),
    removed_records AS (
        DELETE FROM audit.access_log_imported_record r
         USING chosen c
         WHERE r.system_id = c.system_id AND r.sequence BETWEEN c.first_seq AND c.last_seq
        RETURNING 1
    ),
    removed_links AS (
        DELETE FROM audit.access_log_imported_link l
         USING chosen c
         WHERE l.system_id = c.system_id AND l.sequence BETWEEN c.first_seq AND c.last_seq
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM removed_records),
           (SELECT count(*) FROM removed_links),
           (SELECT count(*) FROM chosen);
    PERFORM set_config('ferroehr.access_log_imported_folding', 'off', true);
END;
$$;

COMMENT ON FUNCTION audit.fold_access_log_imported(timestamptz, integer) IS
    'The one path an imported tombstone and link leave by: per system, the positions before the first record still kept between two imported checkpoints fold into one access_log_imported_gap per run that holds a tombstone.';

-- ── grants ───────────────────────────────────────────────────────────────────

REVOKE ALL ON FUNCTION audit.fold_access_log_imported(timestamptz, integer) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_tombstone_end(text) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT EXECUTE ON FUNCTION audit.fold_access_log_imported(timestamptz, integer)
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the imported access-log fold grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
