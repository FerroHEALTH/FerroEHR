-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the signed checkpoints of the access log, the signature of a link,
-- and the logs imported from another system with an EHR (#5129).
--
-- The draft access log (org.openehr.rm.access_log 0.1.0) seals the head of a
-- system's chain with an ACCESS_LOG_CHECKPOINT: its sequence and digest,
-- signed by the system's key. access_log_checkpoint keeps each one as its
-- ITS-JSON form, append-only.
--
-- ACCESS_LOG_LINK.signature signs a link's digest, which the insert trigger
-- computes, so the application signs after the insert, in the same
-- transaction: a link's signature may be set once, while it is NULL, and
-- nothing else of the record changes with it.
--
-- An EHR moved from another system brings its log: the records with their
-- links, every link between the first of them and the checkpoint they verify
-- against, and that checkpoint. They keep their system_id and links and form
-- no part of this system's chain, so they live apart, keyed by the
-- system_id and sequence of the chain they belong to. An imported entry is
-- replaced by its tombstone as a local one is.
--
-- No openEHR spec governs log signing or storage: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

-- ── the checkpoints ──────────────────────────────────────────────────────────

CREATE TABLE access_log_checkpoint (
    sequence    bigint      NOT NULL,
    system_id   text        NOT NULL,
    digest      text        NOT NULL,
    time_sealed timestamptz NOT NULL,
    content     text        NOT NULL,
    CONSTRAINT pk_access_log_checkpoint PRIMARY KEY (sequence),
    CONSTRAINT ck_access_log_checkpoint_digest CHECK (digest ~ '^[0-9a-f]{64}$')
);

COMMENT ON TABLE access_log_checkpoint IS
    'The signed ACCESS_LOG_CHECKPOINTs of this system''s chain, one per sealed position, append-only.';
COMMENT ON COLUMN access_log_checkpoint.content IS
    'The checkpoint''s ITS-JSON form, proof included.';

CREATE FUNCTION audit.access_log_checkpoint_append_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.access_log_checkpoint is append-only'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER access_log_checkpoint_append_only
    BEFORE UPDATE OR DELETE ON access_log_checkpoint
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_checkpoint_append_only();
CREATE TRIGGER access_log_checkpoint_no_truncate
    BEFORE TRUNCATE ON access_log_checkpoint
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_checkpoint_append_only();

-- The link of the record at a position of the chain, of either form, in
-- lower-case hexadecimal: what a checkpoint at that position seals.
CREATE FUNCTION audit.audit_chain_link_at(p_sequence bigint) RETURNS text
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT coalesce(
        (SELECT r.digest FROM audit.access_log_record r WHERE r.chain_seq = p_sequence),
        (SELECT encode(e.row_hash, 'hex') FROM audit.audit_event e WHERE e.chain_seq = p_sequence))
$$;

COMMENT ON FUNCTION audit.audit_chain_link_at(bigint) IS
    'The link digest of the record at a chain position, an access-log record or an audit_event record; NULL when none is there.';

-- ── the signature of a link, set once ────────────────────────────────────────

CREATE OR REPLACE FUNCTION audit.access_log_record_tombstone_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF OLD.signature IS NULL AND NEW.signature IS NOT NULL
       AND NEW.content = OLD.content
       AND (NEW.chain_seq, NEW.algorithm, NEW.previous_digest, NEW.content_digest,
            NEW.digest, NEW.stored_at)
           IS NOT DISTINCT FROM
           (OLD.chain_seq, OLD.algorithm, OLD.previous_digest, OLD.content_digest,
            OLD.digest, OLD.stored_at)
    THEN
        RETURN NEW;
    END IF;
    NEW := audit.access_log_record_derive(NEW);
    IF OLD.record_type <> 'ACCESS_LOG_ENTRY'
       OR NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_TOMBSTONE'
       OR NEW.uid IS DISTINCT FROM OLD.uid
       OR NEW.system_id IS DISTINCT FROM OLD.system_id
       OR NEW.time_recorded IS DISTINCT FROM OLD.time_recorded
       OR (NEW.chain_seq, NEW.algorithm, NEW.previous_digest, NEW.content_digest,
           NEW.digest, NEW.signature, NEW.stored_at)
          IS DISTINCT FROM
          (OLD.chain_seq, OLD.algorithm, OLD.previous_digest, OLD.content_digest,
           OLD.digest, OLD.signature, OLD.stored_at)
    THEN
        RAISE EXCEPTION
            'audit.access_log_record is append-only: an entry is only replaced by its own tombstone, and a link signed once (record %)',
            OLD.chain_seq
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT UPDATE (signature) ON access_log_record TO ferroehr_clinical;
    END IF;
END
$grants$;

-- ── the imported logs ────────────────────────────────────────────────────────

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

COMMENT ON TABLE access_log_imported_link IS
    'The ACCESS_LOG_LINKs of another system''s chain that imported logs brought, so the imported records stay verifiable against their checkpoint.';

-- One row per ACCESS_LOG_CHECKPOINT of another system an import brought.
CREATE TABLE access_log_imported_checkpoint (
    system_id   text        NOT NULL,
    sequence    bigint      NOT NULL,
    digest      text        NOT NULL,
    content     text        NOT NULL,
    imported_at timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_log_imported_checkpoint PRIMARY KEY (system_id, sequence)
);

COMMENT ON TABLE access_log_imported_checkpoint IS
    'The ACCESS_LOG_CHECKPOINTs of other systems that imported logs verify against, as their ITS-JSON form.';

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
    CONSTRAINT pk_access_log_imported_record PRIMARY KEY (system_id, sequence),
    CONSTRAINT uq_access_log_imported_record_uid UNIQUE (system_id, uid),
    CONSTRAINT fk_access_log_imported_record_link FOREIGN KEY (system_id, sequence)
        REFERENCES access_log_imported_link (system_id, sequence),
    CONSTRAINT ck_access_log_imported_record_type
        CHECK (record_type IN ('ACCESS_LOG_ENTRY', 'ACCESS_LOG_TOMBSTONE'))
);

COMMENT ON TABLE access_log_imported_record IS
    'The access-log records of another system an EHR transfer brought, with their system_id, links and retain_until kept; outside this system''s chain.';

CREATE INDEX ix_access_log_imported_record_ehr ON access_log_imported_record (ehr_id)
    WHERE ehr_id IS NOT NULL;
CREATE INDEX ix_access_log_imported_record_retain_until
    ON access_log_imported_record (retain_until) WHERE retain_until IS NOT NULL;

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

CREATE TRIGGER access_log_imported_link_append_only
    BEFORE UPDATE OR DELETE ON access_log_imported_link
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
CREATE TRIGGER access_log_imported_checkpoint_append_only
    BEFORE UPDATE OR DELETE ON access_log_imported_checkpoint
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();
CREATE TRIGGER access_log_imported_record_no_delete
    BEFORE DELETE ON access_log_imported_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_append_only();

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
    RETURN NEW;
END;
$$;

CREATE TRIGGER access_log_imported_record_tombstone_only
    BEFORE UPDATE ON access_log_imported_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_imported_tombstone_only();

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT ON access_log_checkpoint, access_log_imported_link,
            access_log_imported_checkpoint, access_log_imported_record
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT INSERT ON access_log_checkpoint, access_log_imported_link,
            access_log_imported_checkpoint, access_log_imported_record
            TO ferroehr_clinical;
        GRANT UPDATE (content) ON access_log_imported_record TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.audit_chain_link_at(bigint)
            TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping access-log checkpoint grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
