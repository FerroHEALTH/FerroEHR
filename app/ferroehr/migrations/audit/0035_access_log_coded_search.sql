-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the access log's action and outcome search columns hold the coded
-- term, terminology id and code, in place of the rubric (#5687).
--
-- A rubric is display text (RM Data Types, DV_CODED_TEXT.value), and the
-- draft recognises a term by its defining_code. The columns are derived
-- from content and lie outside the chained content (content_digest is the
-- SHA-256 of content alone), so recomputing them changes no digest.
--
-- No openEHR spec governs audit storage: our own design.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE access_log_record
    DROP COLUMN action,
    DROP COLUMN result,
    ADD COLUMN action_terminology text,
    ADD COLUMN action_code text,
    ADD COLUMN result_terminology text,
    ADD COLUMN result_code text;

COMMENT ON COLUMN access_log_record.action_terminology IS
    'The terminology id of the entry''s action.defining_code, derived from content; NULL on a tombstone.';
COMMENT ON COLUMN access_log_record.action_code IS
    'The code_string of the entry''s action.defining_code, derived from content; NULL on a tombstone.';
COMMENT ON COLUMN access_log_record.result_terminology IS
    'The terminology id of the entry''s outcome.result.defining_code, derived from content; NULL on a tombstone.';
COMMENT ON COLUMN access_log_record.result_code IS
    'The code_string of the entry''s outcome.result.defining_code, derived from content; NULL on a tombstone.';

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
    p_row.action_terminology := doc #>> '{action,defining_code,terminology_id,value}';
    p_row.action_code := doc #>> '{action,defining_code,code_string}';
    p_row.result_terminology := doc #>> '{outcome,result,defining_code,terminology_id,value}';
    p_row.result_code := doc #>> '{outcome,result,defining_code,code_string}';
    p_row.party_id := doc #>> '{accessor,party,external_ref,id,value}';
    p_row.request_id := doc #>> '{channel,request_id}';
    p_row.retain_until := (doc #>> '{retention,retain_until,value}')::date;
    p_row.time_removed := (doc #>> '{time_removed,value}')::timestamptz;
    p_row.fold_after := p_row.time_removed + (p_row.time_removed - p_row.time_recorded);
    RETURN p_row;
END;
$$;

ALTER TABLE access_log_record DISABLE TRIGGER access_log_record_tombstone_only;
ALTER TABLE access_log_record DISABLE TRIGGER access_log_record_unindex;
UPDATE access_log_record
   SET action_terminology = content::jsonb #>> '{action,defining_code,terminology_id,value}',
       action_code = content::jsonb #>> '{action,defining_code,code_string}',
       result_terminology = content::jsonb #>> '{outcome,result,defining_code,terminology_id,value}',
       result_code = content::jsonb #>> '{outcome,result,defining_code,code_string}';
ALTER TABLE access_log_record ENABLE TRIGGER access_log_record_unindex;
ALTER TABLE access_log_record ENABLE TRIGGER access_log_record_tombstone_only;
