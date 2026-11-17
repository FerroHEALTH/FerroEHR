-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: an access-log entry is one row, and its row keeps no member a
-- column already holds (#5784).
--
-- One row: an entry took a row of access_log_record and a row of
-- access_log_search, the ITI-81 index beside it: a second tuple header, a
-- second primary key, the position and the time twice, and an index on each
-- time column. The index columns are now columns of the record, NULL on a
-- tombstone and on an entry whose writer indexes no projection of it, and
-- access_log_search is a view over them. The index's recorded_at is the
-- entry's time_recorded. The render policy is named by the id of its
-- audit_render_policy row instead of its 32-byte digest.
--
-- No member twice: an entry's _type, uid, time_recorded and ehr_id are
-- columns, so the writer leaves them out of the row part whenever the column
-- gives back their exact text: _type always, a uid and an ehr_id in the
-- lower-case hyphenated form of a UUID, a time in UTC to the microsecond in
-- the form audit.access_log_instant_text writes. ehr_id is a uuid, NULL when
-- the entry's EHR id is no such UUID, which then stays in the row part. A row
-- part without _type is such an entry; one with it (a tombstone, an entry
-- written before) holds its members itself. audit.access_log_document is the
-- one reader of an entry: it joins the row part, the shared part and the
-- columns into the record's canonical form without its link, the access
-- details merged in the order of their codes as the writer wrote them.
--
-- The content is not compressed: PostgreSQL compresses a value only when its
-- row exceeds TOAST_TUPLE_THRESHOLD, about 2 kB, whatever toast_tuple_target
-- says (PostgreSQL 18 docs, "TOAST"; measured on 18.7: a 1 960-byte value
-- stays raw at toast_tuple_target = 128, a 2 016-byte one compresses).
--
-- No openEHR spec governs audit storage: our own design.
--
-- Runs with search_path = audit, ext, public.

-- ── the reader of an entry ──────────────────────────────────────────────────

-- The UUID p_text is in its lower-case hyphenated form; NULL for any other
-- text, which a uuid column could not give back as it was written. It has no
-- SET clause, so the planner can inline it and the move runs no nested
-- statement per entry.
CREATE FUNCTION audit.access_log_canonical_uuid(p_text text) RETURNS uuid
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    AS $$
    SELECT CASE WHEN p_text ~ '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                THEN p_text::uuid END
$$;

COMMENT ON FUNCTION audit.access_log_canonical_uuid(text) IS
    'A UUID written in its lower-case hyphenated form, as a uuid; NULL for any other text.';

-- The instant p_at as the writer leaves it out of a row part: UTC, a
-- four-digit year, the seconds and the microseconds without trailing zeros.
CREATE FUNCTION audit.access_log_instant_text(p_at timestamptz) RETURNS text
    LANGUAGE sql STABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT to_char(u.at, 'YYYY-MM-DD"T"HH24:MI:SS')
           || CASE WHEN u.micros = 0 THEN ''
                   ELSE '.' || rtrim(lpad(u.micros::text, 6, '0'), '0') END
           || 'Z'
      FROM (SELECT p_at AT TIME ZONE 'UTC' AS at,
                   (extract(microseconds FROM p_at AT TIME ZONE 'UTC')::bigint % 1000000) AS micros) AS u
$$;

COMMENT ON FUNCTION audit.access_log_instant_text(timestamptz) IS
    'An instant as an access-log row part leaves it out: UTC, the microseconds without trailing zeros, Z.';

CREATE FUNCTION audit.access_log_document(
    p_content text, p_shared text, p_type text, p_uid uuid, p_time timestamptz, p_ehr_id uuid)
    RETURNS jsonb
    LANGUAGE sql STABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE WHEN own ? '_type' THEN '{}'::jsonb
                ELSE jsonb_build_object(
                         '_type', p_type,
                         'uid', jsonb_build_object('_type', 'HIER_OBJECT_ID', 'value', p_uid::text),
                         'time_recorded', jsonb_build_object(
                             '_type', 'DV_DATE_TIME',
                             'value', audit.access_log_instant_text(p_time)))
                     || CASE WHEN p_ehr_id IS NULL THEN '{}'::jsonb
                             ELSE jsonb_build_object('ehr_id', jsonb_build_object(
                                      '_type', 'HIER_OBJECT_ID', 'value', p_ehr_id::text))
                        END
           END
           || CASE
                WHEN common IS NULL THEN own
                ELSE (common - 'other_details' - 'outcome' - 'query')
                     || (own - 'other_details' - 'outcome' - 'query')
                     || CASE WHEN common ? 'other_details' OR own ? 'other_details' THEN
                            jsonb_build_object('other_details',
                                coalesce(common -> 'other_details', '{}'::jsonb)
                                || coalesce(own -> 'other_details', '{}'::jsonb)
                                || jsonb_build_object('items', (
                                       SELECT coalesce(jsonb_agg(i.item ORDER BY
                                                  i.item ->> 'archetype_node_id' COLLATE "C" NULLS FIRST,
                                                  i.side, i.n), '[]'::jsonb)
                                         FROM (SELECT c.item, 0 AS side, c.n
                                                 FROM jsonb_array_elements(coalesce(
                                                          common #> '{other_details,items}', '[]'::jsonb))
                                                      WITH ORDINALITY AS c(item, n)
                                               UNION ALL
                                               SELECT o.item, 1, o.n
                                                 FROM jsonb_array_elements(coalesce(
                                                          own #> '{other_details,items}', '[]'::jsonb))
                                                      WITH ORDINALITY AS o(item, n)) AS i)))
                        ELSE '{}'::jsonb END
                     || CASE WHEN common ? 'outcome' OR own ? 'outcome' THEN
                            jsonb_build_object('outcome',
                                coalesce(common -> 'outcome', '{}'::jsonb)
                                || coalesce(own -> 'outcome', '{}'::jsonb))
                        ELSE '{}'::jsonb END
                     || CASE WHEN common ? 'query' OR own ? 'query' THEN
                            jsonb_build_object('query',
                                coalesce(common -> 'query', '{}'::jsonb)
                                || coalesce(own -> 'query', '{}'::jsonb))
                        ELSE '{}'::jsonb END
              END
      FROM (SELECT p_content::jsonb AS own, p_shared::jsonb AS common) AS parts
$$;

COMMENT ON FUNCTION audit.access_log_document(text, text, text, uuid, timestamptz, uuid) IS
    'An access-log record as one jsonb, its canonical form without its link: the row part, the shared part and the members the columns hold, the access details in the order of their codes. Every reader of a record reads it here.';

-- ── the columns ─────────────────────────────────────────────────────────────

DROP VIEW audit_record;

ALTER TABLE access_log_record
    ALTER COLUMN ehr_id TYPE uuid USING audit.access_log_canonical_uuid(ehr_id);

COMMENT ON COLUMN access_log_record.ehr_id IS
    'The entry''s ehr_id when it is a UUID in its lower-case hyphenated form, which the row part then leaves out; NULL on a tombstone and for any other id, which the row part keeps.';

-- ── the index columns of the record ─────────────────────────────────────────

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
    ADD COLUMN render_policy_id  integer;

ALTER TABLE access_log_record
    ALTER COLUMN event_action SET STORAGE MAIN,
    ALTER COLUMN principal SET STORAGE MAIN,
    ALTER COLUMN patient_id SET STORAGE MAIN,
    ALTER COLUMN resource_id SET STORAGE MAIN,
    ALTER COLUMN request_id SET STORAGE MAIN,
    ALTER COLUMN client_ip SET STORAGE MAIN,
    ALTER COLUMN classified_ehr_id SET STORAGE MAIN,
    ALTER COLUMN search_tokens SET STORAGE MAIN;

ALTER TABLE audit_render_policy
    ADD COLUMN id integer NOT NULL GENERATED ALWAYS AS IDENTITY,
    ADD CONSTRAINT uq_audit_render_policy_id UNIQUE (id);

COMMENT ON COLUMN audit_render_policy.id IS
    'The number an access-log entry names its render policy by, in place of the digest.';

ALTER TABLE access_log_record DISABLE TRIGGER access_log_record_tombstone_only;
ALTER TABLE access_log_record DISABLE TRIGGER access_log_record_unindex;

UPDATE access_log_record r
   SET event_action = s.action, event_outcome = s.outcome, principal = s.principal,
       patient_id = s.patient_id, resource_id = s.resource_id, request_id = s.request_id,
       client_ip = s.client_ip, classified_ehr_id = s.ehr_id, search_tokens = s.search_tokens,
       render_policy_id = p.id
  FROM access_log_search s
  JOIN audit_render_policy p ON p.digest = s.render_policy
 WHERE s.chain_seq = r.chain_seq;

ALTER TABLE access_log_record ENABLE TRIGGER access_log_record_tombstone_only;
ALTER TABLE access_log_record ENABLE TRIGGER access_log_record_unindex;

DROP TABLE access_log_search;

ALTER TABLE access_log_record
    ADD CONSTRAINT ck_access_log_record_index
        CHECK ((render_policy_id IS NULL) = (event_action IS NULL)
               AND (render_policy_id IS NULL) = (event_outcome IS NULL)
               AND (render_policy_id IS NULL) = (search_tokens IS NULL)
               AND (render_policy_id IS NOT NULL
                    OR num_nonnulls(principal, patient_id, resource_id, request_id, client_ip,
                                    classified_ehr_id) = 0)
               AND (render_policy_id IS NULL OR record_type = 'ACCESS_LOG_ENTRY')),
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
COMMENT ON COLUMN access_log_record.render_policy_id IS
    'The id of the render policy the entry''s projection is written under; NULL on a tombstone and on an entry its writer indexes no projection of, which the ITI-81 search does not find.';

CREATE INDEX ix_access_log_record_patient ON access_log_record (patient_id, time_recorded)
    WHERE patient_id IS NOT NULL;
CREATE INDEX ix_access_log_record_principal ON access_log_record (principal, time_recorded)
    WHERE principal IS NOT NULL;
CREATE INDEX ix_access_log_record_request_id ON access_log_record (hashtextextended(request_id, 0));
CREATE INDEX ix_access_log_record_resource_id ON access_log_record (hashtextextended(resource_id, 0));
CREATE INDEX ix_access_log_record_tokens ON access_log_record USING gin (search_tokens);

-- The time index carries whether an entry is indexed, so the ITI-81 total of
-- a date window is counted from the index alone.
DROP INDEX ix_access_log_record_time;
CREATE INDEX ix_access_log_record_time ON access_log_record (time_recorded)
    INCLUDE (render_policy_id);

-- ── the views ───────────────────────────────────────────────────────────────

CREATE VIEW access_log_search AS
    SELECT r.chain_seq, r.time_recorded AS recorded_at, r.event_action AS action,
           r.event_outcome AS outcome, r.principal, r.patient_id, r.resource_id,
           r.request_id, r.client_ip, r.classified_ehr_id AS ehr_id, r.search_tokens
      FROM access_log_record r
     WHERE r.render_policy_id IS NOT NULL;

COMMENT ON VIEW access_log_search IS
    'The ITI-81 index of the access-log entries: what the search filters on, read from the entry''s row. A tombstone is not in it.';

-- As 0039, with the access log's index read from the record and an entry
-- read as its whole canonical form.
CREATE VIEW audit_record AS
    SELECT e.chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.operation,
           e.principal, e.patient_id, e.resource_class, e.resource_id,
           e.client_ip, e.token_id, e.domain, e.purpose, e.legal_basis, e.result_count,
           e.request_id, e.roles, e.origins, e.origin_count, e.category_map_digest,
           e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id,
           e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id,
           e.subject_namespace, e.subject_lookup_failed, e.category_content,
           e.render_policy, e.token_attributes, e.search_tokens,
           NULL::text AS document, NULL::text AS algorithm,
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
           p.digest, NULL::jsonb, r.search_tokens,
           audit.access_log_document(r.content, sh.content, r.record_type, r.uid,
                                     r.time_recorded, r.ehr_id)::text,
           r.algorithm, encode(r.previous_digest, 'hex'),
           encode(r.content_digest, 'hex'), encode(r.digest, 'hex'),
           'access_log'::text
      FROM access_log_record r
      JOIN audit_render_policy p ON p.id = r.render_policy_id
      LEFT JOIN access_log_shared sh ON sh.digest = r.shared;

COMMENT ON VIEW audit_record IS
    'Every record the ITI-81 search, the person''s own trail and the FHIR Feed read: an audit_event record of the system chain by its columns, an access-log entry by its canonical form and its link, which its projection is rendered from. A record is named by its chain and position.';

-- ── the move into the chain ─────────────────────────────────────────────────

-- As 0042, with the index columns written on the record's own row, the
-- render policy by its id and the EHR id as a uuid.
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
            search_tokens, render_policy_id)
    SELECT l.seq, i.record_type, i.uid, i.time_recorded, i.content, 'SHA-256',
           decode(l.previous, 'hex'), i.content_digest, decode(l.digest, 'hex'),
           audit.access_log_canonical_uuid(i.ehr_id),
           i.party_id, i.retain_until, i.action_terminology, i.action_code,
           i.result_terminology, i.result_code, i.shared, i.action, i.outcome, i.principal,
           i.patient_id, i.resource_id, i.request_id, i.client_ip, i.index_ehr_id,
           i.search_tokens, p.id
      FROM unnest(p_ids, p_seqs, p_previous, p_digests) AS l(id, seq, previous, digest)
      JOIN audit.access_log_intake i ON i.id = l.id AND i.reserved_by IS NULL
      LEFT JOIN audit.audit_render_policy p ON p.digest = i.render_policy
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
          IS DISTINCT FROM audit.access_log_document(OLD.content, common, OLD.record_type, OLD.uid,
                                                     OLD.time_recorded, OLD.ehr_id) ->> 'system_id'
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
    NEW.render_policy_id := NULL;
    RETURN NEW;
END;
$$;

-- As 0045, with no index row to remove.
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

-- ── the columns an entry decides ─────────────────────────────────────────────

-- As 0039, with the record read through audit.access_log_document and the
-- EHR id as the uuid its column keeps.
CREATE OR REPLACE FUNCTION audit.access_log_record_derive(p_row audit.access_log_record)
    RETURNS audit.access_log_record
    LANGUAGE plpgsql STABLE
    SET search_path = pg_catalog
    SET TimeZone = 'UTC'
    AS $$
DECLARE
    common text;
    doc    jsonb;
BEGIN
    IF p_row.shared IS NOT NULL THEN
        SELECT s.content INTO common FROM audit.access_log_shared s WHERE s.digest = p_row.shared;
    END IF;
    IF jsonb_typeof(p_row.content::jsonb) IS DISTINCT FROM 'object' THEN
        RAISE EXCEPTION 'an access-log record is stored as its canonical form without its link'
            USING ERRCODE = 'check_violation';
    END IF;
    doc := audit.access_log_document(p_row.content, common, p_row.record_type, p_row.uid,
                                     p_row.time_recorded, p_row.ehr_id);
    IF doc ? 'link' THEN
        RAISE EXCEPTION 'an access-log record is stored as its canonical form without its link'
            USING ERRCODE = 'check_violation';
    END IF;
    p_row.record_type := doc ->> '_type';
    p_row.uid := (doc #>> '{uid,value}')::uuid;
    p_row.time_recorded := (doc #>> '{time_recorded,value}')::timestamptz;
    p_row.ehr_id := audit.access_log_canonical_uuid(doc #>> '{ehr_id,value}');
    p_row.action_terminology := doc #>> '{action,defining_code,terminology_id,value}';
    p_row.action_code := doc #>> '{action,defining_code,code_string}';
    p_row.result_terminology := doc #>> '{outcome,result,defining_code,terminology_id,value}';
    p_row.result_code := doc #>> '{outcome,result,defining_code,code_string}';
    p_row.party_id := doc #>> '{accessor,party,external_ref,id,value}';
    p_row.retain_until := (doc #>> '{retention,retain_until,value}')::date;
    p_row.time_removed := (doc #>> '{time_removed,value}')::timestamptz;
    p_row.fold_after := p_row.time_removed + (p_row.time_removed - p_row.time_recorded);
    RETURN p_row;
END;
$$;

-- As 0039, with the record read through audit.access_log_document.
CREATE OR REPLACE VIEW access_trail AS
    SELECT e.chain_seq, e.id, e.recorded_at, e.stored_at, e.action, e.outcome,
           e.event_code, e.operation, e.principal, e.patient_id,
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
           t.event_action,
           coalesce((audit.access_detail(t.doc, 'at0003') ->> 'magnitude')::smallint,
                    CASE WHEN t.doc #>> '{outcome,result,defining_code,code_string}' = 'granted'
                         THEN 0::smallint END),
           audit.access_event_code(audit.access_detail(t.doc, 'at0002') ->> 'value',
                                   t.event_action),
           coalesce(audit.access_detail(t.doc, 'at0029') ->> 'value',
                    t.doc #>> '{operation,defining_code,code_string}'),
           t.principal,
           audit.access_detail(t.doc, 'at0012') ->> 'id',
           audit.access_detail(t.doc, 'at0002') ->> 'value',
           coalesce(audit.access_detail(t.doc, 'at0014') ->> 'value',
                    CASE WHEN t.doc #>> '{items,0,rm_type}' IS DISTINCT FROM 'ACCESS_LOG_RECORD'
                         THEN coalesce(t.doc #>> '{items,0,version_id,value}',
                                       CASE WHEN t.doc #>> '{items,0,object,type}'
                                                 IS DISTINCT FROM 'ACCESS_LOG'
                                            THEN t.doc #>> '{items,0,object,id,value}' END)
                    END),
           t.doc #>> '{channel,client_address}',
           coalesce(audit.access_detail(t.doc, 'at0007') ->> 'value',
                    t.doc #>> '{accessor,credential,token_id}'),
           coalesce(audit.access_detail(t.doc, 'at0010') ->> 'value', 'clinical'),
           coalesce(audit.access_detail(t.doc, 'at0015') ->> 'value',
                    t.doc -> 'purposes' -> jsonb_array_length(coalesce(t.token -> 'purposes', '[]'::jsonb))
                        #>> '{defining_code,code_string}'),
           audit.access_detail(t.doc, 'at0016') ->> 'value',
           coalesce((audit.access_detail(t.doc, 'at0017') ->> 'magnitude')::bigint,
                    (t.doc #>> '{outcome,delivered}')::bigint,
                    (t.doc #>> '{query,rows}')::bigint),
           t.doc #>> '{channel,request_id}',
           CASE WHEN t.principal IS NULL AND t.roles = '[]'::jsonb THEN NULL ELSE t.roles END,
           audit.access_details(t.doc, 'at0018'),
           (audit.access_detail(t.doc, 'at0019') ->> 'magnitude')::bigint,
           NULL::jsonb, NULL::text, NULL::jsonb,
           audit.access_detail(t.doc, 'at0020') ->> 'value',
           audit.access_detail(t.doc, 'at0022') ->> 'value',
           coalesce(audit.access_detail(t.doc, 'at0023') ->> 'value',
                    t.doc #>> '{accessor,credential,assurance,defining_code,code_string}'),
           audit.access_detail(t.doc, 'at0024') ->> 'value',
           audit.access_detail(t.doc, 'at0025') ->> 'value',
           coalesce((t.doc ->> 'is_emergency')::boolean, false),
           (SELECT a #>> '{reference,id}'
              FROM jsonb_array_elements(coalesce(t.doc -> 'authorisations', '[]'::jsonb)) a
             WHERE a #>> '{kind,defining_code,code_string}' = 'permit' LIMIT 1),
           coalesce(audit.access_detail(t.doc, 'at0005') ->> 'value',
                    t.doc #>> '{accessor,credential,issuer}'),
           coalesce(audit.access_detail(t.doc, 'at0006') ->> 'value',
                    t.doc #>> '{accessor,credential,client_id}'),
           coalesce(audit.access_detail(t.doc, 'at0011') ->> 'value',
                    t.doc #>> '{ehr_id,value}'),
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
           t.token,
           t.uid::text, 'access_log'::text
      FROM (SELECT d.*,
                   coalesce(audit.access_detail(d.doc, 'at0001') ->> 'value',
                            CASE d.doc #>> '{action,defining_code,code_string}'
                                 WHEN 'C' THEN 'C' WHEN 'import' THEN 'C'
                                 WHEN 'U' THEN 'U' WHEN 'D' THEN 'D'
                                 WHEN 'R' THEN 'R' WHEN 'query' THEN 'R' WHEN 'export' THEN 'R'
                            END) AS event_action,
                   coalesce(audit.access_detail(d.doc, 'at0004') ->> 'value',
                            d.doc #>> '{accessor,credential,subject}',
                            (SELECT nullif(i ->> 'id', 'UNKNOWN')
                               FROM jsonb_array_elements(
                                        coalesce(d.doc #> '{accessor,party,identifiers}', '[]'::jsonb)) i
                              WHERE i ->> 'type' = 'login' LIMIT 1)) AS principal,
                   coalesce(audit.access_details(d.doc, 'at0009'),
                            (SELECT coalesce(jsonb_agg(role #> '{defining_code,code_string}'
                                                       ORDER BY n), '[]'::jsonb)
                               FROM jsonb_array_elements(
                                        coalesce(d.doc #> '{accessor,roles}', '[]'::jsonb))
                                    WITH ORDINALITY AS r(role, n)
                              WHERE n <= jsonb_array_length(coalesce(d.doc #> '{accessor,roles}', '[]'::jsonb))
                                         - jsonb_array_length(coalesce(d.token -> 'roles', '[]'::jsonb))))
                       AS roles
              FROM (SELECT r.chain_seq, r.uid, r.time_recorded, r.stored_at, x.doc,
                           (audit.access_detail(x.doc, 'at0027') ->> 'value')::jsonb AS binding,
                           (audit.access_detail(x.doc, 'at0026') ->> 'value')::jsonb AS token
                      FROM access_log_record r
                      LEFT JOIN access_log_shared sh ON sh.digest = r.shared
                     CROSS JOIN LATERAL
                           (SELECT audit.access_log_document(r.content, sh.content, r.record_type,
                                                     r.uid, r.time_recorded, r.ehr_id) AS doc) AS x
                     WHERE r.record_type = 'ACCESS_LOG_ENTRY') AS d) AS t;


DROP FUNCTION audit.access_log_document(text, text);

-- ── the intake keeps no second time ────────────────────────────────────────

-- The ITI-81 index reads an entry's time_recorded, so the intake row keeps no
-- recorded_at of its own.
DROP FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], timestamptz[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[], uuid, bytea[]);

ALTER TABLE access_log_intake
    DROP CONSTRAINT ck_access_log_intake_index,
    DROP COLUMN recorded_at,
    ADD CONSTRAINT ck_access_log_intake_index
        CHECK ((render_policy IS NULL) = (action IS NULL)
               AND (render_policy IS NULL) = (outcome IS NULL)
               AND (render_policy IS NULL) = (search_tokens IS NULL));

-- As 0045, without the recorded_at of each row.
CREATE FUNCTION audit.access_log_intake_take(
    p_record_types text[], p_uids text[], p_times text[], p_contents text[],
    p_shared text[], p_digests text[], p_ehr_ids text[], p_party_ids text[],
    p_retain_until text[], p_action_terminologies text[], p_action_codes text[],
    p_result_terminologies text[], p_result_codes text[],
    p_actions text[], p_outcomes smallint[], p_principals text[], p_patient_ids text[],
    p_resource_ids text[], p_request_ids text[], p_client_ips text[],
    p_index_ehr_ids text[], p_search_tokens text[], p_policies bytea[],
    p_shared_digests text[], p_shared_contents text[],
    p_token_entries integer[], p_token_parameters text[], p_token_systems text[],
    p_token_codes text[], p_token_shares boolean[], p_reserved_by uuid,
    p_token_digests bytea[])
    RETURNS void
    LANGUAGE plpgsql
    SET search_path = pg_catalog
    AS $$
DECLARE
    v_missing_tokens bigint;
    v_missing_sets bigint;
    v_entries integer[];
    v_shared text[];
    v_own text[];
BEGIN
    IF cardinality(p_token_entries) > 0 THEN
        PERFORM audit.hold_search_tokens();
        FOR v_attempt IN 1..4 LOOP
            WITH found AS MATERIALIZED (
                SELECT d.e, h.id, d.shares
                  FROM unnest(p_token_entries, p_token_digests, p_token_shares) AS d(e, digest, shares)
                  LEFT JOIN audit.audit_search_token h ON h.digest = d.digest
            ),
            grouped AS MATERIALIZED (
                SELECT f.e,
                       array_agg(f.id ORDER BY f.id) FILTER (WHERE f.shares) AS shared,
                       array_agg(f.id ORDER BY f.id) FILTER (WHERE NOT f.shares) AS own
                  FROM found f
                 GROUP BY f.e
            )
            SELECT (SELECT count(*) FROM found f WHERE f.id IS NULL),
                   (SELECT count(*) FROM grouped g
                      LEFT JOIN audit.audit_search_set x ON x.tokens = g.shared
                     WHERE g.shared IS NOT NULL AND x.id IS NULL),
                   ARRAY(SELECT g.e FROM grouped g ORDER BY g.e),
                   ARRAY(SELECT g.shared::text FROM grouped g ORDER BY g.e),
                   ARRAY(SELECT g.own::text FROM grouped g ORDER BY g.e)
              INTO v_missing_tokens, v_missing_sets, v_entries, v_shared, v_own;
            EXIT WHEN v_missing_tokens = 0 AND v_missing_sets = 0;
            IF v_attempt = 4 THEN
                RAISE EXCEPTION 'a search token or shared set of the intake could not be stored or read';
            END IF;
            IF v_missing_tokens > 0 THEN
                INSERT INTO audit.audit_search_token (parameter, system, code, digest)
                SELECT DISTINCT ON (t.digest) t.p, t.s, t.c, t.digest
                  FROM unnest(p_token_parameters, p_token_systems, p_token_codes, p_token_digests)
                       AS t(p, s, c, digest)
                 WHERE NOT EXISTS (SELECT FROM audit.audit_search_token h WHERE h.digest = t.digest)
                 ORDER BY t.digest
                ON CONFLICT (digest) DO NOTHING;
            ELSE
                INSERT INTO audit.audit_search_set (tokens, digest)
                SELECT s.tokens, audit.search_set_digest(s.tokens)
                  FROM (SELECT DISTINCT k.tokens::integer[] AS tokens
                          FROM unnest(v_shared) AS k(tokens)
                         WHERE k.tokens IS NOT NULL
                           AND NOT EXISTS (SELECT FROM audit.audit_search_set x
                                            WHERE x.tokens = k.tokens::integer[])) AS s
                 ORDER BY s.tokens
                ON CONFLICT (tokens) DO NOTHING;
            END IF;
        END LOOP;
    END IF;
    IF cardinality(p_shared_digests) > 0 THEN
        INSERT INTO audit.access_log_shared (digest, content)
        SELECT decode(t.digest, 'hex'), t.content
          FROM unnest(p_shared_digests, p_shared_contents) AS t(digest, content)
         ORDER BY decode(t.digest, 'hex')
        ON CONFLICT (digest) DO NOTHING;
    END IF;
    WITH held AS (
        SELECT s.digest
          FROM audit.access_log_shared s
         WHERE s.digest IN (SELECT DISTINCT decode(t.c, 'hex')
                              FROM unnest(p_shared) AS t(c)
                             WHERE t.c IS NOT NULL)
         ORDER BY s.digest
           FOR KEY SHARE
    )
    INSERT INTO audit.access_log_intake (record_type, uid, time_recorded, content, shared,
        content_digest, ehr_id, party_id, retain_until, action_terminology, action_code,
        result_terminology, result_code, action, outcome, principal, patient_id,
        resource_id, request_id, client_ip, index_ehr_id, search_tokens, render_policy,
        reserved_by)
    SELECT u.c1, u.c2::uuid, u.c3::timestamptz, u.c4,
           CASE
               WHEN u.c5 IS NOT NULL AND held.digest IS NULL
               THEN audit.access_log_intake_unknown('shared part', u.c5, batch.digest IS NOT NULL)
               ELSE decode(u.c5, 'hex')
           END,
           decode(u.c6, 'hex'),
           u.c7, u.c8, u.c9::date, u.c10, u.c11, u.c12, u.c13, u.c15, u.c16, u.c17, u.c18,
           u.c19, u.c20, u.c21, u.c22,
           CASE
               WHEN u.c23 IS NULL THEN NULL
               WHEN u.c23 <> '' THEN u.c23::integer[]
               WHEN x.id IS NULL THEN coalesce(g.own, '{}')::integer[]
               ELSE ARRAY[-x.id] || coalesce(g.own, '{}')::integer[]
           END,
           CASE
               WHEN u.c24 IS NOT NULL AND policy.digest IS NULL
               THEN audit.access_log_intake_unknown('render policy', encode(u.c24, 'hex'), false)
               ELSE u.c24
           END,
           p_reserved_by
      FROM unnest(p_record_types, p_uids, p_times, p_contents, p_shared, p_digests, p_ehr_ids,
                  p_party_ids, p_retain_until, p_action_terminologies, p_action_codes,
                  p_result_terminologies, p_result_codes, p_actions, p_outcomes,
                  p_principals, p_patient_ids, p_resource_ids, p_request_ids, p_client_ips,
                  p_index_ehr_ids, p_search_tokens, p_policies) WITH ORDINALITY
           AS u(c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c15, c16, c17, c18,
                c19, c20, c21, c22, c23, c24, n)
      LEFT JOIN unnest(v_entries, v_shared, v_own) AS g(e, shared, own) ON g.e = u.n
      LEFT JOIN audit.audit_search_set x ON x.tokens = g.shared::integer[]
      LEFT JOIN held ON held.digest = decode(u.c5, 'hex')
      LEFT JOIN unnest(p_shared_digests) AS batch(digest) ON batch.digest = u.c5
      LEFT JOIN audit.audit_render_policy policy ON policy.digest = u.c24
     ORDER BY u.n;
END
$$;

COMMENT ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[], uuid, bytea[]) IS
    'One batch''s access-log entries into the intake: their search tokens, found by the digests the server computed, grouped by entry in one pass, their shared sets and shared parts stored or found and checked, and their rows (#5770, #5887, #5889).';

REVOKE ALL ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[], uuid, bytea[]) FROM PUBLIC;

-- ── grants ──────────────────────────────────────────────────────────────────

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT ON access_log_search, audit_record
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[], uuid, bytea[])
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the access-log view grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
