-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: a local access-log link is signed before its row is written and
-- stored by the same INSERT (#5562).
--
-- Whenever the keyring holds an OpenPGP key, the application locks the head
-- of the chain (audit.audit_chain_reserve), computes the links its rows will
-- get by the chain rule the insert trigger applies, signs each digest, and
-- inserts the content with its signature. The trigger keeps the signature it
-- is given instead of clearing it, and checks nothing of it: a signature over
-- another digest fails verification, and the application refuses the
-- transaction when a link it signed is not the link the trigger gave. The
-- signature is no longer set by an UPDATE after the insert (0028), so the
-- tombstone trigger loses that exception and the clinical role its UPDATE
-- grant on the column.
--
-- No openEHR spec governs log signing or storage: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

-- The head of the chain, locked until the transaction ends: the position and
-- link digest (lower-case hexadecimal) the next record follows.
CREATE FUNCTION audit.audit_chain_reserve()
    RETURNS TABLE (head_seq bigint, head_digest text)
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RETURN QUERY
        SELECT s.head_seq, encode(s.head_hash, 'hex')
          FROM audit.audit_chain_state s
         WHERE s.singleton
           FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'audit chain state is missing: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
END;
$$;

REVOKE ALL ON FUNCTION audit.audit_chain_reserve() FROM PUBLIC;

COMMENT ON FUNCTION audit.audit_chain_reserve() IS
    'Lock the head of the audit chain until the transaction ends and return its position and link digest, so a writer can sign the links its next rows will get.';

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
           NEW.digest, NEW.signature, NEW.stored_at)
          IS DISTINCT FROM
          (OLD.chain_seq, OLD.algorithm, OLD.previous_digest, OLD.content_digest,
           OLD.digest, OLD.signature, OLD.stored_at)
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
        GRANT INSERT (signature) ON access_log_record TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.audit_chain_reserve() TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping access-log signature grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
