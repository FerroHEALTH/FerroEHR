-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: an access record names no organisation of its own (#5586).
--
-- audit_event.organisation held the organisation a token claim named, which
-- the removed ABAC engine read from a configured claim; nothing has written
-- it since. The party binding's provider is the healthcare provider EHDS
-- (Regulation (EU) 2025/327) Annex II 3.2(a) asks the log for
-- (docs/law/eu/ehds/text.html), so the column, its place in the two views
-- that read it and its place in the chain digest go.
--
-- The chain digest of a stored row reads each listed column from the row's
-- jsonb form, so a dropped column reads as null, as a row that held no
-- organisation already hashed it: versions 1 to 3 keep their lists, and a new
-- row is digest version 4, hashed over version 3's list without organisation.
--
-- No openEHR spec governs audit storage: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

DROP VIEW audit_record;
DROP VIEW access_trail;

ALTER TABLE audit_event DROP COLUMN organisation;

ALTER TABLE audit_event ALTER COLUMN digest_version SET DEFAULT 4;

ALTER TABLE audit_event
    DROP CONSTRAINT ck_audit_event_digest_version,
    ADD CONSTRAINT ck_audit_event_digest_version CHECK (digest_version IN (1, 2, 3, 4));

CREATE OR REPLACE FUNCTION audit.audit_event_digest_columns(p_version smallint) RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE p_version
        WHEN 1 THEN v1.columns
        WHEN 2 THEN v1.columns || v2.columns
        WHEN 3 THEN v1.columns || v2.columns || ARRAY['token_attributes']
        WHEN 4 THEN array_remove(v1.columns || v2.columns || ARRAY['token_attributes'],
                                 'organisation')
    END
      FROM (SELECT ARRAY[
            'id', 'recorded_at', 'stored_at', 'action', 'outcome', 'event_code',
            'operation', 'principal', 'patient_id', 'resource_class',
            'resource_id', 'client_ip', 'token_id',
            'chain_seq', 'prev_hash',
            'domain', 'purpose', 'legal_basis', 'result_count', 'request_id',
            'organisation', 'roles', 'origins', 'origin_count',
            'categories', 'category_basis', 'category_evidence',
            'category_map_digest',
            'acting_mode', 'assurance_level', 'assurance_value',
            'professional_id',
            'emergency_access',
            'retention_ends', 'retention_stated',
            'permit',
            'issuer', 'client_id', 'ehr_id', 'subject_namespace',
            'subject_lookup_failed', 'category_content', 'render_policy',
            'search_tokens'
        ]) AS v1(columns),
           (SELECT ARRAY[
            'binding', 'party', 'party_groups', 'provider', 'guest', 'representation',
            'external_fields', 'agreement', 'actor_chain', 'facilities', 'binding_detail',
            'summary', 'digest_version'
        ]) AS v2(columns)
$$;

-- ── the relation the search reads ────────────────────────────────────────────

CREATE VIEW audit_record AS
    SELECT e.chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.operation,
           e.principal, e.patient_id, e.resource_class, e.resource_id,
           e.client_ip, e.token_id, e.domain, e.purpose, e.legal_basis, e.result_count,
           e.request_id, e.roles, e.origins, e.origin_count, e.category_map_digest,
           e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id,
           e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id,
           e.subject_namespace, e.subject_lookup_failed, e.category_content,
           e.render_policy, e.token_attributes, e.search_tokens,
           NULL::text AS content, NULL::text AS algorithm, NULL::text AS previous_digest,
           NULL::text AS content_digest, NULL::text AS digest, NULL::text AS signature
      FROM audit_event e
    UNION ALL
    SELECT s.chain_seq, s.recorded_at, r.stored_at, s.action, s.outcome, NULL::text,
           s.principal, s.patient_id, NULL::text, s.resource_id,
           s.client_ip, NULL::text, NULL::text, NULL::text, NULL::text, NULL::bigint,
           s.request_id, NULL::jsonb, NULL::jsonb, NULL::bigint, NULL::text,
           NULL::text, NULL::text, NULL::text, NULL::text,
           false, NULL::text, NULL::text, NULL::text, s.ehr_id,
           NULL::text, false, NULL::jsonb,
           s.render_policy, NULL::jsonb, s.search_tokens,
           r.content, r.algorithm, r.previous_digest, r.content_digest, r.digest, r.signature
      FROM access_log_search s
      JOIN access_log_record r ON r.chain_seq = s.chain_seq;

COMMENT ON VIEW audit_record IS
    'Every record the ITI-81 search, the person''s own trail and the FHIR Feed read: an audit_event record by its columns, an access-log entry by its content and link, which its projection is rendered from.';

-- ── the access trail in SQL ──────────────────────────────────────────────────

CREATE VIEW access_trail AS
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
           NULL::text AS entry_uid
      FROM audit_event e
    UNION ALL
    SELECT t.chain_seq, NULL::uuid, t.time_recorded, t.stored_at,
           audit.access_detail(t.doc, 'at0001') ->> 'value',
           (audit.access_detail(t.doc, 'at0003') ->> 'magnitude')::smallint,
           audit.access_event_code(audit.access_detail(t.doc, 'at0002') ->> 'value',
                                   audit.access_detail(t.doc, 'at0001') ->> 'value'),
           coalesce(audit.access_detail(t.doc, 'at0029') ->> 'value',
                    t.doc #>> '{operation,defining_code,code_string}'),
           audit.access_detail(t.doc, 'at0004') ->> 'value',
           audit.access_detail(t.doc, 'at0012') ->> 'id',
           audit.access_detail(t.doc, 'at0002') ->> 'value',
           audit.access_detail(t.doc, 'at0014') ->> 'value',
           t.doc #>> '{channel,client_address}',
           audit.access_detail(t.doc, 'at0007') ->> 'value',
           coalesce(audit.access_detail(t.doc, 'at0010') ->> 'value', 'clinical'),
           audit.access_detail(t.doc, 'at0015') ->> 'value',
           audit.access_detail(t.doc, 'at0016') ->> 'value',
           (audit.access_detail(t.doc, 'at0017') ->> 'magnitude')::bigint,
           t.doc #>> '{channel,request_id}',
           CASE WHEN audit.access_detail(t.doc, 'at0004') IS NULL
                     AND audit.access_details(t.doc, 'at0009') IS NULL THEN NULL
                ELSE coalesce(audit.access_details(t.doc, 'at0009'), '[]'::jsonb) END,
           audit.access_details(t.doc, 'at0018'),
           (audit.access_detail(t.doc, 'at0019') ->> 'magnitude')::bigint,
           NULL::jsonb, NULL::text, NULL::jsonb,
           audit.access_detail(t.doc, 'at0020') ->> 'value',
           audit.access_detail(t.doc, 'at0022') ->> 'value',
           audit.access_detail(t.doc, 'at0023') ->> 'value',
           audit.access_detail(t.doc, 'at0024') ->> 'value',
           audit.access_detail(t.doc, 'at0025') ->> 'value',
           coalesce((t.doc ->> 'is_emergency')::boolean, false),
           (SELECT a #>> '{reference,id}'
              FROM jsonb_array_elements(coalesce(t.doc -> 'authorisations', '[]'::jsonb)) a
             WHERE a #>> '{kind,defining_code,code_string}' = 'permit' LIMIT 1),
           audit.access_detail(t.doc, 'at0005') ->> 'value',
           audit.access_detail(t.doc, 'at0006') ->> 'value',
           audit.access_detail(t.doc, 'at0011') ->> 'value',
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
           (audit.access_detail(t.doc, 'at0026') ->> 'value')::jsonb,
           t.uid
      FROM (SELECT r.chain_seq, r.uid, r.time_recorded, r.stored_at, r.content::jsonb AS doc,
                   (audit.access_detail(r.content::jsonb, 'at0027') ->> 'value')::jsonb AS binding
              FROM access_log_record r
             WHERE r.record_type = 'ACCESS_LOG_ENTRY') t;

COMMENT ON VIEW access_trail IS
    'The access trail in SQL: every audit_event record and every access-log entry with the facts it states, in the audit vocabulary. An entry''s categories are computed by its projection and are no column here.';

-- ── grants ───────────────────────────────────────────────────────────────────

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON audit_record, access_trail
            FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON audit_record, access_trail
            TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping access-log view grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
