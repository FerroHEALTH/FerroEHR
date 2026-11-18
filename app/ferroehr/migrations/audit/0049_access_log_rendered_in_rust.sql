-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: an access-log entry is read as its stored parts and rendered once,
-- by the server (#6005).
--
-- audit_record rebuilt each entry's canonical form as a jsonb document
-- (audit.access_log_document) for every reader: 4.0 of the 4.9 ms of
-- PostgreSQL time an ITI-81 page of 20 records cost. The view now gives the
-- entry's stored parts and the columns that hold the members its row part
-- leaves out, and the server joins them into the exact canonical form the
-- content digest covers (access_log::stored_form::parts::whole), which it
-- reads into the entry and renders.
--
-- No openEHR spec governs audit storage: our own design.
--
-- Runs with search_path = audit, ext, public.

DROP VIEW audit_record;

CREATE VIEW audit_record AS
    SELECT e.chain_seq, e.recorded_at, e.stored_at, e.action, e.outcome, e.operation,
           e.principal, e.patient_id, e.resource_class, e.resource_id,
           e.client_ip, e.token_id, e.domain, e.purpose, e.legal_basis, e.result_count,
           e.request_id, e.roles, e.origins, e.origin_count, e.category_map_digest,
           e.acting_mode, e.assurance_level, e.assurance_value, e.professional_id,
           e.emergency_access, e.permit, e.issuer, e.client_id, e.ehr_id,
           e.subject_namespace, e.subject_lookup_failed, e.category_content,
           e.render_policy, e.token_attributes, e.search_tokens,
           NULL::text AS row_part, NULL::text AS shared_part, NULL::text AS entry_type,
           NULL::uuid AS entry_uid, NULL::uuid AS entry_ehr,
           NULL::text AS algorithm, NULL::text AS previous_digest,
           NULL::text AS content_digest, NULL::text AS digest,
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
           r.content, sh.content, r.record_type, r.uid, r.ehr_id,
           r.algorithm, encode(r.previous_digest, 'hex'),
           encode(r.content_digest, 'hex'), encode(r.digest, 'hex'),
           'access_log'::text
      FROM access_log_record r
      JOIN audit_render_policy p ON p.id = r.render_policy_id
      LEFT JOIN access_log_shared sh ON sh.digest = r.shared;

COMMENT ON VIEW audit_record IS
    'Every record the ITI-81 search, the person''s own trail and the FHIR Feed read: an audit_event record of the system chain by its columns, an access-log entry by its stored parts, the columns its row part leaves out (recorded_at is its time_recorded) and its link, from which the server renders it. A record is named by its chain and position.';

-- ── the verifier ────────────────────────────────────────────────────────────

-- As 0048, with no derivation of an entry's columns: the server reads each
-- record's parts, joins them, and checks the content digest and every
-- derived column (system_log::store::chain_walk), and passes the positions
-- it found changed to the verification step.
CREATE OR REPLACE FUNCTION audit.verify_access_log_chain_window(
    p_after_seq bigint, p_after_digest text, p_upto bigint, p_check_head boolean)
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
        SELECT l.chain_seq AS first_seq, l.chain_seq AS last_seq,
               e.uid AS id,
               e.time_recorded, encode(l.previous_digest, 'hex') AS previous_digest,
               encode(l.digest, 'hex') AS digest, false AS gap,
               encode(l.digest, 'hex') IS DISTINCT FROM audit.access_log_link_digest(
                   encode(l.previous_digest, 'hex'), l.chain_seq, encode(e.content_digest, 'hex'))
                 AS modified
          FROM audit.access_log_link l
          JOIN audit.access_log_entry e ON e.id = l.entry_id
         WHERE l.chain_seq > p_after_seq
           AND (p_upto IS NULL OR l.chain_seq <= p_upto)
        UNION ALL
        SELECT g.first_seq, g.last_seq, NULL::uuid, NULL::timestamptz, NULL::text, g.link,
               true, false
          FROM audit.access_log_chain_gap g
         WHERE g.last_seq > p_after_seq
           AND (p_upto IS NULL OR g.last_seq <= p_upto)
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

    IF NOT p_check_head THEN
        RETURN;
    END IF;

    SELECT n.last_seq, n.link INTO highest, highest_link
      FROM (
          (SELECT l.chain_seq AS last_seq, encode(l.digest, 'hex') AS link
             FROM audit.access_log_link l ORDER BY l.chain_seq DESC LIMIT 1)
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

-- ── the tombstone ───────────────────────────────────────────────────────────

-- As 0048, reading the replaced entry's system_id from its row part, which
-- keeps it (access_log::stored_form::SHARED leaves it out of the shared
-- part), with no document of the entry built. The tombstone's own columns
-- are derived from its content, a whole canonical form.
CREATE OR REPLACE FUNCTION audit.access_log_entry_tombstone_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    NEW := audit.access_log_entry_derive(NEW);
    IF OLD.record_type <> 'ACCESS_LOG_ENTRY'
       OR NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_TOMBSTONE'
       OR NEW.shared IS NOT NULL
       OR NEW.uid IS DISTINCT FROM OLD.uid
       OR NEW.content::jsonb ->> 'system_id' IS DISTINCT FROM OLD.content::jsonb ->> 'system_id'
       OR NEW.time_recorded IS DISTINCT FROM OLD.time_recorded
       OR (NEW.id, NEW.content_digest) IS DISTINCT FROM (OLD.id, OLD.content_digest)
    THEN
        RAISE EXCEPTION
            'audit.access_log_entry is append-only: an entry is only replaced by its own tombstone (entry %)',
            OLD.id
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

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT ON audit_record TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the audit_record grant (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
