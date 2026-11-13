-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: a local access-log link is not signed on its own (#5562).
--
-- The checkpoints seal this system's chain whenever the keyring holds an
-- OpenPGP key, on a built-in schedule, so the signature of each local link
-- (0028) is gone: the record loses its signature column, the tombstone
-- trigger loses the one update it admitted for it, and the clinical role
-- loses the grant to set it. A link imported from another system keeps its
-- optional ACCESS_LOG_LINK.signature in access_log_imported_link.
--
-- No openEHR spec governs log signing or storage: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

CREATE OR REPLACE FUNCTION audit.access_log_record_chain_link() RETURNS trigger
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
    PERFORM set_config('ferroehr.audit_chain_head', NEW.chain_seq || ':' || NEW.digest, true);
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION audit.access_log_record_tombstone_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    NEW := audit.access_log_record_derive(NEW);
    IF OLD.record_type <> 'ACCESS_LOG_ENTRY'
       OR NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_TOMBSTONE'
       OR NEW.uid IS DISTINCT FROM OLD.uid
       OR NEW.system_id IS DISTINCT FROM OLD.system_id
       OR NEW.time_recorded IS DISTINCT FROM OLD.time_recorded
       OR (NEW.chain_seq, NEW.algorithm, NEW.previous_digest, NEW.content_digest,
           NEW.digest, NEW.stored_at)
          IS DISTINCT FROM
          (OLD.chain_seq, OLD.algorithm, OLD.previous_digest, OLD.content_digest,
           OLD.digest, OLD.stored_at)
    THEN
        RAISE EXCEPTION
            'audit.access_log_record is append-only: an entry is only replaced by its own tombstone (record %)',
            OLD.chain_seq
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE UPDATE (signature) ON access_log_record FROM ferroehr_clinical;
    END IF;
END
$grants$;

ALTER TABLE access_log_record DROP COLUMN signature;
