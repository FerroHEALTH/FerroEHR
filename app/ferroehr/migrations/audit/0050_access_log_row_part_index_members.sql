-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: an access-log entry's row part keeps no member an index column of
-- its row already holds (#6005).
--
-- The request an entry's channel names is its request_id column, and the
-- version its one item names is its resource_id column, about 180 bytes of
-- every read entry's row. The writer leaves channel.request_id out when the
-- column holds its exact text, and the version_id of an only item that names
-- no object when resource_id holds its value; where a reader would give back
-- a member the entry does not have, the row part names it null instead
-- (access_log::stored_form::parts::without_columns). Only a row part without
-- _type is read so; a tombstone holds its members itself.
--
-- audit.access_log_row_part gives a row part its members back, as the server
-- does in Rust (access_log::stored_form::parts::whole), and
-- audit.access_log_document, which the SQL readers of a member both parts or
-- the columns hold use, reads the row part through it.
--
-- No openEHR spec governs audit storage: our own design.
--
-- Runs with search_path = audit, ext, public.

-- As the functions of 0046: no SET clause and every name by its pg_catalog
-- name, so the planner inlines it.
CREATE FUNCTION audit.access_log_row_part(p_row jsonb, p_request_id text, p_resource_id text)
    RETURNS jsonb
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    AS $$
    SELECT CASE WHEN p_row OPERATOR(pg_catalog.?) '_type' THEN p_row
                ELSE p_row
                     OPERATOR(pg_catalog.||) CASE
                         WHEN pg_catalog.jsonb_typeof(p_row OPERATOR(pg_catalog.->) 'channel')
                              OPERATOR(pg_catalog.=) 'object' THEN
                             pg_catalog.jsonb_build_object('channel', CASE
                                 WHEN (p_row OPERATOR(pg_catalog.->) 'channel')
                                      OPERATOR(pg_catalog.?) 'request_id' THEN
                                     CASE WHEN pg_catalog.jsonb_typeof(p_row OPERATOR(pg_catalog.#>)
                                                   '{channel,request_id}'::pg_catalog.text[])
                                               OPERATOR(pg_catalog.=) 'null'
                                          THEN (p_row OPERATOR(pg_catalog.->) 'channel')
                                               OPERATOR(pg_catalog.-) 'request_id'
                                          ELSE p_row OPERATOR(pg_catalog.->) 'channel' END
                                 WHEN p_request_id IS NULL
                                 THEN p_row OPERATOR(pg_catalog.->) 'channel'
                                 ELSE (p_row OPERATOR(pg_catalog.->) 'channel')
                                      OPERATOR(pg_catalog.||)
                                      pg_catalog.jsonb_build_object('request_id', p_request_id)
                             END)
                         ELSE '{}'::pg_catalog.jsonb END
                     OPERATOR(pg_catalog.||) CASE
                         WHEN pg_catalog.jsonb_typeof(p_row OPERATOR(pg_catalog.->) 'items')
                                  OPERATOR(pg_catalog.=) 'array'
                              AND pg_catalog.jsonb_array_length(p_row OPERATOR(pg_catalog.->) 'items')
                                  OPERATOR(pg_catalog.=) 1
                              AND pg_catalog.jsonb_typeof(p_row OPERATOR(pg_catalog.#>)
                                      '{items,0}'::pg_catalog.text[]) OPERATOR(pg_catalog.=) 'object'
                         THEN pg_catalog.jsonb_build_object('items', pg_catalog.jsonb_build_array(CASE
                                 WHEN (p_row OPERATOR(pg_catalog.#>) '{items,0}'::pg_catalog.text[])
                                      OPERATOR(pg_catalog.?) 'version_id' THEN
                                     CASE WHEN pg_catalog.jsonb_typeof(p_row OPERATOR(pg_catalog.#>)
                                                   '{items,0,version_id}'::pg_catalog.text[])
                                               OPERATOR(pg_catalog.=) 'null'
                                          THEN (p_row OPERATOR(pg_catalog.#>) '{items,0}'::pg_catalog.text[])
                                               OPERATOR(pg_catalog.-) 'version_id'
                                          ELSE p_row OPERATOR(pg_catalog.#>) '{items,0}'::pg_catalog.text[]
                                     END
                                 WHEN p_resource_id IS NULL
                                      OR (p_row OPERATOR(pg_catalog.#>) '{items,0}'::pg_catalog.text[])
                                         OPERATOR(pg_catalog.?) 'object'
                                 THEN p_row OPERATOR(pg_catalog.#>) '{items,0}'::pg_catalog.text[]
                                 ELSE (p_row OPERATOR(pg_catalog.#>) '{items,0}'::pg_catalog.text[])
                                      OPERATOR(pg_catalog.||)
                                      pg_catalog.jsonb_build_object('version_id',
                                          pg_catalog.jsonb_build_object('_type', 'OBJECT_VERSION_ID',
                                                                        'value', p_resource_id))
                             END))
                         ELSE '{}'::pg_catalog.jsonb END
           END
$$;

COMMENT ON FUNCTION audit.access_log_row_part(jsonb, text, text) IS
    'An access-log row part with the members its request_id and resource_id columns hold given back: the channel''s request_id and the version_id of an only item that names no object, and without a member it names null.';

-- As 0046, the row part read through audit.access_log_row_part.
CREATE FUNCTION audit.access_log_document(
    p_content text, p_shared text, p_type text, p_uid uuid, p_time timestamptz, p_ehr_id uuid,
    p_request_id text, p_resource_id text)
    RETURNS jsonb
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT CASE WHEN own OPERATOR(pg_catalog.?) '_type' THEN '{}'::pg_catalog.jsonb
                ELSE pg_catalog.jsonb_build_object(
                         '_type', p_type,
                         'uid', pg_catalog.jsonb_build_object(
                             '_type', 'HIER_OBJECT_ID', 'value', p_uid::pg_catalog.text),
                         'time_recorded', pg_catalog.jsonb_build_object(
                             '_type', 'DV_DATE_TIME',
                             'value', audit.access_log_instant_text(p_time)))
                     OPERATOR(pg_catalog.||)
                     CASE WHEN p_ehr_id IS NULL THEN '{}'::pg_catalog.jsonb
                          ELSE pg_catalog.jsonb_build_object('ehr_id', pg_catalog.jsonb_build_object(
                                   '_type', 'HIER_OBJECT_ID', 'value', p_ehr_id::pg_catalog.text))
                     END
           END
           OPERATOR(pg_catalog.||) CASE
                WHEN common IS NULL THEN own
                ELSE (common OPERATOR(pg_catalog.-) 'other_details'
                             OPERATOR(pg_catalog.-) 'outcome' OPERATOR(pg_catalog.-) 'query')
                     OPERATOR(pg_catalog.||)
                     (own OPERATOR(pg_catalog.-) 'other_details'
                          OPERATOR(pg_catalog.-) 'outcome' OPERATOR(pg_catalog.-) 'query')
                     OPERATOR(pg_catalog.||)
                     CASE WHEN common OPERATOR(pg_catalog.?) 'other_details'
                               OR own OPERATOR(pg_catalog.?) 'other_details' THEN
                            pg_catalog.jsonb_build_object('other_details',
                                coalesce(common OPERATOR(pg_catalog.->) 'other_details',
                                         '{}'::pg_catalog.jsonb)
                                OPERATOR(pg_catalog.||)
                                coalesce(own OPERATOR(pg_catalog.->) 'other_details',
                                         '{}'::pg_catalog.jsonb)
                                OPERATOR(pg_catalog.||)
                                pg_catalog.jsonb_build_object('items', (
                                    SELECT coalesce(pg_catalog.jsonb_agg(i.item ORDER BY
                                               i.item OPERATOR(pg_catalog.->>) 'archetype_node_id'
                                                   COLLATE pg_catalog."C" NULLS FIRST,
                                               i.side, i.n), '[]'::pg_catalog.jsonb)
                                      FROM (SELECT c.item, 0 AS side, c.n
                                              FROM pg_catalog.jsonb_array_elements(coalesce(
                                                       common OPERATOR(pg_catalog.#>)
                                                           '{other_details,items}'::pg_catalog.text[],
                                                       '[]'::pg_catalog.jsonb))
                                                   WITH ORDINALITY AS c(item, n)
                                            UNION ALL
                                            SELECT o.item, 1, o.n
                                              FROM pg_catalog.jsonb_array_elements(coalesce(
                                                       own OPERATOR(pg_catalog.#>)
                                                           '{other_details,items}'::pg_catalog.text[],
                                                       '[]'::pg_catalog.jsonb))
                                                   WITH ORDINALITY AS o(item, n)) AS i)))
                        ELSE '{}'::pg_catalog.jsonb END
                     OPERATOR(pg_catalog.||)
                     CASE WHEN common OPERATOR(pg_catalog.?) 'outcome'
                               OR own OPERATOR(pg_catalog.?) 'outcome' THEN
                            pg_catalog.jsonb_build_object('outcome',
                                coalesce(common OPERATOR(pg_catalog.->) 'outcome',
                                         '{}'::pg_catalog.jsonb)
                                OPERATOR(pg_catalog.||)
                                coalesce(own OPERATOR(pg_catalog.->) 'outcome',
                                         '{}'::pg_catalog.jsonb))
                        ELSE '{}'::pg_catalog.jsonb END
                     OPERATOR(pg_catalog.||)
                     CASE WHEN common OPERATOR(pg_catalog.?) 'query'
                               OR own OPERATOR(pg_catalog.?) 'query' THEN
                            pg_catalog.jsonb_build_object('query',
                                coalesce(common OPERATOR(pg_catalog.->) 'query',
                                         '{}'::pg_catalog.jsonb)
                                OPERATOR(pg_catalog.||)
                                coalesce(own OPERATOR(pg_catalog.->) 'query',
                                         '{}'::pg_catalog.jsonb))
                        ELSE '{}'::pg_catalog.jsonb END
              END
      FROM (SELECT audit.access_log_row_part(p_content::pg_catalog.jsonb, p_request_id,
                                             p_resource_id) AS own,
                   p_shared::pg_catalog.jsonb AS common) AS parts
$$;

COMMENT ON FUNCTION audit.access_log_document(text, text, text, uuid, timestamptz, uuid, text, text) IS
    'An access-log record as one jsonb, its canonical form without its link: the row part with the members its index columns hold, the shared part and the members the columns hold, the access details in the order of their codes. The SQL readers of a member both parts or the columns hold read it here; the server joins the parts in Rust.';

-- As 0048, with the index columns the row part leaves members to.
CREATE OR REPLACE FUNCTION audit.access_log_entry_derive(p_row audit.access_log_entry)
    RETURNS audit.access_log_entry
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
                                     p_row.time_recorded, p_row.ehr_id, p_row.request_id,
                                     p_row.resource_id);
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

-- As 0048, with the index columns the row part leaves members to.
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
                                                     r.uid, r.time_recorded, r.ehr_id,
                                                     r.request_id, r.resource_id) AS doc) AS x
                     WHERE r.record_type = 'ACCESS_LOG_ENTRY') AS d) AS t;

DROP FUNCTION audit.access_log_document(text, text, text, uuid, timestamptz, uuid);
