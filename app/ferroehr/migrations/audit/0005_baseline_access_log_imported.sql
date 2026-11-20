-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the access logs imported from other systems, kept as they were
-- signed there.
--
-- Runs with search_path = audit, ext, public.

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

CREATE FUNCTION audit.access_log_imported_tombstone_only() RETURNS trigger
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

-- One row per link of another system's chain an import brought: the links of
-- the imported records and every link between them and the checkpoint.
CREATE TABLE access_log_imported_link (
    system_id       text   NOT NULL,
    sequence        bigint NOT NULL,
    algorithm       text   NOT NULL,
    previous_digest text   NOT NULL,
    content_digest  text   NOT NULL,
    digest          text   NOT NULL,
    signature       text,
    CONSTRAINT pk_access_log_imported_link PRIMARY KEY (system_id, sequence),
    CONSTRAINT ck_access_log_imported_link_digests
        CHECK (previous_digest ~ '^[0-9a-f]{64}$'
               AND content_digest ~ '^[0-9a-f]{64}$'
               AND digest ~ '^[0-9a-f]{64}$')
);
CREATE TRIGGER access_log_imported_link_append_only
    BEFORE UPDATE ON access_log_imported_link
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
CREATE TRIGGER access_log_imported_link_no_delete
    BEFORE DELETE ON access_log_imported_link
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_reject_removal();
COMMENT ON TABLE access_log_imported_link IS
    'The ACCESS_LOG_LINKs of another system''s chain that imported logs brought, so the imported records stay verifiable against their checkpoint.';

-- One row per imported record: its canonical form without its link, as
-- access_log_record keeps a local one, beside the link its chain gave it.
CREATE TABLE access_log_imported_record (
    system_id     text        NOT NULL,
    sequence      bigint      NOT NULL,
    uid           text        NOT NULL,
    record_type   text        NOT NULL,
    ehr_id        text,
    retain_until  date,
    content       text        NOT NULL,
    imported_at   timestamptz NOT NULL DEFAULT now(),
    -- An imported tombstone's end, derived from its content; NULL on an
    -- entry.
    fold_after    timestamptz,
    CONSTRAINT pk_access_log_imported_record PRIMARY KEY (system_id, sequence),
    CONSTRAINT uq_access_log_imported_record_uid UNIQUE (system_id, uid),
    CONSTRAINT fk_access_log_imported_record_link FOREIGN KEY (system_id, sequence)
        REFERENCES access_log_imported_link (system_id, sequence),
    CONSTRAINT ck_access_log_imported_record_type
        CHECK (record_type IN ('ACCESS_LOG_ENTRY', 'ACCESS_LOG_TOMBSTONE'))
);
CREATE INDEX ix_access_log_imported_record_ehr ON access_log_imported_record (ehr_id)
    WHERE ehr_id IS NOT NULL;
CREATE INDEX ix_access_log_imported_record_retain_until
    ON access_log_imported_record (retain_until) WHERE retain_until IS NOT NULL;
CREATE TRIGGER access_log_imported_record_tombstone_only
    BEFORE UPDATE ON access_log_imported_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_tombstone_only();
CREATE TRIGGER access_log_imported_record_end
    BEFORE INSERT ON access_log_imported_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_record_end();
CREATE TRIGGER access_log_imported_record_no_delete
    BEFORE DELETE ON access_log_imported_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_reject_removal();
COMMENT ON TABLE access_log_imported_record IS
    'The access-log records of another system an EHR transfer brought, with their system_id, links and retain_until kept; outside this system''s chain.';
COMMENT ON COLUMN access_log_imported_record.fold_after IS
    'An imported tombstone''s end: time_removed plus the time its entry was kept (time_removed minus time_recorded), derived from content; NULL on an entry.';

-- One row per ACCESS_LOG_CHECKPOINT of another system an import brought.
CREATE TABLE access_log_imported_checkpoint (
    system_id   text        NOT NULL,
    sequence    bigint      NOT NULL,
    digest      text        NOT NULL,
    content     text        NOT NULL,
    imported_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_log_imported_checkpoint PRIMARY KEY (system_id, sequence)
);
CREATE TRIGGER access_log_imported_checkpoint_append_only
    BEFORE UPDATE OR DELETE ON access_log_imported_checkpoint
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
COMMENT ON TABLE access_log_imported_checkpoint IS
    'The ACCESS_LOG_CHECKPOINTs of other systems that imported logs verify against, as their ITS-JSON form.';

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
    CONSTRAINT pk_access_log_imported_gap PRIMARY KEY (system_id, first_seq, last_seq),
    CONSTRAINT ck_access_log_imported_gap_span CHECK (last_seq >= first_seq AND first_seq >= 1),
    CONSTRAINT ck_access_log_imported_gap_link CHECK (link ~ '^[0-9a-f]{64}$'),
    CONSTRAINT ck_access_log_imported_gap_grounds CHECK (jsonb_typeof(grounds) = 'array')
);
CREATE INDEX ix_access_log_imported_gap_last ON access_log_imported_gap (system_id, last_seq);
CREATE TRIGGER access_log_imported_gap_append_only
    BEFORE UPDATE OR DELETE ON access_log_imported_gap
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
COMMENT ON TABLE access_log_imported_gap IS
    'The gaps of another system''s chain: those its exports brought and those this system''s retention folded, so the imported records stay verifiable against their checkpoint across a fold.';

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
COMMENT ON FUNCTION fold_access_log_imported(timestamptz, integer) IS
    'The one path an imported tombstone and link leave by: per system, the positions before the first record still kept between two imported checkpoints fold into one access_log_imported_gap per run that holds a tombstone.';
