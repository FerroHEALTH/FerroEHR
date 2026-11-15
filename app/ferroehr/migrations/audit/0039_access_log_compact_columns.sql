-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the access log keeps its digests as bytes and its uid as a uuid,
-- drops the search columns another holds (#5714), and a published
-- checkpoint keeps verifying after retention folds the run it sealed (#5727).
--
-- Storage (#5714): what repeats of an entry's outcome and query joins the
-- shared part, the row keeping the delivered count and the rows; previous_digest, content_digest, digest and the shared
-- part's key are 32-byte bytea in place of 64-character hex, and uid is a
-- uuid; every function and view that hands them on still speaks hex, so a
-- link reads as the draft writes it. system_id is the recording system's
-- own, in the content of every record, and request_id is the index row's
-- (access_log_search) and the content's, so neither is a column any more.
--
-- Published checkpoints (#5727): the publication marks a checkpoint it wrote
-- outside the database (published_at, set once). The fold keeps a published
-- checkpoint inside the run it folds, and the gap records, for each, the
-- position, the link the log held there and the SHA-256 of the checkpoint's
-- proof, its signed statement. audit.access_log_sealed_link() answers the
-- link a checkpoint seals from a record, the end of a gap, or a gap's record
-- of that checkpoint, so a checkpoint published before the fold verifies
-- after it. An unpublished checkpoint inside a run still leaves with it.
--
-- No openEHR spec governs audit storage or retention: our own design.
--
-- Runs with search_path = audit, ext, public.

-- ── the columns ──────────────────────────────────────────────────────────────

DROP VIEW audit_record;
DROP VIEW access_trail;

ALTER TABLE access_log_record DROP CONSTRAINT fk_access_log_record_shared;
ALTER TABLE access_log_record DROP CONSTRAINT ck_access_log_record_digests;
ALTER TABLE access_log_shared DROP CONSTRAINT ck_access_log_shared_digest;

ALTER TABLE access_log_shared ALTER COLUMN digest TYPE bytea USING decode(digest, 'hex');
ALTER TABLE access_log_shared
    ADD CONSTRAINT ck_access_log_shared_digest
        CHECK (digest = sha256(convert_to(content, 'UTF8')));
COMMENT ON COLUMN access_log_shared.digest IS
    'The SHA-256 of content: the key a row names it by.';

ALTER TABLE access_log_record
    ALTER COLUMN previous_digest TYPE bytea USING decode(previous_digest, 'hex'),
    ALTER COLUMN content_digest TYPE bytea USING decode(content_digest, 'hex'),
    ALTER COLUMN digest TYPE bytea USING decode(digest, 'hex'),
    ALTER COLUMN shared TYPE bytea USING decode(shared, 'hex'),
    ALTER COLUMN uid TYPE uuid USING uid::uuid,
    DROP COLUMN system_id,
    DROP COLUMN request_id;
ALTER TABLE access_log_record
    ALTER COLUMN previous_digest SET STORAGE MAIN,
    ALTER COLUMN content_digest SET STORAGE MAIN,
    ALTER COLUMN digest SET STORAGE MAIN,
    ALTER COLUMN shared SET STORAGE MAIN,
    ADD CONSTRAINT ck_access_log_record_digests
        CHECK (octet_length(previous_digest) = 32
               AND octet_length(content_digest) = 32
               AND octet_length(digest) = 32),
    ADD CONSTRAINT fk_access_log_record_shared
        FOREIGN KEY (shared) REFERENCES access_log_shared (digest);

-- A person's entries are read by EHR newest first, ties broken by position,
-- so the EHR index carries both and serves that order without a sort.
DROP INDEX ix_access_log_record_ehr;
CREATE INDEX ix_access_log_record_ehr ON access_log_record (ehr_id, time_recorded, chain_seq)
    WHERE ehr_id IS NOT NULL;

COMMENT ON COLUMN access_log_record.uid IS
    'The record''s uid, derived from content.';
COMMENT ON COLUMN access_log_record.previous_digest IS
    'ACCESS_LOG_LINK.previous_digest, the 32 bytes the link writes in lower-case hexadecimal.';
COMMENT ON COLUMN access_log_record.content_digest IS
    'ACCESS_LOG_LINK.content_digest: the SHA-256 of the whole canonical form when the entry was written, computed by the writer; a tombstone keeps it.';
COMMENT ON COLUMN access_log_record.digest IS
    'ACCESS_LOG_LINK.digest: the SHA-256 of the hexadecimal previous_digest, a line feed, the decimal chain_seq, a line feed and the hexadecimal content_digest.';

-- ── the entry as one document ────────────────────────────────────────────────

-- The shared part also holds what repeats of an entry's outcome and query,
-- the row the count each delivered (#5714), so both are merged member by
-- member.
CREATE OR REPLACE FUNCTION audit.access_log_document(p_content text, p_shared text) RETURNS jsonb
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE
        WHEN p_shared IS NULL THEN own
        ELSE (common - 'other_details' - 'outcome' - 'query')
             || (own - 'other_details' - 'outcome' - 'query')
             || CASE WHEN common ? 'other_details' THEN
                    jsonb_build_object('other_details',
                        (common -> 'other_details')
                        || jsonb_build_object('items',
                               coalesce(common #> '{other_details,items}', '[]'::jsonb)
                               || coalesce(own #> '{other_details,items}', '[]'::jsonb)))
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

-- ── the functions speak hex ──────────────────────────────────────────────────

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
    p_row.uid := (doc #>> '{uid,value}')::uuid;
    p_row.time_recorded := (doc #>> '{time_recorded,value}')::timestamptz;
    p_row.ehr_id := doc #>> '{ehr_id,value}';
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
    NEW.previous_digest := decode(split_part(cursor_value, ':', 2), 'hex');
    NEW.algorithm := 'SHA-256';
    NEW.digest := decode(audit.access_log_link_digest(split_part(cursor_value, ':', 2),
                                                      NEW.chain_seq,
                                                      encode(NEW.content_digest, 'hex')), 'hex');
    NEW.stored_at := now();
    PERFORM set_config('ferroehr.access_log_chain_head',
                      NEW.chain_seq || ':' || encode(NEW.digest, 'hex'), true);
    RETURN NEW;
END;
$$;

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
    RETURN NEW;
END;
$$;

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
               r.uid AS id,
               r.time_recorded, encode(r.previous_digest, 'hex') AS previous_digest,
               encode(r.digest, 'hex') AS digest, false AS gap,
               encode(r.digest, 'hex') IS DISTINCT FROM audit.access_log_link_digest(
                   encode(r.previous_digest, 'hex'), r.chain_seq, encode(r.content_digest, 'hex'))
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
          (SELECT r.chain_seq AS last_seq, encode(r.digest, 'hex') AS link
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

-- The incremental verification over what the server checked (0038), the
-- digests read as hex.
CREATE OR REPLACE FUNCTION audit.verify_access_log_chain_incremental(
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
          SELECT r.chain_seq, r.uid, r.time_recorded,
                 'record content was modified after it was written'::text
            FROM audit.access_log_record r
           WHERE r.chain_seq = ANY (coalesce(p_modified, '{}'))
      ) AS f;

    SELECT n.last_seq, n.link INTO top_seq, top_digest
      FROM (
          (SELECT r.chain_seq AS last_seq, encode(r.digest, 'hex') AS link
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

CREATE OR REPLACE FUNCTION audit.access_log_link_at(p_sequence bigint) RETURNS text
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT coalesce(
        (SELECT encode(r.digest, 'hex') FROM audit.access_log_record r WHERE r.chain_seq = p_sequence),
        (SELECT g.link FROM audit.access_log_chain_gap g WHERE g.last_seq = p_sequence))
$$;

-- ── a published checkpoint survives the fold ────────────────────────────────

ALTER TABLE access_log_checkpoint ADD COLUMN published_at timestamptz;
COMMENT ON COLUMN access_log_checkpoint.published_at IS
    'When the publication wrote the checkpoint outside the database, set once; the fold keeps a published checkpoint.';

ALTER TABLE access_log_chain_gap ADD COLUMN seals jsonb NOT NULL DEFAULT '[]'::jsonb;
ALTER TABLE access_log_chain_gap
    ADD CONSTRAINT ck_access_log_chain_gap_seals CHECK (jsonb_typeof(seals) = 'array');
COMMENT ON COLUMN access_log_chain_gap.seals IS
    'Each published checkpoint inside the run, before its last position: its sequence, the link digest the log held there and the SHA-256 of its proof, in lower-case hexadecimal.';

CREATE OR REPLACE FUNCTION audit.access_log_checkpoint_reject_removal() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND coalesce(current_setting('ferroehr.access_log_folding', true), 'off') = 'on'
    THEN
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' AND OLD.published_at IS NULL AND NEW.published_at IS NOT NULL
       AND (NEW.sequence, NEW.system_id, NEW.digest, NEW.time_sealed, NEW.proof)
           IS NOT DISTINCT FROM (OLD.sequence, OLD.system_id, OLD.digest, OLD.time_sealed, OLD.proof)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'audit.access_log_checkpoint is append-only: a checkpoint is marked published once and leaves only with the run of tombstones it lies inside'
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- The link digest `p_proof`'s checkpoint at `p_sequence` seals, in
-- lower-case hexadecimal: the record's, the link of the gap that ends
-- there, or the link a gap recorded for that checkpoint when it folded the
-- position; NULL when none of them holds it.
CREATE FUNCTION audit.access_log_sealed_link(p_sequence bigint, p_proof text) RETURNS text
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT coalesce(
        audit.access_log_link_at(p_sequence),
        (SELECT s ->> 'link'
           FROM audit.access_log_chain_gap g
          CROSS JOIN LATERAL jsonb_array_elements(g.seals) AS s
          WHERE g.first_seq <= p_sequence AND g.last_seq > p_sequence
            AND (s ->> 'sequence')::bigint = p_sequence
            AND s ->> 'proof' = encode(sha256(convert_to(p_proof, 'UTF8')), 'hex')
          LIMIT 1))
$$;

COMMENT ON FUNCTION audit.access_log_sealed_link(bigint, text) IS
    'The link a checkpoint at a position seals: the log''s link there, or the one a gap recorded for that checkpoint''s proof when it folded the position.';

CREATE OR REPLACE FUNCTION audit.fold_access_log_tombstones(p_now timestamptz, p_limit integer)
    RETURNS TABLE (folded bigint, gaps bigint)
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF p_now IS NULL OR coalesce(p_limit, 0) < 1 THEN
        RETURN QUERY SELECT 0::bigint, 0::bigint;
        RETURN;
    END IF;
    PERFORM pg_advisory_xact_lock(hashtext('audit.access_log_chain_gap'), 0);
    PERFORM set_config('ferroehr.access_log_folding', 'on', true);
    RETURN QUERY
    WITH due AS (
        SELECT r.chain_seq, encode(r.digest, 'hex') AS digest, r.content
          FROM audit.access_log_record r
         WHERE r.fold_after <= p_now
         ORDER BY r.fold_after, r.chain_seq
         LIMIT p_limit
    ),
    islanded AS (
        SELECT d.*, d.chain_seq - row_number() OVER (ORDER BY d.chain_seq) AS island
          FROM due d
    ),
    grounded AS (
        SELECT i.island, jsonb_agg(DISTINCT g.ground ORDER BY g.ground) AS grounds
          FROM islanded i
          CROSS JOIN LATERAL jsonb_array_elements(i.content::jsonb -> 'grounds') AS g(ground)
         GROUP BY i.island
    ),
    spans AS (
        SELECT min(i.chain_seq) AS first_seq, max(i.chain_seq) AS last_seq,
               (array_agg(i.digest ORDER BY i.chain_seq DESC))[1] AS link,
               i.island
          FROM islanded i
         GROUP BY i.island
    ),
    sealed AS (
        SELECT s.island,
               jsonb_agg(jsonb_build_object(
                   'sequence', c.sequence,
                   'link', i.digest,
                   'proof', encode(sha256(convert_to(c.proof, 'UTF8')), 'hex'))
                 ORDER BY c.sequence) AS seals
          FROM spans s
          JOIN audit.access_log_checkpoint c
            ON c.sequence >= s.first_seq AND c.sequence < s.last_seq
               AND c.published_at IS NOT NULL
          JOIN islanded i ON i.chain_seq = c.sequence
         GROUP BY s.island
    ),
    written AS (
        INSERT INTO audit.access_log_chain_gap
               (first_seq, last_seq, link, grounds, time_folded, seals)
        SELECT s.first_seq, s.last_seq, s.link, g.grounds, p_now,
               coalesce(x.seals, '[]'::jsonb)
          FROM spans s
          JOIN grounded g ON g.island = s.island
          LEFT JOIN sealed x ON x.island = s.island
         ORDER BY s.first_seq
        RETURNING 1
    ),
    unsealed AS (
        DELETE FROM audit.access_log_checkpoint c
         USING spans s
         WHERE c.sequence >= s.first_seq AND c.sequence < s.last_seq
           AND c.published_at IS NULL
        RETURNING 1
    ),
    removed AS (
        DELETE FROM audit.access_log_record r
         USING due d
         WHERE r.chain_seq = d.chain_seq
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM removed), (SELECT count(*) FROM written)
      FROM (SELECT count(*) FROM unsealed) AS taken;
    PERFORM set_config('ferroehr.access_log_folding', 'off', true);
END;
$$;

COMMENT ON FUNCTION audit.fold_access_log_tombstones(timestamptz, integer) IS
    'The one path a tombstone leaves the access log by: up to p_limit tombstones past their end fold into one access_log_chain_gap per run; a published checkpoint inside a run stays and the gap records the link it seals, an unpublished one leaves with the run.';

-- ── the views ────────────────────────────────────────────────────────────────

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
           r.content, sh.content, r.algorithm, encode(r.previous_digest, 'hex'),
           encode(r.content_digest, 'hex'), encode(r.digest, 'hex'),
           'access_log'::text
      FROM access_log_search s
      JOIN access_log_record r ON r.chain_seq = s.chain_seq
      LEFT JOIN access_log_shared sh ON sh.digest = r.shared;

COMMENT ON VIEW audit_record IS
    'Every record the ITI-81 search, the person''s own trail and the FHIR Feed read: an audit_event record of the system chain by its columns, an access-log entry by its row and shared parts and its link, which its projection is rendered from. A record is named by its chain and position.';

-- A detail the entry's core states is not written (#5703, #5714), so each column
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
                           (SELECT audit.access_log_document(r.content, sh.content) AS doc) AS x
                     WHERE r.record_type = 'ACCESS_LOG_ENTRY') AS d) AS t;

COMMENT ON VIEW access_trail IS
    'Every access the audit trail holds, one row each: an audit_event record of the system chain, and each access-log entry read from its row and shared parts, each column from the entry''s access detail and else from its core.';

-- ── grants ───────────────────────────────────────────────────────────────────

REVOKE ALL ON FUNCTION audit.access_log_sealed_link(bigint, text) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON audit_record, access_trail FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON audit_record, access_trail TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT UPDATE (published_at) ON access_log_checkpoint TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.access_log_sealed_link(bigint, text)
            TO ferroehr_clinical, ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the access-log column and seal grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
