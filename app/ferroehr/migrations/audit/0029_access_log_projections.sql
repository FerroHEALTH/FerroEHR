-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the access-log entry is the record of every access it covers, and
-- the ITI-81 search and the FHIR Feed serve its projections (#5127).
--
-- An access to EHR data is stored as its ACCESS_LOG_ENTRY alone; the IHE
-- BALP AuditEvent, the DICOM audit message and the EHDS access record are
-- rendered from the entry when they are read or sent. access_log_search is
-- the ITI-81 index of the entries: the columns the search filters on and the
-- render policy the entry's projection is written under, one row per entry,
-- written in the transaction that appends it. It is an index, never a
-- record: the document a search serves is rendered from the entry's content,
-- and the row goes when the entry is replaced by its tombstone.
--
-- audit_record is the one relation the ITI-81 search, the person's own trail
-- and the FHIR Feed read: the audit_event records, which keep the accesses
-- outside the access log (an authentication, a template or a party
-- operation), and the entries through their index.
--
-- fhir_feed_outbox names records of either form, so its key no longer
-- references audit_event; a reaped audit_event record and an entry replaced
-- by its tombstone leave it by trigger.
--
-- No openEHR spec governs audit storage: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

-- ── the index of the entries ─────────────────────────────────────────────────

CREATE TABLE access_log_search (
    chain_seq     bigint      NOT NULL,
    recorded_at   timestamptz NOT NULL,
    action        text        NOT NULL,
    outcome       smallint    NOT NULL,
    principal     text,
    patient_id    text,
    resource_id   text,
    request_id    text,
    client_ip     text,
    ehr_id        text,
    search_tokens integer[]   NOT NULL,
    render_policy bytea       NOT NULL,
    CONSTRAINT pk_access_log_search PRIMARY KEY (chain_seq),
    CONSTRAINT fk_access_log_search_record FOREIGN KEY (chain_seq)
        REFERENCES access_log_record (chain_seq),
    CONSTRAINT fk_access_log_search_policy FOREIGN KEY (render_policy)
        REFERENCES audit_render_policy (digest),
    CONSTRAINT ck_access_log_search_action CHECK (action IN ('C', 'R', 'U', 'D', 'E')),
    CONSTRAINT ck_access_log_search_outcome CHECK (outcome IN (0, 4, 8))
);

COMMENT ON TABLE access_log_search IS
    'The ITI-81 index of the access-log entries: what the search filters on and the render policy an entry''s projection is written under. An index, never a record: removed when its entry is replaced by its tombstone.';
COMMENT ON COLUMN access_log_search.action IS
    'The DICOM EventActionCode (PS3.15 A.5.1) the projection writes.';
COMMENT ON COLUMN access_log_search.outcome IS
    'The DICOM EventOutcomeIndicator (PS3.15 A.5.1) the projection writes.';
COMMENT ON COLUMN access_log_search.patient_id IS
    'The subject of care of the entry''s EHR, as the entry''s access details name it.';
COMMENT ON COLUMN access_log_search.ehr_id IS
    'The EHR of a classified access, which its EHDS access record names; NULL for every other entry.';
COMMENT ON COLUMN access_log_search.search_tokens IS
    'The ids of the search tokens of the entry''s projection, or of their shared set.';

CREATE INDEX ix_access_log_search_recorded_at ON access_log_search (recorded_at);
CREATE INDEX ix_access_log_search_patient ON access_log_search (patient_id, recorded_at)
    WHERE patient_id IS NOT NULL;
CREATE INDEX ix_access_log_search_principal ON access_log_search (principal, recorded_at)
    WHERE principal IS NOT NULL;
CREATE INDEX ix_access_log_search_resource_id
    ON access_log_search (hashtextextended(resource_id, 0));
CREATE INDEX ix_access_log_search_request_id
    ON access_log_search (hashtextextended(request_id, 0));
CREATE INDEX ix_access_log_search_tokens ON access_log_search USING gin (search_tokens);

-- ── the relation the search reads ────────────────────────────────────────────

CREATE VIEW audit_record AS
    SELECT e.chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.operation,
           e.principal, e.organisation, e.patient_id, e.resource_class, e.resource_id,
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
           s.principal, NULL::text, s.patient_id, NULL::text, s.resource_id,
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

-- The value of the detail at-code p_code of an entry's FerroEHR access
-- details, its first element when it holds several.
CREATE FUNCTION audit.access_detail(p_doc jsonb, p_code text) RETURNS jsonb
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT jsonb_path_query_first(p_doc,
        '$.other_details.items[*] ? (@.archetype_node_id == $code).value',
        jsonb_build_object('code', p_code))
$$;

COMMENT ON FUNCTION audit.access_detail(jsonb, text) IS
    'The value of one of FerroEHR''s access details (an ELEMENT of the entry''s other_details, named by its at-code); NULL when the entry holds none.';

-- The texts of every element of the detail p_code, as a JSON array; NULL
-- when the entry holds none.
CREATE FUNCTION audit.access_details(p_doc jsonb, p_code text) RETURNS jsonb
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT nullif(jsonb_path_query_array(p_doc,
        '$.other_details.items[*] ? (@.archetype_node_id == $code).value.value',
        jsonb_build_object('code', p_code)), '[]'::jsonb)
$$;

COMMENT ON FUNCTION audit.access_details(jsonb, text) IS
    'The texts of every element of one of FerroEHR''s access details, as a JSON array; NULL when the entry holds none.';

-- The DICOM EventID (PS3.15 A.5.1) of a resource class and action, as the
-- audit_event records state it.
CREATE FUNCTION audit.access_event_code(p_class text, p_action text) RETURNS text
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE
        WHEN p_class = 'query' THEN '110112'
        WHEN p_class IN ('template', 'application_activity') THEN '110100'
        WHEN p_class = 'authentication' THEN '110114'
        WHEN p_class = 'extract' AND p_action IN ('C', 'U') THEN '110107'
        WHEN p_class = 'extract' THEN '110106'
        ELSE '110110'
    END
$$;

-- Every record of the trail with the facts it states, in the audit
-- vocabulary: an audit_event record by its columns, an entry by its content
-- (the core for its time and request, its access details for the rest). The
-- categories a classification computes are no column of an entry: its
-- projection writes them.
CREATE VIEW access_trail AS
    SELECT e.chain_seq, e.id, e.recorded_at, e.stored_at, e.action, e.outcome,
           e.event_code, e.operation, e.principal, e.organisation, e.patient_id,
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
           audit.access_detail(t.doc, 'at0008') ->> 'value',
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

-- ── the outbox names either form ─────────────────────────────────────────────

ALTER TABLE fhir_feed_outbox DROP CONSTRAINT fk_fhir_feed_outbox_record;

CREATE FUNCTION audit.audit_event_leave_outbox() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    DELETE FROM audit.fhir_feed_outbox o WHERE o.chain_seq = OLD.chain_seq;
    RETURN OLD;
END;
$$;

COMMENT ON FUNCTION audit.audit_event_leave_outbox() IS
    'Removes a reaped audit_event record from the FHIR Feed''s outbox, as the foreign key that cascaded did.';

CREATE TRIGGER audit_event_leave_outbox
    AFTER DELETE ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit.audit_event_leave_outbox();

-- An entry replaced by its tombstone keeps no index row and is owed to no
-- sink: its projection is gone with its content.
CREATE FUNCTION audit.access_log_record_unindex() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF NEW.record_type = 'ACCESS_LOG_TOMBSTONE' THEN
        DELETE FROM audit.access_log_search s WHERE s.chain_seq = NEW.chain_seq;
        DELETE FROM audit.fhir_feed_outbox o WHERE o.chain_seq = NEW.chain_seq;
    END IF;
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION audit.access_log_record_unindex() IS
    'Removes the index row and the FHIR Feed debt of an entry replaced by its tombstone.';

CREATE TRIGGER access_log_record_unindex
    AFTER UPDATE ON access_log_record
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_record_unindex();

-- ── grants ───────────────────────────────────────────────────────────────────

REVOKE ALL ON FUNCTION audit.audit_event_leave_outbox() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_record_unindex() FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON access_log_search, audit_record, access_trail
            FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT, INSERT ON access_log_search TO ferroehr_clinical;
        GRANT SELECT ON access_log_search TO ferroehr_clinical_reader;
        GRANT SELECT ON audit_record, access_trail
            TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping access-log search grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
