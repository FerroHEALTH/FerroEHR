-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: an access-log entry is one row (#5784).
--
-- An entry took a row of access_log_record and a row of access_log_search,
-- the ITI-81 index beside it: a second tuple header, a second primary key,
-- the position and the time twice, and an index on each time column. The
-- index columns are now columns of the record, NULL on a tombstone and on
-- an entry whose writer indexes no projection, and access_log_search is a
-- view over them, so the ITI-81 search reads what it read before. The
-- index's recorded_at is the entry's time_recorded: both carry the instant
-- of the access, and the document a search serves is rendered from
-- time_recorded. A tombstone empties the index columns where its update
-- removed the index row.
--
-- The entry's content is not compressed. PostgreSQL compresses a value only
-- when the row exceeds TOAST_TUPLE_THRESHOLD, about 2 kB; toast_tuple_target
-- sets the size it compresses towards once it does (PostgreSQL 18 docs,
-- "TOAST"; measured on 18.7: a 1 960-byte value stays raw at
-- toast_tuple_target = 128, a 2 016-byte one compresses).
--
-- No openEHR spec governs audit storage: our own design.
--
-- Runs with search_path = audit, ext, public.

-- ── the index columns of the record ─────────────────────────────────────────

DROP VIEW audit_record;

ALTER TABLE access_log_record
    ADD COLUMN event_action      text,
    ADD COLUMN event_outcome     smallint,
    ADD COLUMN principal         text,
    ADD COLUMN patient_id        text,
    ADD COLUMN resource_id       text,
    ADD COLUMN request_id        text,
    ADD COLUMN client_ip         text,
    ADD COLUMN classified_ehr_id text,
    ADD COLUMN search_tokens     integer[],
    ADD COLUMN render_policy     bytea;

ALTER TABLE access_log_record
    ALTER COLUMN event_action SET STORAGE MAIN,
    ALTER COLUMN principal SET STORAGE MAIN,
    ALTER COLUMN patient_id SET STORAGE MAIN,
    ALTER COLUMN resource_id SET STORAGE MAIN,
    ALTER COLUMN request_id SET STORAGE MAIN,
    ALTER COLUMN client_ip SET STORAGE MAIN,
    ALTER COLUMN classified_ehr_id SET STORAGE MAIN,
    ALTER COLUMN search_tokens SET STORAGE MAIN,
    ALTER COLUMN render_policy SET STORAGE MAIN;

ALTER TABLE access_log_record DISABLE TRIGGER access_log_record_tombstone_only;
ALTER TABLE access_log_record DISABLE TRIGGER access_log_record_unindex;

UPDATE access_log_record r
   SET event_action = s.action, event_outcome = s.outcome, principal = s.principal,
       patient_id = s.patient_id, resource_id = s.resource_id, request_id = s.request_id,
       client_ip = s.client_ip, classified_ehr_id = s.ehr_id, search_tokens = s.search_tokens,
       render_policy = s.render_policy
  FROM access_log_search s
 WHERE s.chain_seq = r.chain_seq;

ALTER TABLE access_log_record ENABLE TRIGGER access_log_record_tombstone_only;
ALTER TABLE access_log_record ENABLE TRIGGER access_log_record_unindex;

DROP TABLE access_log_search;

ALTER TABLE access_log_record
    ADD CONSTRAINT ck_access_log_record_index
        CHECK ((render_policy IS NULL) = (event_action IS NULL)
               AND (render_policy IS NULL) = (event_outcome IS NULL)
               AND (render_policy IS NULL) = (search_tokens IS NULL)
               AND (render_policy IS NOT NULL
                    OR num_nonnulls(principal, patient_id, resource_id, request_id, client_ip,
                                    classified_ehr_id) = 0)
               AND (render_policy IS NULL OR record_type = 'ACCESS_LOG_ENTRY')),
    ADD CONSTRAINT ck_access_log_record_event_action
        CHECK (event_action IN ('C', 'R', 'U', 'D', 'E')),
    ADD CONSTRAINT ck_access_log_record_event_outcome CHECK (event_outcome IN (0, 4, 8));

COMMENT ON COLUMN access_log_record.event_action IS
    'The DICOM EventActionCode (PS3.15 A.5.1) the entry''s projection writes; NULL when the entry is not indexed.';
COMMENT ON COLUMN access_log_record.event_outcome IS
    'The DICOM EventOutcomeIndicator (PS3.15 A.5.1) the entry''s projection writes; NULL when the entry is not indexed.';
COMMENT ON COLUMN access_log_record.principal IS
    'The caller the entry''s projection names, for the ITI-81 search.';
COMMENT ON COLUMN access_log_record.patient_id IS
    'The subject of care of the entry''s EHR, as the entry''s access details name it, for the ITI-81 search.';
COMMENT ON COLUMN access_log_record.resource_id IS
    'The object the access reached, for the ITI-81 search.';
COMMENT ON COLUMN access_log_record.request_id IS
    'The request the access belongs to, for the ITI-81 search.';
COMMENT ON COLUMN access_log_record.client_ip IS
    'The caller''s network address, for the ITI-81 search.';
COMMENT ON COLUMN access_log_record.classified_ehr_id IS
    'The EHR of a classified access, which its EHDS access record names; NULL for every other entry.';
COMMENT ON COLUMN access_log_record.search_tokens IS
    'The ids of the search tokens of the entry''s projection, or of their shared set; NULL when the entry is not indexed.';
COMMENT ON COLUMN access_log_record.render_policy IS
    'The render policy the entry''s projection is written under; NULL on a tombstone and on an entry its writer indexes no projection of, which the ITI-81 search does not find.';

CREATE INDEX ix_access_log_record_patient ON access_log_record (patient_id, time_recorded)
    WHERE patient_id IS NOT NULL;
CREATE INDEX ix_access_log_record_principal ON access_log_record (principal, time_recorded)
    WHERE principal IS NOT NULL;
CREATE INDEX ix_access_log_record_request_id ON access_log_record (hashtextextended(request_id, 0));
CREATE INDEX ix_access_log_record_resource_id ON access_log_record (hashtextextended(resource_id, 0));
CREATE INDEX ix_access_log_record_tokens ON access_log_record USING gin (search_tokens);

-- ── the views ───────────────────────────────────────────────────────────────

CREATE VIEW access_log_search AS
    SELECT r.chain_seq, r.time_recorded AS recorded_at, r.event_action AS action,
           r.event_outcome AS outcome, r.principal, r.patient_id, r.resource_id,
           r.request_id, r.client_ip, r.classified_ehr_id AS ehr_id, r.search_tokens,
           r.render_policy
      FROM access_log_record r
     WHERE r.render_policy IS NOT NULL;

COMMENT ON VIEW access_log_search IS
    'The ITI-81 index of the access-log entries: what the search filters on and the render policy an entry''s projection is written under, read from the entry''s row. A tombstone is not in it.';

-- As 0039, with the access log's index read from the record.
CREATE VIEW audit_record AS
    SELECT e.chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.operation,
           e.principal, e.patient_id, e.resource_class, e.resource_id,
           e.client_ip, e.token_id, e.domain, e.purpose, e.legal_basis, e.result_count,
           e.request_id, e.roles, e.origins, e.origin_count, e.category_map_digest,
           e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id,
           e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id,
           e.subject_namespace, e.subject_lookup_failed, e.category_content,
           e.render_policy, e.token_attributes, e.search_tokens,
           NULL::text AS content, NULL::text AS shared, NULL::text AS algorithm,
           NULL::text AS previous_digest, NULL::text AS content_digest, NULL::text AS digest,
           'system'::text AS chain
      FROM audit_event e
    UNION ALL
    SELECT r.chain_seq, r.time_recorded, r.stored_at, r.event_action, r.event_outcome, NULL::text,
           r.principal, r.patient_id, NULL::text, r.resource_id,
           r.client_ip, NULL::text, NULL::text, NULL::text, NULL::text, NULL::bigint,
           r.request_id, NULL::jsonb, NULL::jsonb, NULL::bigint, NULL::text,
           NULL::text, NULL::text, NULL::text, NULL::text,
           false, NULL::text, NULL::text, NULL::text, r.classified_ehr_id,
           NULL::text, false, NULL::jsonb,
           r.render_policy, NULL::jsonb, r.search_tokens,
           r.content, sh.content, r.algorithm, encode(r.previous_digest, 'hex'),
           encode(r.content_digest, 'hex'), encode(r.digest, 'hex'),
           'access_log'::text
      FROM access_log_record r
      LEFT JOIN access_log_shared sh ON sh.digest = r.shared
     WHERE r.render_policy IS NOT NULL;

COMMENT ON VIEW audit_record IS
    'Every record the ITI-81 search, the person''s own trail and the FHIR Feed read: an audit_event record of the system chain by its columns, an access-log entry by its row and shared parts and its link, which its projection is rendered from. A record is named by its chain and position.';

-- ── the move into the chain ─────────────────────────────────────────────────

-- As 0042, with the index columns written on the record's own row.
CREATE OR REPLACE FUNCTION audit.access_log_chain_move(
    p_ids bigint[], p_seqs bigint[], p_previous text[], p_digests text[],
    p_fhir_feed boolean, p_syslog boolean)
    RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    moved bigint;
BEGIN
    INSERT INTO audit.access_log_record
           (chain_seq, record_type, uid, time_recorded, content, algorithm, previous_digest,
            content_digest, digest, ehr_id, party_id, retain_until, action_terminology,
            action_code, result_terminology, result_code, shared, event_action, event_outcome,
            principal, patient_id, resource_id, request_id, client_ip, classified_ehr_id,
            search_tokens, render_policy)
    SELECT l.seq, i.record_type, i.uid, i.time_recorded, i.content, 'SHA-256',
           decode(l.previous, 'hex'), i.content_digest, decode(l.digest, 'hex'), i.ehr_id,
           i.party_id, i.retain_until, i.action_terminology, i.action_code,
           i.result_terminology, i.result_code, i.shared, i.action, i.outcome, i.principal,
           i.patient_id, i.resource_id, i.request_id, i.client_ip, i.index_ehr_id,
           i.search_tokens, i.render_policy
      FROM unnest(p_ids, p_seqs, p_previous, p_digests) AS l(id, seq, previous, digest)
      JOIN audit.access_log_intake i ON i.id = l.id AND i.reserved_by IS NULL
     ORDER BY l.seq;
    GET DIAGNOSTICS moved = ROW_COUNT;
    IF moved IS DISTINCT FROM cardinality(p_ids)::bigint THEN
        RAISE EXCEPTION 'the chainer named % intake rows and % are in the intake', cardinality(p_ids), moved
            USING ERRCODE = 'restrict_violation';
    END IF;
    IF p_fhir_feed THEN
        INSERT INTO audit.fhir_feed_outbox (chain, chain_seq)
        SELECT 'access_log', l.seq
          FROM unnest(p_ids, p_seqs) AS l(id, seq)
          JOIN audit.access_log_intake i ON i.id = l.id
         WHERE i.render_policy IS NOT NULL
         ORDER BY l.seq;
    END IF;
    IF p_syslog THEN
        INSERT INTO audit.syslog_outbox (chain, chain_seq)
        SELECT 'access_log', l.seq
          FROM unnest(p_ids, p_seqs) AS l(id, seq)
          JOIN audit.access_log_intake i ON i.id = l.id
         WHERE i.render_policy IS NOT NULL
         ORDER BY l.seq;
    END IF;
    PERFORM set_config('ferroehr.audit_chaining', 'on', true);
    DELETE FROM audit.access_log_intake i WHERE i.id = ANY (p_ids);
    PERFORM set_config('ferroehr.audit_chaining', 'off', true);
    RETURN moved;
END;
$$;

-- ── the tombstone ───────────────────────────────────────────────────────────

-- As 0039, with the entry's index columns emptied: a tombstone is not
-- searched.
CREATE OR REPLACE FUNCTION audit.access_log_record_tombstone_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    common text;
BEGIN
    NEW := audit.access_log_record_derive(NEW);
    SELECT s.content INTO common FROM audit.access_log_shared s WHERE s.digest = OLD.shared;
    IF OLD.record_type <> 'ACCESS_LOG_ENTRY'
       OR NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_TOMBSTONE'
       OR NEW.shared IS NOT NULL
       OR NEW.uid IS DISTINCT FROM OLD.uid
       OR NEW.content::jsonb ->> 'system_id'
          IS DISTINCT FROM audit.access_log_document(OLD.content, common) ->> 'system_id'
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
    NEW.event_action := NULL;
    NEW.event_outcome := NULL;
    NEW.principal := NULL;
    NEW.patient_id := NULL;
    NEW.resource_id := NULL;
    NEW.request_id := NULL;
    NEW.client_ip := NULL;
    NEW.classified_ehr_id := NULL;
    NEW.search_tokens := NULL;
    NEW.render_policy := NULL;
    RETURN NEW;
END;
$$;

-- As 0042, with no index row to remove.
CREATE OR REPLACE FUNCTION audit.access_log_record_unindex() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF NEW.record_type = 'ACCESS_LOG_TOMBSTONE' THEN
        DELETE FROM audit.fhir_feed_outbox o
         WHERE o.chain = 'access_log' AND o.chain_seq = NEW.chain_seq;
        DELETE FROM audit.syslog_outbox o
         WHERE o.chain = 'access_log' AND o.chain_seq = NEW.chain_seq;
        IF OLD.shared IS NOT NULL THEN
            PERFORM FROM audit.access_log_shared s WHERE s.digest = OLD.shared FOR UPDATE;
            DELETE FROM audit.access_log_shared s
             WHERE s.digest = OLD.shared
               AND NOT EXISTS (SELECT 1 FROM audit.access_log_record r WHERE r.shared = OLD.shared)
               AND NOT EXISTS (SELECT 1 FROM audit.access_log_intake i WHERE i.shared = OLD.shared);
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

-- ── grants ──────────────────────────────────────────────────────────────────

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT ON access_log_search, audit_record
            TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the access-log view grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
