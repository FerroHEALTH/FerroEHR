-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: an access-log entry keeps its caller's part once, in
-- access_log_shared, and its own part on its row (#5703).
--
-- Every entry of a request repeated the accessor, the credential, the roles,
-- the authorisations, the purposes and the retention, and the caller's next
-- request repeated them again. The writer now splits each entry's canonical
-- form: the caller's part is stored once in access_log_shared, keyed by the
-- SHA-256 of its canonical form (which the table checks), and the row keeps
-- the rest. content_digest is the SHA-256 of the whole canonical form,
-- computed by the writer, which alone can produce that form; the insert
-- trigger takes it as given and links it, so the chain still covers every
-- byte, and the server's verifier joins both parts back into the form and
-- recomputes it (Rust, access_log::stored_form). The SQL verifier checks the
-- links, the gaps, the head and the search columns, and an incremental run
-- advances no further than the records the server checked.
--
-- A tombstone replaces one row and keeps no shared part; a shared part no row
-- names any more leaves with the last tombstone that named it, so retention
-- removes the caller's facts with the entries that carried them. A row's
-- content is compressed in place (STORAGE MAIN with a low toast_tuple_target),
-- and no column is moved out of line.
--
-- audit.access_log_document(content, shared) is the entry as one jsonb, for
-- the search columns, the AQL over the log, the ITI-81 filters and the
-- access_trail view; its element order inside other_details is not the
-- canonical one, which only the server's join restores.
--
-- Greenfield: a record written before this migration keeps its whole form on
-- its row with no shared part, which is the form a tombstone has.
--
-- No openEHR spec governs audit storage: our own design.
--
-- Runs with search_path = audit, ext, public.

-- ── the shared part ──────────────────────────────────────────────────────────

CREATE TABLE access_log_shared (
    digest  text NOT NULL,
    content text NOT NULL,
    CONSTRAINT pk_access_log_shared PRIMARY KEY (digest),
    CONSTRAINT ck_access_log_shared_digest
        CHECK (digest = audit.access_log_content_digest(content))
);

COMMENT ON TABLE access_log_shared IS
    'The part of access-log entries their caller shares (accessor, credential, roles, authorisations, purposes, retention and the caller''s access details), once per distinct canonical form; removed when the last row naming it is replaced by its tombstone.';
COMMENT ON COLUMN access_log_shared.digest IS
    'The SHA-256 of content, in lower-case hexadecimal: the key a row names it by.';
COMMENT ON COLUMN access_log_shared.content IS
    'The shared members of the entries, canonicalised by RFC 8785.';

CREATE FUNCTION audit.access_log_shared_unchanged() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.access_log_shared is never rewritten: a shared part leaves only when no record names it'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER access_log_shared_unchanged
    BEFORE UPDATE ON access_log_shared
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_shared_unchanged();
CREATE TRIGGER access_log_shared_no_truncate
    BEFORE TRUNCATE ON access_log_shared
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_shared_unchanged();

ALTER TABLE access_log_record
    ADD COLUMN shared text CONSTRAINT fk_access_log_record_shared
        REFERENCES access_log_shared (digest);

COMMENT ON COLUMN access_log_record.shared IS
    'The digest of the entry''s shared part in access_log_shared; NULL on a tombstone and on a record that keeps its whole form on its row.';
COMMENT ON COLUMN access_log_record.content IS
    'The record''s own part: its members the shared part does not hold and its own access details, canonicalised by RFC 8785; with the shared part joined back, its canonical form without its link.';
COMMENT ON COLUMN access_log_record.content_digest IS
    'ACCESS_LOG_LINK.content_digest: the SHA-256 of the whole canonical form when the entry was written, computed by the writer, in lower-case hexadecimal; a tombstone keeps it.';

CREATE INDEX ix_access_log_record_shared ON access_log_record (shared)
    WHERE shared IS NOT NULL;

-- Every column stays in the row: content compressed in place once the row
-- passes 128 bytes, and nothing moved to the TOAST table but a row larger
-- than a page (PostgreSQL 18 docs, Database Physical Storage, TOAST).
ALTER TABLE access_log_record
    ALTER COLUMN record_type SET STORAGE MAIN,
    ALTER COLUMN uid SET STORAGE MAIN,
    ALTER COLUMN system_id SET STORAGE MAIN,
    ALTER COLUMN content SET STORAGE MAIN,
    ALTER COLUMN algorithm SET STORAGE MAIN,
    ALTER COLUMN previous_digest SET STORAGE MAIN,
    ALTER COLUMN content_digest SET STORAGE MAIN,
    ALTER COLUMN digest SET STORAGE MAIN,
    ALTER COLUMN ehr_id SET STORAGE MAIN,
    ALTER COLUMN action_terminology SET STORAGE MAIN,
    ALTER COLUMN action_code SET STORAGE MAIN,
    ALTER COLUMN result_terminology SET STORAGE MAIN,
    ALTER COLUMN result_code SET STORAGE MAIN,
    ALTER COLUMN party_id SET STORAGE MAIN,
    ALTER COLUMN request_id SET STORAGE MAIN,
    ALTER COLUMN shared SET STORAGE MAIN;
ALTER TABLE access_log_record SET (toast_tuple_target = 128);

-- ── the entry as one document ────────────────────────────────────────────────

CREATE FUNCTION audit.access_log_document(p_content text, p_shared text) RETURNS jsonb
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE
        WHEN p_shared IS NULL THEN own
        WHEN common ? 'other_details' THEN
            (common - 'other_details') || (own - 'other_details')
            || jsonb_build_object('other_details',
                   (common -> 'other_details')
                   || jsonb_build_object('items',
                          coalesce(common #> '{other_details,items}', '[]'::jsonb)
                          || coalesce(own #> '{other_details,items}', '[]'::jsonb)))
        ELSE common || own
    END
      FROM (SELECT p_content::jsonb AS own, p_shared::jsonb AS common) AS parts
$$;

COMMENT ON FUNCTION audit.access_log_document(text, text) IS
    'An access-log record as one jsonb: its row part with its shared part joined, the access details of both in one list; the canonical form only the server''s join restores.';

-- ── the writer links the digest it is given ─────────────────────────────────

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
    doc := audit.access_log_document(p_row.content, common);
    IF jsonb_typeof(p_row.content::jsonb) IS DISTINCT FROM 'object' OR doc ? 'link' THEN
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

CREATE OR REPLACE FUNCTION audit.access_log_record_chain_link() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.access_log_chain_head', true);
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RAISE EXCEPTION 'the access-log chain cursor is unset: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW := audit.access_log_record_derive(NEW);
    IF NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_ENTRY' THEN
        RAISE EXCEPTION 'an access-log record is written as an ACCESS_LOG_ENTRY, not %', NEW.record_type
            USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.content_digest IS NULL THEN
        RAISE EXCEPTION 'an access-log entry is written with the content digest of its canonical form'
            USING ERRCODE = 'not_null_violation';
    END IF;
    NEW.chain_seq := split_part(cursor_value, ':', 1)::bigint + 1;
    NEW.previous_digest := split_part(cursor_value, ':', 2);
    NEW.algorithm := 'SHA-256';
    NEW.digest := audit.access_log_link_digest(NEW.previous_digest, NEW.chain_seq, NEW.content_digest);
    NEW.stored_at := now();
    PERFORM set_config('ferroehr.access_log_chain_head', NEW.chain_seq || ':' || NEW.digest, true);
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
       OR NEW.shared IS NOT NULL
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

-- The tombstone lets go of the entry's index row, its feed debt and, when no
-- other record names it, its shared part.
CREATE OR REPLACE FUNCTION audit.access_log_record_unindex() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF NEW.record_type = 'ACCESS_LOG_TOMBSTONE' THEN
        DELETE FROM audit.access_log_search s WHERE s.chain_seq = NEW.chain_seq;
        DELETE FROM audit.fhir_feed_outbox o
         WHERE o.chain = 'access_log' AND o.chain_seq = NEW.chain_seq;
        IF OLD.shared IS NOT NULL THEN
            DELETE FROM audit.access_log_shared s
             WHERE s.digest = OLD.shared
               AND NOT EXISTS (SELECT 1 FROM audit.access_log_record r WHERE r.shared = OLD.shared);
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

COMMENT ON FUNCTION audit.access_log_record_unindex() IS
    'Removes the index row, the FHIR Feed debt and, when no other record names it, the shared part of an entry replaced by its tombstone.';

-- ── the verification checks links; the server checks the content ────────────

CREATE OR REPLACE FUNCTION audit.verify_access_log_chain_after(p_after_seq bigint, p_after_digest text)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql STABLE
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    head_position bigint;
    head_link     text;
    highest       bigint;
    highest_link  text;
BEGIN
    SELECT s.head_seq, s.head_digest INTO head_position, head_link
      FROM audit.access_log_chain_state s WHERE s.singleton;
    IF NOT FOUND THEN
        RETURN QUERY SELECT NULL::bigint, NULL::uuid, NULL::timestamptz,
            'the access-log chain state row is missing: the log cannot be verified'::text;
        RETURN;
    END IF;

    RETURN QUERY
    WITH elements AS (
        SELECT r.chain_seq AS first_seq, r.chain_seq AS last_seq,
               CASE WHEN r.uid ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                    THEN r.uid::uuid END AS id,
               r.time_recorded, r.previous_digest, r.digest, false AS gap,
               r.digest IS DISTINCT FROM audit.access_log_link_digest(
                   r.previous_digest, r.chain_seq, r.content_digest)
                 OR audit.access_log_record_derive(r) IS DISTINCT FROM r AS modified
          FROM audit.access_log_record r
         WHERE r.chain_seq > p_after_seq
        UNION ALL
        SELECT g.first_seq, g.last_seq, NULL::uuid, NULL::timestamptz, NULL::text, g.link,
               true, false
          FROM audit.access_log_chain_gap g
         WHERE g.last_seq > p_after_seq
    ),
    linked AS (
        SELECT e.*,
               coalesce(lag(e.digest) OVER w, p_after_digest) AS before_digest,
               coalesce(lag(e.last_seq) OVER w, p_after_seq) AS before_seq,
               lag(e.last_seq) OVER w IS NULL AS opening
          FROM elements e
        WINDOW w AS (ORDER BY e.last_seq, e.first_seq)
    ),
    judged AS (
        SELECT l.*,
               l.first_seq = l.before_seq + 1
                 OR (l.gap AND l.opening AND l.first_seq <= p_after_seq + 1) AS adjacent
          FROM linked l
    )
    SELECT j.first_seq, j.id, j.time_recorded,
           CASE
             WHEN j.modified
               THEN 'record content was modified after it was written'
             WHEN NOT j.adjacent
               THEN 'record(s) deleted between chain position ' || j.before_seq
                    || ' and this one, with no gap for the removal'
             ELSE 'the link to the preceding record does not match'
           END::text
      FROM judged j
     WHERE j.modified
        OR NOT j.adjacent
        OR (NOT j.gap AND j.previous_digest IS DISTINCT FROM j.before_digest);

    SELECT n.last_seq, n.link INTO highest, highest_link
      FROM (
          (SELECT r.chain_seq AS last_seq, r.digest AS link
             FROM audit.access_log_record r ORDER BY r.chain_seq DESC LIMIT 1)
          UNION ALL
          (SELECT g.last_seq, g.link
             FROM audit.access_log_chain_gap g ORDER BY g.last_seq DESC LIMIT 1)
      ) AS n
     ORDER BY n.last_seq DESC LIMIT 1;
    highest := coalesce(highest, 0);

    IF highest <> head_position THEN
        RETURN QUERY SELECT highest, NULL::uuid, NULL::timestamptz,
            'record(s) deleted from the end of the access log'::text;
    ELSIF highest > 0 AND highest_link IS DISTINCT FROM head_link THEN
        RETURN QUERY SELECT highest, NULL::uuid, NULL::timestamptz,
            'the newest record does not match the recorded head of the access log'::text;
    END IF;
END;
$$;

-- The incremental verification over what the server checked: p_through is
-- the newest position whose content the server recomputed (NULL: none
-- bounds it), p_modified the positions whose content did not; each is a
-- finding, and the cursor never moves past p_through.
DROP FUNCTION audit.verify_access_log_chain_incremental(boolean);

CREATE FUNCTION audit.verify_access_log_chain_incremental(
    p_full boolean DEFAULT false,
    p_through bigint DEFAULT NULL,
    p_modified bigint[] DEFAULT '{}')
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    from_seq    bigint := 0;
    from_digest text := audit.access_log_genesis();
    damage      audit.audit_chain_finding[];
    top_seq     bigint;
    top_digest  text;
BEGIN
    IF NOT coalesce(p_full, false) THEN
        SELECT c.through_seq, c.through_digest INTO from_seq, from_digest
          FROM audit.access_log_chain_verified c WHERE c.singleton;
        IF NOT FOUND THEN
            from_seq := 0;
            from_digest := audit.access_log_genesis();
        END IF;
    END IF;

    SELECT coalesce(array_agg(f ORDER BY f.chain_seq NULLS FIRST), '{}')
      INTO damage
      FROM (
          SELECT * FROM audit.verify_access_log_chain_after(from_seq, from_digest)
          UNION ALL
          SELECT r.chain_seq,
                 CASE WHEN r.uid ~* '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                      THEN r.uid::uuid END,
                 r.time_recorded,
                 'record content was modified after it was written'::text
            FROM audit.access_log_record r
           WHERE r.chain_seq = ANY (coalesce(p_modified, '{}'))
      ) AS f;

    SELECT n.last_seq, n.link INTO top_seq, top_digest
      FROM (
          (SELECT r.chain_seq AS last_seq, r.digest AS link
             FROM audit.access_log_record r
            WHERE p_through IS NULL OR r.chain_seq <= p_through
            ORDER BY r.chain_seq DESC LIMIT 1)
          UNION ALL
          (SELECT g.last_seq, g.link
             FROM audit.access_log_chain_gap g
            WHERE p_through IS NULL OR g.last_seq <= p_through
            ORDER BY g.last_seq DESC LIMIT 1)
      ) AS n
     ORDER BY n.last_seq DESC LIMIT 1;

    IF cardinality(damage) = 0 AND top_seq IS NOT NULL THEN
        INSERT INTO audit.access_log_chain_verified AS c
               (through_seq, through_digest, verified_at, full_verified_at)
        VALUES (top_seq, top_digest, now(), CASE WHEN p_full THEN now() END)
        ON CONFLICT (singleton) DO UPDATE
           SET through_seq = excluded.through_seq,
               through_digest = excluded.through_digest,
               verified_at = excluded.verified_at,
               full_verified_at = coalesce(excluded.full_verified_at, c.full_verified_at)
         WHERE excluded.through_seq >= c.through_seq;
    END IF;

    RETURN QUERY SELECT * FROM unnest(damage);
END;
$$;

-- ── the views read the joined document ───────────────────────────────────────

DROP VIEW audit_record;
DROP VIEW access_trail;

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
    SELECT s.chain_seq, s.recorded_at, r.stored_at, s.action, s.outcome, NULL::text,
           s.principal, s.patient_id, NULL::text, s.resource_id,
           s.client_ip, NULL::text, NULL::text, NULL::text, NULL::text, NULL::bigint,
           s.request_id, NULL::jsonb, NULL::jsonb, NULL::bigint, NULL::text,
           NULL::text, NULL::text, NULL::text, NULL::text,
           false, NULL::text, NULL::text, NULL::text, s.ehr_id,
           NULL::text, false, NULL::jsonb,
           s.render_policy, NULL::jsonb, s.search_tokens,
           r.content, sh.content, r.algorithm, r.previous_digest, r.content_digest, r.digest,
           'access_log'::text
      FROM access_log_search s
      JOIN access_log_record r ON r.chain_seq = s.chain_seq
      LEFT JOIN access_log_shared sh ON sh.digest = r.shared;

COMMENT ON VIEW audit_record IS
    'Every record the ITI-81 search, the person''s own trail and the FHIR Feed read: an audit_event record of the system chain by its columns, an access-log entry by its row and shared parts and its link, which its projection is rendered from. A record is named by its chain and position.';

-- A detail the entry's core states is not written (#5703), so each column
-- reads the detail and else the core, as the server's reader does.
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
           audit.access_detail(t.doc, 'at0014') ->> 'value',
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
           t.uid, 'access_log'::text
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
                           (SELECT audit.access_log_document(r.content, sh.content) AS doc) AS x
                     WHERE r.record_type = 'ACCESS_LOG_ENTRY') AS d) AS t;

COMMENT ON VIEW access_trail IS
    'Every access the audit trail holds, one row each: an audit_event record of the system chain, and each access-log entry read from its row and shared parts, each column from the entry''s access detail and else from its core.';

-- ── grants ───────────────────────────────────────────────────────────────────

REVOKE ALL ON FUNCTION audit.access_log_shared_unchanged() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.verify_access_log_chain_incremental(boolean, bigint, bigint[]) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON access_log_shared FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON access_log_shared TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT INSERT ON access_log_shared TO ferroehr_clinical;
        GRANT INSERT (shared, content_digest), UPDATE (shared) ON access_log_record
            TO ferroehr_clinical;
        REVOKE ALL ON audit_record, access_trail FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON audit_record, access_trail TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION audit.verify_access_log_chain_incremental(boolean, bigint, bigint[])
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the access-log shared part grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
