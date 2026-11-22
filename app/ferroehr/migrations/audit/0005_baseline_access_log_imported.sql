-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the access logs imported from other systems (#5129, #6242).
--
-- An export is one EHR's records, signed whole by the exporting system's
-- keyring. The importer verifies that signature, keeps the bundle's digest
-- and proof here, and keeps each record as it was exported with the content
-- digest it had there. The import's own access-log entry, which a seal
-- window of this system covers, names the bundle digest and a digest over
-- the imported records' uids and content digests, so the verifier reports a
-- later change to an imported record.
--
-- No openEHR spec governs the transfer: our own design.
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

-- An imported tombstone leaves only by the fold of tombstones past their end.
CREATE FUNCTION audit.access_log_imported_reject_removal() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF coalesce(current_setting('ferroehr.access_log_imported_folding', true), 'off') = 'on'
       AND OLD.record_type = 'ACCESS_LOG_TOMBSTONE'
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION '% is append-only: an imported tombstone leaves only by the fold', TG_TABLE_NAME
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
       OR (NEW.system_id, NEW.uid, NEW.bundle, NEW.content_digest, NEW.imported_at)
          IS DISTINCT FROM (OLD.system_id, OLD.uid, OLD.bundle, OLD.content_digest,
                            OLD.imported_at)
    THEN
        RAISE EXCEPTION
            'audit.access_log_imported_record is append-only: an entry is only replaced by its own tombstone (record %:%)',
            OLD.system_id, OLD.uid
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW.record_type := 'ACCESS_LOG_TOMBSTONE';
    NEW.ehr_id := NULL;
    NEW.retain_until := NULL;
    NEW.fold_after := audit.access_log_tombstone_end(NEW.content);
    RETURN NEW;
END;
$$;

-- One row per export imported: its digest, the exporting system's signature
-- over it, and the digest over its records' uids and content digests.
CREATE TABLE access_log_imported_bundle (
    digest         text        NOT NULL,
    system_id      text        NOT NULL,
    ehr_id         text        NOT NULL,
    records_digest text        NOT NULL,
    time_signed    text        NOT NULL,
    proof          text        NOT NULL,
    -- The import's own access-log entry, which names this bundle.
    entry_uid      uuid        NOT NULL,
    imported_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_log_imported_bundle PRIMARY KEY (digest),
    CONSTRAINT ck_access_log_imported_bundle_digests
        CHECK (digest ~ '^[0-9a-f]{64}$' AND records_digest ~ '^[0-9a-f]{64}$'),
    CONSTRAINT ck_access_log_imported_bundle_proof CHECK (proof <> '')
);
CREATE TRIGGER access_log_imported_bundle_append_only
    BEFORE UPDATE OR DELETE ON access_log_imported_bundle
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
COMMENT ON TABLE access_log_imported_bundle IS
    'The signed exports of other systems that EHR transfers brought: each bundle''s digest, signature and the digest over its records'' uids and content digests.';

-- One row per imported record: its canonical form as it was exported, and
-- the content digest it had there, which a tombstone keeps.
CREATE TABLE access_log_imported_record (
    system_id      text        NOT NULL,
    uid            text        NOT NULL,
    bundle         text        NOT NULL,
    record_type    text        NOT NULL,
    ehr_id         text,
    retain_until   date,
    content        text        NOT NULL,
    content_digest text        NOT NULL,
    imported_at    timestamptz NOT NULL DEFAULT now(),
    -- An imported tombstone's end, derived from its content; NULL on an
    -- entry.
    fold_after     timestamptz,
    CONSTRAINT pk_access_log_imported_record PRIMARY KEY (system_id, uid),
    CONSTRAINT ck_access_log_imported_record_type
        CHECK (record_type IN ('ACCESS_LOG_ENTRY', 'ACCESS_LOG_TOMBSTONE')),
    CONSTRAINT ck_access_log_imported_record_digest CHECK (content_digest ~ '^[0-9a-f]{64}$')
);
CREATE INDEX ix_access_log_imported_record_ehr ON access_log_imported_record (ehr_id)
    WHERE ehr_id IS NOT NULL;
CREATE INDEX ix_access_log_imported_record_retain_until
    ON access_log_imported_record (retain_until) WHERE retain_until IS NOT NULL;
CREATE INDEX ix_access_log_imported_record_fold_after
    ON access_log_imported_record (fold_after) WHERE fold_after IS NOT NULL;
CREATE INDEX ix_access_log_imported_record_bundle ON access_log_imported_record (bundle);
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
    'The access-log records of another system an EHR transfer brought, as exported, with their system_id, retain_until and content digest kept.';
COMMENT ON COLUMN access_log_imported_record.fold_after IS
    'An imported tombstone''s end: time_removed plus the time its entry was kept (time_removed minus time_recorded), derived from content; NULL on an entry.';

-- What a fold kept of each imported tombstone it removed: its uid, its
-- bundle and its content digest, which the bundle's records digest covers.
CREATE TABLE access_log_imported_folded (
    system_id      text        NOT NULL,
    uid            text        NOT NULL,
    bundle         text        NOT NULL,
    content_digest text        NOT NULL,
    time_folded    timestamptz NOT NULL,
    CONSTRAINT pk_access_log_imported_folded PRIMARY KEY (system_id, uid)
);
CREATE INDEX ix_access_log_imported_folded_bundle ON access_log_imported_folded (bundle);
CREATE TRIGGER access_log_imported_folded_append_only
    BEFORE UPDATE OR DELETE ON access_log_imported_folded
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
COMMENT ON TABLE access_log_imported_folded IS
    'The imported tombstones the fold removed, by uid, bundle and content digest, so each bundle''s records digest still verifies.';

-- Removes up to p_limit imported tombstones past their end, oldest end
-- first, keeping each one's uid, bundle and content digest.
CREATE FUNCTION audit.fold_access_log_imported(p_now timestamptz, p_limit integer)
    RETURNS TABLE (tombstones bigint, gaps bigint)
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF p_now IS NULL OR coalesce(p_limit, 0) < 1 THEN
        RETURN QUERY SELECT 0::bigint, 0::bigint;
        RETURN;
    END IF;
    PERFORM set_config('ferroehr.access_log_imported_folding', 'on', true);
    RETURN QUERY
    WITH due AS (
        SELECT r.system_id, r.uid
          FROM audit.access_log_imported_record r
         WHERE r.fold_after <= p_now
         ORDER BY r.fold_after, r.system_id, r.uid
         LIMIT p_limit
           FOR UPDATE SKIP LOCKED
    ),
    removed AS (
        DELETE FROM audit.access_log_imported_record r
         USING due d
         WHERE r.system_id = d.system_id AND r.uid = d.uid
        RETURNING r.system_id, r.uid, r.bundle, r.content_digest
    ),
    kept AS (
        INSERT INTO audit.access_log_imported_folded
               (system_id, uid, bundle, content_digest, time_folded)
        SELECT x.system_id, x.uid, x.bundle, x.content_digest, p_now FROM removed x
        RETURNING 1
    )
    SELECT count(*), (SELECT count(*) FROM kept) FROM removed;
    PERFORM set_config('ferroehr.access_log_imported_folding', 'off', true);
END;
$$;
COMMENT ON FUNCTION fold_access_log_imported(timestamptz, integer) IS
    'Removes up to p_limit imported tombstones past their end, keeping in access_log_imported_folded what each bundle''s records digest covers.';
