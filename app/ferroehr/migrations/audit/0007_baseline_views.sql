-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the views the ITI-81 search and the person's access trail read.
--
-- Runs with search_path = audit, ext, public.

-- The value of the detail id-code p_code of an entry's access log details
-- (openEHR-EHR-ITEM_TREE.access_log_details.v1), its first element when it
-- holds several.
CREATE FUNCTION audit.access_detail(p_doc jsonb, p_code text) RETURNS jsonb
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT jsonb_path_query_first(p_doc,
        '$.other_details.items[*] ? (@.archetype_node_id == $code).value',
        jsonb_build_object('code', p_code))
$$;
COMMENT ON FUNCTION access_detail(jsonb, text) IS
    'The value of one of the access log details (an ELEMENT of the entry''s other_details, named by its id-code); NULL when the entry holds none.';

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
COMMENT ON FUNCTION access_details(jsonb, text) IS
    'The texts of every element of one of the access log details, as a JSON array; NULL when the entry holds none.';

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

-- The entry id joins the index of the entries (#6000). A page and the total
-- read the entries no write holds, each named by its id (#6242): the indexed
-- entries through access_log_index, whose columns the search's conditions and
-- order reach, and the entries past its cursor by key (#6319). The join to the
-- row is a left join on its key, so a total that names the index's columns
-- alone reads no row.
CREATE VIEW access_log_search AS
    SELECT i.entry_id AS chain_seq, i.time_recorded AS recorded_at, e.event_action AS action,
           e.event_outcome AS outcome, i.principal, i.patient_id, e.resource_id,
           e.request_id, e.client_ip, e.classified_ehr_id AS ehr_id, e.search_tokens,
           i.search_keys, i.entry_id
      FROM access_log_index i
      LEFT JOIN access_log_entry e ON e.id = i.entry_id
     WHERE i.render_policy_id IS NOT NULL
    UNION ALL
    SELECT e.id AS chain_seq, e.time_recorded AS recorded_at, e.event_action AS action,
           e.event_outcome AS outcome, e.principal, e.patient_id, e.resource_id,
           e.request_id, e.client_ip, e.classified_ehr_id AS ehr_id, e.search_tokens,
           audit.search_keys(e.search_tokens, e.request_id, e.resource_id) AS search_keys,
           e.id AS entry_id
      FROM access_log_entry e
     WHERE e.render_policy_id IS NOT NULL
       AND e.id > (SELECT x.last_id FROM access_log_index_state x WHERE x.singleton)
       AND NOT EXISTS (SELECT 1 FROM access_log_hold h WHERE h.entry_id = e.id);
COMMENT ON VIEW access_log_search IS
    'The ITI-81 index of the access-log entries no write holds: what the search filters on, read through access_log_index for the indexed entries and from the entry''s row past its cursor, and the entry''s id, which names it. A tombstone is not in it.';

-- The ITI-81 index of the system records no write holds (#6203, #6242): what
-- the search filters on, read from the records, and each record's position.
-- A plain scan, so a page reads it in the search's order through
-- ix_audit_event_recorded_at, where the system member of audit_record's union
-- is planned without that order.
CREATE VIEW system_search AS
    SELECT e.seq AS chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.principal,
           e.patient_id, e.resource_id, e.request_id, e.client_ip, e.ehr_id, e.search_tokens,
           audit.search_keys(e.search_tokens, e.request_id, e.resource_id) AS search_keys,
           'system'::text AS chain, e.id AS event_id
      FROM audit_event e
     WHERE NOT EXISTS (SELECT 1 FROM audit_event_hold h WHERE h.seq = e.seq);
COMMENT ON VIEW system_search IS
    'The ITI-81 index of the system records no write holds: what the search filters on, read from each audit_event record, its position and its id.';

CREATE VIEW audit_record AS
    SELECT e.seq AS chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.operation,
           e.principal, e.patient_id, e.resource_class, e.resource_id,
           e.client_ip, e.token_id, e.domain, e.purpose, e.legal_basis, e.result_count,
           e.request_id, e.roles, e.origins, e.origin_count, e.category_map_digest,
           e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id,
           e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id,
           e.subject_namespace, e.subject_lookup_failed, e.review, e.category_content,
           e.render_policy, e.token_attributes, e.search_tokens,
           audit.search_keys(e.search_tokens, e.request_id, e.resource_id) AS search_keys,
           NULL::text AS row_part, NULL::text AS shared_part, NULL::text AS entry_type,
           NULL::uuid AS entry_uid, NULL::uuid AS entry_ehr,
           NULL::text AS algorithm, NULL::text AS previous_digest,
           NULL::text AS content_digest, NULL::text AS digest, NULL::bytea AS shared_digest,
           'system'::text AS chain
      FROM audit_event e
     WHERE NOT EXISTS (SELECT 1 FROM audit_event_hold h WHERE h.seq = e.seq)
    UNION ALL
    SELECT r.chain_seq, r.time_recorded, r.time_recorded, r.event_action, r.event_outcome, NULL::text,
           r.principal, r.patient_id, NULL::text, r.resource_id,
           r.client_ip, NULL::text, NULL::text, NULL::text, NULL::text, NULL::bigint,
           r.request_id, NULL::jsonb, NULL::jsonb, NULL::bigint, NULL::text,
           NULL::text, NULL::text, NULL::text, NULL::text,
           false, NULL::text, NULL::text, NULL::text, r.classified_ehr_id,
           NULL::text, false, NULL::text, NULL::jsonb,
           p.digest, NULL::jsonb, r.search_tokens,
           audit.search_keys(r.search_tokens, r.request_id, r.resource_id),
           r.content, sh.content, r.record_type, r.uid, r.ehr_id,
           NULL::text, NULL::text, encode(r.content_digest, 'hex'), NULL::text, r.shared,
           'access_log'::text
      FROM access_log_record r
      JOIN audit_render_policy p ON p.id = r.render_policy_id
      LEFT JOIN access_log_shared sh ON sh.digest = r.shared;
COMMENT ON VIEW audit_record IS
    'Every record the ITI-81 search, the person''s own trail and the FHIR Feed read: an audit_event record no write holds by its columns and its position, an access-log entry by its stored parts, the digest of its shared part (which a page reads once for every entry naming it), the columns its row part leaves out (recorded_at is its time_recorded) and its content digest, from which the server renders it. A record is named by its chain and position.';

-- The person's access trail, with the index columns the row part leaves
-- members to.
CREATE VIEW access_trail AS
    SELECT e.seq AS chain_seq, e.id, e.recorded_at, e.stored_at, e.action, e.outcome,
           e.event_code, e.operation, e.principal, e.patient_id,
           e.resource_class, e.resource_id, e.client_ip, e.token_id, e.domain, e.purpose,
           e.legal_basis, e.result_count, e.request_id, e.roles, e.origins, e.origin_count,
           e.categories, e.category_basis, e.category_evidence, e.category_map_digest,
           e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id,
           e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id,
           e.subject_namespace, e.subject_lookup_failed, e.review, e.category_content,
           e.binding, e.guest, e.party, e.party_groups, e.provider, e.representation,
           e.external_fields, e.agreement, e.actor_chain, e.facilities, e.binding_detail,
           e.summary, e.token_attributes,
           NULL::text AS entry_uid, 'system'::text AS chain
      FROM audit_event e
     WHERE NOT EXISTS (SELECT 1 FROM audit_event_hold h WHERE h.seq = e.seq)
    UNION ALL
    SELECT t.chain_seq, NULL::uuid, t.time_recorded, t.time_recorded,
           t.event_action,
           coalesce((audit.access_detail(t.doc, 'id13') ->> 'magnitude')::smallint,
                    CASE WHEN t.doc #>> '{outcome,result,defining_code,code_string}' = 'granted'
                         THEN 0::smallint END),
           audit.access_event_code(audit.access_detail(t.doc, 'id12') ->> 'value',
                                   t.event_action),
           coalesce(audit.access_detail(t.doc, 'id39') ->> 'value',
                    t.doc #>> '{operation,defining_code,code_string}'),
           t.principal,
           audit.access_detail(t.doc, 'id22') ->> 'id',
           audit.access_detail(t.doc, 'id12') ->> 'value',
           coalesce(audit.access_detail(t.doc, 'id24') ->> 'value',
                    CASE WHEN t.doc #>> '{items,0,rm_type}' IS DISTINCT FROM 'ACCESS_LOG_RECORD'
                         THEN coalesce(t.doc #>> '{items,0,version_id,value}',
                                       CASE WHEN t.doc #>> '{items,0,object,type}'
                                                 IS DISTINCT FROM 'ACCESS_LOG'
                                            THEN t.doc #>> '{items,0,object,id,value}' END)
                    END),
           t.doc #>> '{channel,client_address}',
           coalesce(audit.access_detail(t.doc, 'id17') ->> 'value',
                    t.doc #>> '{accessor,credential,token_id}'),
           coalesce(audit.access_detail(t.doc, 'id20') ->> 'value', 'clinical'),
           coalesce(audit.access_detail(t.doc, 'id25') ->> 'value',
                    t.doc -> 'purposes' -> jsonb_array_length(coalesce(t.token -> 'purposes', '[]'::jsonb))
                        #>> '{defining_code,code_string}'),
           audit.access_detail(t.doc, 'id26') ->> 'value',
           coalesce((audit.access_detail(t.doc, 'id27') ->> 'magnitude')::bigint,
                    (t.doc #>> '{outcome,delivered}')::bigint,
                    (t.doc #>> '{query,rows}')::bigint),
           t.doc #>> '{channel,request_id}',
           CASE WHEN t.principal IS NULL AND t.roles = '[]'::jsonb THEN NULL ELSE t.roles END,
           audit.access_details(t.doc, 'id28'),
           (audit.access_detail(t.doc, 'id29') ->> 'magnitude')::bigint,
           NULL::jsonb, NULL::text, NULL::jsonb,
           audit.access_detail(t.doc, 'id30') ->> 'value',
           audit.access_detail(t.doc, 'id32') ->> 'value',
           coalesce(audit.access_detail(t.doc, 'id33') ->> 'value',
                    t.doc #>> '{accessor,credential,assurance,defining_code,code_string}'),
           audit.access_detail(t.doc, 'id34') ->> 'value',
           audit.access_detail(t.doc, 'id35') ->> 'value',
           coalesce((t.doc ->> 'is_emergency')::boolean, false),
           (SELECT a #>> '{reference,id}'
              FROM jsonb_array_elements(coalesce(t.doc -> 'authorisations', '[]'::jsonb)) a
             WHERE a #>> '{kind,defining_code,code_string}' = 'permit' LIMIT 1),
           coalesce(audit.access_detail(t.doc, 'id15') ->> 'value',
                    t.doc #>> '{accessor,credential,issuer}'),
           coalesce(audit.access_detail(t.doc, 'id16') ->> 'value',
                    t.doc #>> '{accessor,credential,client_id}'),
           coalesce(audit.access_detail(t.doc, 'id21') ->> 'value',
                    t.doc #>> '{ehr_id,value}'),
           audit.access_detail(t.doc, 'id22') ->> 'issuer',
           coalesce((audit.access_detail(t.doc, 'id23') ->> 'value')::boolean, false),
           audit.access_detail(t.doc, 'id40') #>> '{defining_code,code_string}',
           (audit.access_detail(t.doc, 'id31') ->> 'value')::jsonb,
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
           (audit.access_detail(t.doc, 'id38') ->> 'value')::jsonb,
           t.token,
           t.uid::text, 'access_log'::text
      FROM (SELECT d.*,
                   coalesce(audit.access_detail(d.doc, 'id11') ->> 'value',
                            CASE d.doc #>> '{action,defining_code,code_string}'
                                 WHEN 'C' THEN 'C' WHEN 'import' THEN 'C'
                                 WHEN 'U' THEN 'U' WHEN 'D' THEN 'D'
                                 WHEN 'R' THEN 'R' WHEN 'query' THEN 'R' WHEN 'export' THEN 'R'
                            END) AS event_action,
                   coalesce(audit.access_detail(d.doc, 'id14') ->> 'value',
                            d.doc #>> '{accessor,credential,subject}',
                            (SELECT nullif(i ->> 'id', 'UNKNOWN')
                               FROM jsonb_array_elements(
                                        coalesce(d.doc #> '{accessor,party,identifiers}', '[]'::jsonb)) i
                              WHERE i ->> 'type' = 'login' LIMIT 1)) AS principal,
                   coalesce(audit.access_details(d.doc, 'id19'),
                            (SELECT coalesce(jsonb_agg(role #> '{defining_code,code_string}'
                                                       ORDER BY n), '[]'::jsonb)
                               FROM jsonb_array_elements(
                                        coalesce(d.doc #> '{accessor,roles}', '[]'::jsonb))
                                    WITH ORDINALITY AS r(role, n)
                              WHERE n <= jsonb_array_length(coalesce(d.doc #> '{accessor,roles}', '[]'::jsonb))
                                         - jsonb_array_length(coalesce(d.token -> 'roles', '[]'::jsonb))))
                       AS roles
              FROM (SELECT r.chain_seq, r.uid, r.time_recorded, x.doc,
                           (audit.access_detail(x.doc, 'id37') ->> 'value')::jsonb AS binding,
                           (audit.access_detail(x.doc, 'id36') ->> 'value')::jsonb AS token
                      FROM access_log_record r
                      LEFT JOIN access_log_shared sh ON sh.digest = r.shared
                     CROSS JOIN LATERAL
                           (SELECT audit.access_log_document(r.content, sh.content, r.record_type,
                                                     r.uid, r.time_recorded, r.ehr_id,
                                                     r.request_id, r.resource_id) AS doc) AS x
                     WHERE r.record_type = 'ACCESS_LOG_ENTRY') AS d) AS t;
COMMENT ON VIEW access_trail IS
    'Every access the audit trail holds, one row each: each audit_event record no write holds, and each access-log entry read from its row and shared parts, each column from the entry''s access detail and else from its core.';
