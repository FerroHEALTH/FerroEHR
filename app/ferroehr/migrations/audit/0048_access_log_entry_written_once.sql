-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: an access-log entry is written once (#5968).
--
-- An entry was written twice: into access_log_intake before the response,
-- and again, after the chainer deleted it there, into access_log_record with
-- its thirteen indexes. Measured with full-page writes off, the intake copy
-- and its delete were about 1500 of the 4000 bytes of WAL an entry cost.
--
-- Now the request writes the entry into access_log_entry, its only row, with
-- every column and index it keeps. The chainer adds one narrow
-- access_log_link row per entry under the head: the position, the previous
-- digest and the digest, checked against the head and the entry's content
-- digest by the link table's statement trigger. access_log_record is the
-- view of the entries that have a link, with the columns the table had, so
-- every reader keeps reading chained records only. access_log_intake keeps
-- one row per intake statement, the ids of the entries it wrote and their
-- reservation, until the chainer links them: the one row a request writes
-- that is deleted again.
--
-- A shared part is no longer removed with the last row that names it, which
-- took an index on every entry's shared digest. It carries the latest
-- retain_until of the entries that name it, raised by the intake when an
-- entry outlives it, and the retention sweep collects the parts past it that
-- no entry names, in bounded batches read through the retain_until index
-- (audit.access_log_shared_collect).
--
-- No openEHR spec governs audit storage: our own design.
--
-- Runs with search_path = audit, ext, public.

-- ── what reads the record table ─────────────────────────────────────────────

DROP VIEW access_log_search;
DROP VIEW audit_record;
DROP VIEW access_trail;
DROP FUNCTION audit.access_log_record_derive(audit.access_log_record);

DROP TRIGGER access_log_record_chain_begin ON access_log_record;
DROP TRIGGER access_log_record_chain_commit ON access_log_record;
DROP TRIGGER access_log_record_chain_link ON access_log_record;
DROP TRIGGER access_log_record_reject_deletion ON access_log_record;
DROP TRIGGER access_log_record_reject_truncate ON access_log_record;
DROP TRIGGER access_log_record_tombstone_only ON access_log_record;
DROP TRIGGER access_log_record_unindex ON access_log_record;
DROP FUNCTION audit.access_log_chain_begin();
DROP FUNCTION audit.access_log_chain_commit();
DROP FUNCTION audit.access_log_record_chain_link();
DROP FUNCTION audit.access_log_record_reject_removal();
DROP FUNCTION audit.access_log_record_tombstone_only();
DROP FUNCTION audit.access_log_record_unindex();
DROP FUNCTION audit.access_log_chain_move(bigint[], bigint[], text[], text[], boolean, boolean);

-- ── the entry ───────────────────────────────────────────────────────────────

ALTER TABLE access_log_record RENAME TO access_log_entry;
ALTER TABLE access_log_entry RENAME CONSTRAINT uq_access_log_record_uid TO uq_access_log_entry_uid;
ALTER TABLE access_log_entry RENAME CONSTRAINT ck_access_log_record_type TO ck_access_log_entry_type;
ALTER TABLE access_log_entry RENAME CONSTRAINT ck_access_log_record_removed TO ck_access_log_entry_removed;
ALTER TABLE access_log_entry RENAME CONSTRAINT ck_access_log_record_index TO ck_access_log_entry_index;
ALTER TABLE access_log_entry
    RENAME CONSTRAINT ck_access_log_record_event_action TO ck_access_log_entry_event_action;
ALTER TABLE access_log_entry
    RENAME CONSTRAINT ck_access_log_record_event_outcome TO ck_access_log_entry_event_outcome;
ALTER INDEX ix_access_log_record_fold_after RENAME TO ix_access_log_entry_fold_after;
ALTER INDEX ix_access_log_record_party RENAME TO ix_access_log_entry_party;
ALTER INDEX ix_access_log_record_patient RENAME TO ix_access_log_entry_patient;
ALTER INDEX ix_access_log_record_principal RENAME TO ix_access_log_entry_principal;
ALTER INDEX ix_access_log_record_request_id RENAME TO ix_access_log_entry_request_id;
ALTER INDEX ix_access_log_record_resource_id RENAME TO ix_access_log_entry_resource_id;
ALTER INDEX ix_access_log_record_retain_until RENAME TO ix_access_log_entry_retain_until;
ALTER INDEX ix_access_log_record_time RENAME TO ix_access_log_entry_time;
ALTER INDEX ix_access_log_record_tokens RENAME TO ix_access_log_entry_tokens;
DROP INDEX ix_access_log_record_shared;

ALTER TABLE access_log_entry ADD COLUMN id bigint NOT NULL GENERATED ALWAYS AS IDENTITY;

COMMENT ON TABLE access_log_entry IS
    'The openEHR access log''s records (draft org.openehr.rm.access_log 0.1.0), each an ACCESS_LOG_ENTRY or the ACCESS_LOG_TOMBSTONE that replaced one, written once by the request that made the access; its link is its access_log_link row, and access_log_record is the view of the linked ones. Append-only except the replacement of an entry by its tombstone.';
COMMENT ON COLUMN access_log_entry.id IS
    'The number the intake and the link name the entry by.';

-- ── the link ────────────────────────────────────────────────────────────────

CREATE TABLE access_log_link (
    chain_seq       bigint      NOT NULL,
    entry_id        bigint      NOT NULL,
    algorithm       text        NOT NULL,
    previous_digest bytea       NOT NULL,
    digest          bytea       NOT NULL,
    stored_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_log_link PRIMARY KEY (chain_seq),
    CONSTRAINT uq_access_log_link_entry UNIQUE (entry_id),
    CONSTRAINT ck_access_log_link_algorithm CHECK (algorithm = 'SHA-256'),
    CONSTRAINT ck_access_log_link_digests
        CHECK (octet_length(previous_digest) = 32 AND octet_length(digest) = 32)
);

ALTER TABLE access_log_link
    ALTER COLUMN previous_digest SET STORAGE MAIN,
    ALTER COLUMN digest SET STORAGE MAIN;

COMMENT ON TABLE access_log_link IS
    'The ACCESS_LOG_LINK of each chained access-log entry: its position, the digest of the link before it and its own digest, written by the chainer under the head of the log.';
COMMENT ON COLUMN access_log_link.chain_seq IS 'ACCESS_LOG_LINK.sequence.';
COMMENT ON COLUMN access_log_link.entry_id IS 'The access_log_entry the link chains.';
COMMENT ON COLUMN access_log_link.previous_digest IS 'ACCESS_LOG_LINK.previous_digest.';
COMMENT ON COLUMN access_log_link.digest IS
    'ACCESS_LOG_LINK.digest: the SHA-256 of previous_digest, a line feed, the decimal chain_seq, a line feed and the entry''s content_digest, each in lower-case hexadecimal.';
COMMENT ON COLUMN access_log_link.stored_at IS 'When the chainer linked the entry.';

INSERT INTO access_log_link (chain_seq, entry_id, algorithm, previous_digest, digest, stored_at)
SELECT e.chain_seq, e.id, e.algorithm, e.previous_digest, e.digest, e.stored_at
  FROM access_log_entry e
 ORDER BY e.chain_seq;

ALTER TABLE access_log_entry
    DROP COLUMN chain_seq,
    DROP COLUMN algorithm,
    DROP COLUMN previous_digest,
    DROP COLUMN digest,
    DROP COLUMN stored_at,
    ADD CONSTRAINT pk_access_log_entry PRIMARY KEY (id);

-- A person's lookup by time reads this index in its order, the entry id breaking a
-- tie, as the chain position did.
CREATE INDEX ix_access_log_entry_ehr ON access_log_entry (ehr_id, time_recorded, id)
    WHERE ehr_id IS NOT NULL;

-- The ITI-81 total is counted from the time index alone: the planner removes
-- the link from access_log_search for a count, and still reads the entry id
-- its join named, so the index carries it.
DROP INDEX ix_access_log_entry_time;
CREATE INDEX ix_access_log_entry_time ON access_log_entry (time_recorded)
    INCLUDE (render_policy_id, id);

-- ── the shared part's retention ─────────────────────────────────────────────

ALTER TABLE access_log_shared ADD COLUMN retain_until date;

UPDATE access_log_shared s
   SET retain_until = greatest(coalesce(
           (SELECT max(coalesce(e.retain_until, 'infinity'::date))
              FROM access_log_entry e WHERE e.shared = s.digest),
           '-infinity'::date),
       coalesce((SELECT max(coalesce(i.retain_until, 'infinity'::date))
                   FROM access_log_intake i WHERE i.shared = s.digest),
                '-infinity'::date));

ALTER TABLE access_log_shared ALTER COLUMN retain_until SET NOT NULL;

CREATE INDEX ix_access_log_shared_retain_until ON access_log_shared (retain_until);

COMMENT ON TABLE access_log_shared IS
    'The part of access-log entries their caller shares (accessor, credential, roles, authorisations, purposes, retention and the caller''s access details), once per distinct canonical form; collected by the retention sweep once its retain_until has passed and no entry names it.';
COMMENT ON COLUMN access_log_shared.retain_until IS
    'The latest retain_until of the entries that name the part, infinity when one states none, and never before the day the intake last named it.';

-- A shared part's content never changes: the intake only raises its
-- retain_until.
CREATE OR REPLACE FUNCTION audit.access_log_shared_unchanged() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'UPDATE'
       AND NEW.digest = OLD.digest AND NEW.content = OLD.content
       AND NEW.retain_until > OLD.retain_until
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'audit.access_log_shared is never rewritten: a shared part leaves only when no record names it'
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- ── the intake ──────────────────────────────────────────────────────────────

ALTER TABLE access_log_intake RENAME TO access_log_intake_before;

CREATE TABLE access_log_intake (
    id          bigint      NOT NULL GENERATED ALWAYS AS IDENTITY,
    taken_at    timestamptz NOT NULL DEFAULT now(),
    entries     bigint[]    NOT NULL,
    reserved_by uuid,
    CONSTRAINT pk_access_log_intake_batch PRIMARY KEY (id),
    CONSTRAINT ck_access_log_intake_entries CHECK (cardinality(entries) > 0)
) WITH (vacuum_truncate = off);

CREATE INDEX ix_access_log_intake_held ON access_log_intake (reserved_by)
    WHERE reserved_by IS NOT NULL;

COMMENT ON TABLE access_log_intake IS
    'One row per intake statement: the access_log_entry ids it wrote, which no link chains yet. Append-only; left only by audit.access_log_chain_move(), which links its entries in the same statement, or by the discard of a reservation that never committed.';
COMMENT ON COLUMN access_log_intake.taken_at IS
    'When the request wrote the entries, for the age of the oldest not chained yet.';
COMMENT ON COLUMN access_log_intake.entries IS
    'The access_log_entry ids, in the order the chainer links them.';
COMMENT ON COLUMN access_log_intake.reserved_by IS
    'The reservation a write''s records are held under until its commit is confirmed (#5762); the chainer takes no held row. NULL for records that are not held.';

INSERT INTO access_log_entry (record_type, uid, time_recorded, content, content_digest, ehr_id,
    party_id, retain_until, action_terminology, action_code, result_terminology, result_code,
    shared, event_action, event_outcome, principal, patient_id, resource_id, request_id,
    client_ip, classified_ehr_id, search_tokens, render_policy_id)
SELECT i.record_type, i.uid, i.time_recorded, i.content, i.content_digest,
       audit.access_log_canonical_uuid(i.ehr_id), i.party_id, i.retain_until,
       i.action_terminology, i.action_code, i.result_terminology, i.result_code, i.shared,
       i.action, i.outcome, i.principal, i.patient_id, i.resource_id, i.request_id, i.client_ip,
       i.index_ehr_id, i.search_tokens, p.id
  FROM access_log_intake_before i
  LEFT JOIN audit_render_policy p ON p.digest = i.render_policy
 ORDER BY i.id;

INSERT INTO access_log_intake (taken_at, entries, reserved_by)
SELECT i.taken_at, ARRAY[e.id], i.reserved_by
  FROM access_log_intake_before i
  JOIN access_log_entry e ON e.uid = i.uid
 ORDER BY i.id;

DROP TABLE access_log_intake_before;

CREATE TRIGGER access_log_intake_append_only
    BEFORE UPDATE OR DELETE ON access_log_intake
    FOR EACH ROW EXECUTE FUNCTION audit.audit_intake_reject_change();
CREATE TRIGGER access_log_intake_no_truncate
    BEFORE TRUNCATE ON access_log_intake
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_intake_reject_change();

-- ── what the link refuses ───────────────────────────────────────────────────

-- The links a statement inserts, checked together against the head: each at
-- the position after the one before it, following its digest, over the
-- content digest of an entry no other link chains, and the head moved to
-- the last. One statement for all of them, so a round costs the same
-- statements however many entries it links.
CREATE FUNCTION audit.access_log_link_append() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    head_position bigint;
    head_link     text;
    wrong         bigint;
    last_position bigint;
    last_link     text;
BEGIN
    SELECT s.head_seq, s.head_digest INTO head_position, head_link
      FROM audit.access_log_chain_state s
     WHERE s.singleton
       FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'the access-log chain state is missing: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    SELECT min(o.chain_seq) FILTER (WHERE o.wrong), max(o.chain_seq),
           (array_agg(encode(o.digest, 'hex') ORDER BY o.chain_seq DESC))[1]
      INTO wrong, last_position, last_link
      FROM (SELECT a.chain_seq, a.digest,
                   e.record_type IS DISTINCT FROM 'ACCESS_LOG_ENTRY'
                   OR a.chain_seq IS DISTINCT FROM head_position + row_number() OVER w
                   OR a.previous_digest IS DISTINCT FROM
                      coalesce(lag(a.digest) OVER w, decode(head_link, 'hex'))
                   OR a.algorithm IS DISTINCT FROM 'SHA-256'
                   OR a.digest IS DISTINCT FROM decode(audit.access_log_link_digest(
                          encode(a.previous_digest, 'hex'), a.chain_seq,
                          encode(e.content_digest, 'hex')), 'hex') AS wrong
              FROM added a
              LEFT JOIN audit.access_log_entry e ON e.id = a.entry_id
            WINDOW w AS (ORDER BY a.chain_seq)) AS o;
    IF wrong IS NOT NULL THEN
        RAISE EXCEPTION 'an access-log record is written with the link that follows position % of the log', head_position
            USING ERRCODE = 'check_violation';
    END IF;
    IF last_position IS NOT NULL THEN
        UPDATE audit.access_log_chain_state
           SET head_seq = last_position, head_digest = last_link, updated_at = now()
         WHERE singleton;
    END IF;
    RETURN NULL;
END;
$$;

COMMENT ON FUNCTION audit.access_log_link_append() IS
    'The one check of a statement''s new links against the head of the access log, which it moves to the last of them.';

-- A link never changes, and leaves only with its tombstone's fold into a gap.
CREATE FUNCTION audit.access_log_link_reject_change() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND coalesce(current_setting('ferroehr.access_log_folding', true), 'off') = 'on'
       AND EXISTS (SELECT 1 FROM audit.access_log_entry e
                    WHERE e.id = OLD.entry_id AND e.record_type = 'ACCESS_LOG_TOMBSTONE')
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'audit.access_log_link is append-only: a link leaves only with its tombstone''s fold into a gap'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER access_log_link_append
    AFTER INSERT ON access_log_link
    REFERENCING NEW TABLE AS added
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_link_append();
CREATE TRIGGER access_log_link_unchanged
    BEFORE UPDATE OR DELETE ON access_log_link
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_link_reject_change();
CREATE TRIGGER access_log_link_no_truncate
    BEFORE TRUNCATE ON access_log_link
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_link_reject_change();

REVOKE ALL ON FUNCTION audit.access_log_link_append() FROM PUBLIC;

-- ── the columns an entry decides ─────────────────────────────────────────────

-- As 0046, over the entry's own row.
CREATE FUNCTION audit.access_log_entry_derive(p_row audit.access_log_entry)
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

COMMENT ON FUNCTION audit.access_log_entry_derive(audit.access_log_entry) IS
    'The entry row with every column its content decides derived from the content again, which the tombstone trigger writes and the verifier compares.';

-- ── what the entry refuses ──────────────────────────────────────────────────

-- As 0046's record trigger: an entry is only replaced by its own tombstone,
-- which keeps its uid, time, content digest and system, and empties its
-- index columns.
CREATE FUNCTION audit.access_log_entry_tombstone_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    common text;
BEGIN
    NEW := audit.access_log_entry_derive(NEW);
    SELECT s.content INTO common FROM audit.access_log_shared s WHERE s.digest = OLD.shared;
    IF OLD.record_type <> 'ACCESS_LOG_ENTRY'
       OR NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_TOMBSTONE'
       OR NEW.shared IS NOT NULL
       OR NEW.uid IS DISTINCT FROM OLD.uid
       OR NEW.content::jsonb ->> 'system_id'
          IS DISTINCT FROM audit.access_log_document(OLD.content, common, OLD.record_type, OLD.uid,
                                                     OLD.time_recorded, OLD.ehr_id) ->> 'system_id'
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

-- A tombstone leaves the outboxes of the sinks; its shared part stays until
-- the retention sweep collects it.
CREATE FUNCTION audit.access_log_entry_unindex() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF NEW.record_type = 'ACCESS_LOG_TOMBSTONE' THEN
        DELETE FROM audit.fhir_feed_outbox o
         USING audit.access_log_link l
         WHERE l.entry_id = NEW.id AND o.chain = 'access_log' AND o.chain_seq = l.chain_seq;
        DELETE FROM audit.syslog_outbox o
         USING audit.access_log_link l
         WHERE l.entry_id = NEW.id AND o.chain = 'access_log' AND o.chain_seq = l.chain_seq;
    END IF;
    RETURN NEW;
END;
$$;

-- An entry leaves only as a tombstone folded into a gap, or unchained, when
-- the reservation it was held under never committed.
CREATE FUNCTION audit.access_log_entry_reject_removal() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND OLD.record_type = 'ACCESS_LOG_TOMBSTONE'
       AND coalesce(current_setting('ferroehr.access_log_folding', true), 'off') = 'on'
    THEN
        RETURN OLD;
    END IF;
    IF TG_OP = 'DELETE'
       AND coalesce(current_setting('ferroehr.audit_discarding', true), 'off') = 'on'
       AND NOT EXISTS (SELECT 1 FROM audit.access_log_link l WHERE l.entry_id = OLD.id)
    THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'audit.access_log_entry is append-only: a record is replaced by its tombstone, and a tombstone leaves only by the fold into a gap'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER access_log_entry_tombstone_only
    BEFORE UPDATE ON access_log_entry
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_entry_tombstone_only();
CREATE TRIGGER access_log_entry_unindex
    AFTER UPDATE ON access_log_entry
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_entry_unindex();
CREATE TRIGGER access_log_entry_reject_deletion
    BEFORE DELETE ON access_log_entry
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_entry_reject_removal();
CREATE TRIGGER access_log_entry_reject_truncate
    BEFORE TRUNCATE ON access_log_entry
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_entry_reject_removal();

REVOKE ALL ON FUNCTION audit.access_log_entry_unindex() FROM PUBLIC;

-- ── the views ───────────────────────────────────────────────────────────────

-- The chained records, with the columns of the table they were before.
CREATE VIEW access_log_record AS
    SELECT l.chain_seq, e.record_type, e.uid, e.time_recorded, e.content, l.algorithm,
           l.previous_digest, e.content_digest, l.digest, l.stored_at, e.ehr_id, e.party_id,
           e.retain_until, e.time_removed, e.fold_after, e.action_terminology, e.action_code,
           e.result_terminology, e.result_code, e.shared, e.event_action, e.event_outcome,
           e.principal, e.patient_id, e.resource_id, e.request_id, e.client_ip,
           e.classified_ehr_id, e.search_tokens, e.render_policy_id, e.id AS entry_id
      FROM access_log_entry e
      JOIN access_log_link l ON l.entry_id = e.id;

COMMENT ON VIEW access_log_record IS
    'The openEHR access log (draft org.openehr.rm.access_log 0.1.0): every chained ACCESS_LOG_ENTRY or ACCESS_LOG_TOMBSTONE, its entry row with its link. The record as served is its canonical form with this link. Every reader of the log reads it here.';

-- The link is joined only for the position: a count names no column of it,
-- so the planner removes the join (the link names an entry at most once) and
-- counts from the time index alone. The count then also holds the entries of
-- the intake that the chainer has not linked yet, a fraction of a second of
-- them; a page reads only linked entries.
CREATE VIEW access_log_search AS
    SELECT l.chain_seq, e.time_recorded AS recorded_at, e.event_action AS action,
           e.event_outcome AS outcome, e.principal, e.patient_id, e.resource_id,
           e.request_id, e.client_ip, e.classified_ehr_id AS ehr_id, e.search_tokens
      FROM access_log_entry e
      LEFT JOIN access_log_link l ON l.entry_id = e.id
     WHERE e.render_policy_id IS NOT NULL;

COMMENT ON VIEW access_log_search IS
    'The ITI-81 index of the access-log entries: what the search filters on, read from the entry''s row, and the position of a linked entry (NULL for one the chainer has not linked yet). A tombstone is not in it.';

-- As 0046, over the view of the chained records.
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

-- As 0046, over the view of the chained records.
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
                           (SELECT audit.access_log_document(r.content, sh.content, r.record_type,
                                                     r.uid, r.time_recorded, r.ehr_id) AS doc) AS x
                     WHERE r.record_type = 'ACCESS_LOG_ENTRY') AS d) AS t;


COMMENT ON VIEW access_trail IS
    'Every access the audit trail holds, one row each: an audit_event record of the system chain, and each access-log entry read from its row and shared parts, each column from the entry''s access detail and else from its core.';

-- ── the move into the chain ─────────────────────────────────────────────────

-- Link the entries p_entries of the intake rows p_batches at the positions
-- p_seqs with the links the chainer computed, write the outbox rows of the
-- sinks that deliver from the store, and remove the intake rows. Every entry
-- the batches name is linked, or nothing is.
CREATE FUNCTION audit.access_log_chain_move(
    p_batches bigint[], p_entries bigint[], p_seqs bigint[], p_previous text[],
    p_digests text[], p_fhir_feed boolean, p_syslog boolean)
    RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    named bigint;
    listed bigint;
    moved bigint;
BEGIN
    PERFORM set_config('ferroehr.audit_chaining', 'on', true);
    WITH taken AS (
        DELETE FROM audit.access_log_intake i
         WHERE i.id = ANY (p_batches) AND i.reserved_by IS NULL
        RETURNING i.entries
    )
    SELECT count(*) FILTER (WHERE n.id = ANY (p_entries)), count(*) INTO named, listed
      FROM taken t
     CROSS JOIN LATERAL unnest(t.entries) AS n(id);
    PERFORM set_config('ferroehr.audit_chaining', 'off', true);
    IF named IS DISTINCT FROM cardinality(p_entries)::bigint OR listed IS DISTINCT FROM named THEN
        RAISE EXCEPTION 'the chainer named % entries and the intake rows it took name % entries, % of them those',
            cardinality(p_entries), listed, named
            USING ERRCODE = 'restrict_violation';
    END IF;
    INSERT INTO audit.access_log_link (chain_seq, entry_id, algorithm, previous_digest, digest)
    SELECT l.seq, l.entry, 'SHA-256', decode(l.previous, 'hex'), decode(l.digest, 'hex')
      FROM unnest(p_entries, p_seqs, p_previous, p_digests) AS l(entry, seq, previous, digest)
     ORDER BY l.seq;
    GET DIAGNOSTICS moved = ROW_COUNT;
    IF p_fhir_feed THEN
        INSERT INTO audit.fhir_feed_outbox (chain, chain_seq)
        SELECT 'access_log', l.seq
          FROM unnest(p_entries, p_seqs) AS l(entry, seq)
          JOIN audit.access_log_entry e ON e.id = l.entry
         WHERE e.render_policy_id IS NOT NULL
         ORDER BY l.seq;
    END IF;
    IF p_syslog THEN
        INSERT INTO audit.syslog_outbox (chain, chain_seq)
        SELECT 'access_log', l.seq
          FROM unnest(p_entries, p_seqs) AS l(entry, seq)
          JOIN audit.access_log_entry e ON e.id = l.entry
         WHERE e.render_policy_id IS NOT NULL
         ORDER BY l.seq;
    END IF;
    RETURN moved;
END;
$$;

COMMENT ON FUNCTION audit.access_log_chain_move(bigint[], bigint[], bigint[], text[], text[], boolean, boolean) IS
    'The one path an access-log entry enters the chain by: its link at the position the chainer computed, its outbox rows, and the removal of the intake rows that named it, all of whose entries are linked in the same statement.';

REVOKE ALL ON FUNCTION audit.access_log_chain_move(bigint[], bigint[], bigint[], text[], text[], boolean, boolean)
    FROM PUBLIC;

-- ── release and discard ─────────────────────────────────────────────────────

-- As 0042, counting the entries the released intake rows name.
CREATE OR REPLACE FUNCTION audit.audit_intake_release(p_ids uuid[]) RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    v_entries bigint;
    v_others  bigint;
BEGIN
    PERFORM set_config('ferroehr.audit_releasing', 'on', true);
    WITH released AS (
        UPDATE audit.access_log_intake SET reserved_by = NULL
         WHERE reserved_by = ANY (p_ids)
        RETURNING cardinality(entries) AS n
    )
    SELECT coalesce(sum(r.n), 0) INTO v_entries FROM released r;
    UPDATE audit.audit_event_intake SET reserved_by = NULL WHERE reserved_by = ANY (p_ids);
    GET DIAGNOSTICS v_others = ROW_COUNT;
    PERFORM set_config('ferroehr.audit_releasing', 'off', true);
    RETURN v_entries + v_others;
END;
$$;

-- As 0042, removing the entries the discarded intake rows name, which no
-- link chains.
CREATE OR REPLACE FUNCTION audit.audit_intake_discard(p_ids uuid[]) RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    v_entries bigint;
    v_others  bigint;
BEGIN
    PERFORM set_config('ferroehr.audit_discarding', 'on', true);
    WITH dropped AS (
        DELETE FROM audit.access_log_intake WHERE reserved_by = ANY (p_ids)
        RETURNING entries
    ),
    removed AS (
        DELETE FROM audit.access_log_entry e
         USING dropped d
         WHERE e.id = ANY (d.entries)
        RETURNING 1
    )
    SELECT count(*) INTO v_entries FROM removed;
    DELETE FROM audit.audit_event_intake WHERE reserved_by = ANY (p_ids);
    GET DIAGNOSTICS v_others = ROW_COUNT;
    PERFORM set_config('ferroehr.audit_discarding', 'off', true);
    RETURN v_entries + v_others;
END;
$$;

-- ── the readers of a position ───────────────────────────────────────────────

-- As 0033, from the link.
CREATE OR REPLACE FUNCTION audit.access_log_link_at(p_sequence bigint) RETURNS text
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT coalesce(
        (SELECT encode(l.digest, 'hex') FROM audit.access_log_link l WHERE l.chain_seq = p_sequence),
        (SELECT g.link FROM audit.access_log_chain_gap g WHERE g.last_seq = p_sequence))
$$;

-- As 0041, from the links.
CREATE OR REPLACE FUNCTION audit.access_log_verification_step_end(
    p_after bigint, p_records integer, p_head bigint)
    RETURNS bigint
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT greatest(p_after, coalesce(
        (SELECT l.chain_seq FROM audit.access_log_link l
          WHERE l.chain_seq > p_after AND l.chain_seq < p_head
          ORDER BY l.chain_seq OFFSET greatest(p_records, 1) - 1 LIMIT 1),
        p_head))
$$;

-- ── the fold ────────────────────────────────────────────────────────────────

-- As 0037, removing each folded tombstone's link and entry.
CREATE OR REPLACE FUNCTION audit.fold_access_log_tombstones(p_now timestamptz, p_limit integer)
    RETURNS TABLE (folded bigint, gaps bigint)
    LANGUAGE plpgsql SECURITY DEFINER
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
        SELECT r.chain_seq, r.entry_id, encode(r.digest, 'hex') AS digest, r.content
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
    unlinked AS (
        DELETE FROM audit.access_log_link l
         USING due d
         WHERE l.chain_seq = d.chain_seq
        RETURNING 1
    ),
    removed AS (
        DELETE FROM audit.access_log_entry e
         USING due d
         WHERE e.id = d.entry_id
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM removed), (SELECT count(*) FROM written)
      FROM (SELECT count(*) FROM unsealed) AS taken,
           (SELECT count(*) FROM unlinked) AS links;
    PERFORM set_config('ferroehr.access_log_folding', 'off', true);
END;
$$;

-- ── the verifier ────────────────────────────────────────────────────────────

-- As 0033, reading each link with its entry, whose columns are derived
-- from its content again and compared.
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
                 OR audit.access_log_entry_derive(e) IS DISTINCT FROM e AS modified
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

-- ── the collection of shared parts ──────────────────────────────────────────

-- Remove up to p_limit shared parts whose retain_until is before p_today and
-- the database's own date and that no entry names. An entry that names a
-- part has a retain_until at or before the part's, or the part's is
-- infinity, so the check reads only the entries the retention sweep has not
-- tombstoned yet, through the retain_until index. A part an intake statement
-- holds is skipped, and one it named since this statement began no longer
-- qualifies, since the intake raised its retain_until to today at least.
CREATE FUNCTION audit.access_log_shared_collect(p_today date, p_limit integer) RETURNS bigint
    LANGUAGE sql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    WITH due AS (
        SELECT s.digest, s.retain_until
          FROM audit.access_log_shared s
         WHERE s.retain_until < least(p_today, current_date)
         ORDER BY s.retain_until, s.digest
         LIMIT greatest(p_limit, 0)
           FOR UPDATE SKIP LOCKED
    ),
    gone AS (
        DELETE FROM audit.access_log_shared s
         USING due d
         WHERE s.digest = d.digest
           AND NOT EXISTS (SELECT 1 FROM audit.access_log_entry e
                            WHERE e.retain_until <= d.retain_until AND e.shared = d.digest)
        RETURNING 1
    )
    SELECT count(*) FROM gone
$$;

COMMENT ON FUNCTION audit.access_log_shared_collect(date, integer) IS
    'Remove up to p_limit shared parts past their retain_until that no entry names; the retention sweep calls it until it removes none.';

REVOKE ALL ON FUNCTION audit.access_log_shared_collect(date, integer) FROM PUBLIC;

-- ── the intake statement ────────────────────────────────────────────────────

-- As 0046, writing each entry once into access_log_entry and one intake row
-- naming them; a shared part's retain_until follows its entries'.
CREATE OR REPLACE FUNCTION audit.access_log_intake_take(
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
        INSERT INTO audit.access_log_shared (digest, content, retain_until)
        SELECT decode(t.digest, 'hex'), t.content, greatest(
                   (SELECT max(coalesce(r.until::date, 'infinity'::date))
                      FROM unnest(p_shared, p_retain_until) AS r(digest, until)
                     WHERE r.digest = t.digest),
                   current_date)
          FROM unnest(p_shared_digests, p_shared_contents) AS t(digest, content)
         ORDER BY decode(t.digest, 'hex')
        ON CONFLICT (digest) DO NOTHING;
    END IF;
    WITH held AS (
        SELECT s.digest, s.retain_until
          FROM audit.access_log_shared s
         WHERE s.digest IN (SELECT DISTINCT decode(t.c, 'hex')
                              FROM unnest(p_shared) AS t(c)
                             WHERE t.c IS NOT NULL)
         ORDER BY s.digest
           FOR KEY SHARE
    ),
    needed AS (
        SELECT decode(t.c, 'hex') AS digest,
               greatest(max(coalesce(t.until::date, 'infinity'::date)), current_date) AS until
          FROM unnest(p_shared, p_retain_until) AS t(c, until)
         WHERE t.c IS NOT NULL
         GROUP BY t.c
    ),
    kept AS (
        UPDATE audit.access_log_shared s
           SET retain_until = n.until
          FROM needed n
          JOIN held h ON h.digest = n.digest
         WHERE s.digest = n.digest AND h.retain_until < n.until
        RETURNING 1
    ),
    written AS (
        INSERT INTO audit.access_log_entry (record_type, uid, time_recorded, content, shared,
            content_digest, ehr_id, party_id, retain_until, action_terminology, action_code,
            result_terminology, result_code, event_action, event_outcome, principal, patient_id,
            resource_id, request_id, client_ip, classified_ehr_id, search_tokens,
            render_policy_id)
        SELECT u.c1, u.c2::uuid, u.c3::timestamptz, u.c4,
               CASE
                   WHEN u.c5 IS NOT NULL AND held.digest IS NULL
                   THEN audit.access_log_intake_unknown('shared part', u.c5, batch.digest IS NOT NULL)
                   ELSE decode(u.c5, 'hex')
               END,
               decode(u.c6, 'hex'),
               audit.access_log_canonical_uuid(u.c7), u.c8, u.c9::date, u.c10, u.c11, u.c12,
               u.c13, u.c15, u.c16, u.c17, u.c18, u.c19, u.c20, u.c21, u.c22,
               CASE
                   WHEN u.c23 IS NULL THEN NULL
                   WHEN u.c23 <> '' THEN u.c23::integer[]
                   WHEN x.id IS NULL THEN coalesce(g.own, '{}')::integer[]
                   ELSE ARRAY[-x.id] || coalesce(g.own, '{}')::integer[]
               END,
               CASE
                   WHEN u.c24 IS NOT NULL AND policy.digest IS NULL
                   -- access_log_intake_unknown raises; octet_length only types the arm.
                   THEN octet_length(audit.access_log_intake_unknown(
                            'render policy', encode(u.c24, 'hex'), false))
                   ELSE policy.id
               END
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
         ORDER BY u.n
        RETURNING id
    )
    INSERT INTO audit.access_log_intake (entries, reserved_by)
    SELECT array_agg(w.id ORDER BY w.id), p_reserved_by
      FROM written w
    HAVING count(*) > 0;
END
$$;

COMMENT ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[], uuid, bytea[]) IS
    'One batch''s access-log entries, each written once: their search tokens, found by the digests the server computed, grouped by entry in one pass, their shared sets and shared parts stored or found, checked and kept as long as the entries, their rows in access_log_entry, and one intake row naming them for the chainer (#5770, #5887, #5889, #5968).';

-- ── grants ──────────────────────────────────────────────────────────────────

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT SELECT ON access_log_record, access_log_search, audit_record, access_trail,
                        access_log_link, access_log_intake
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT INSERT ON access_log_entry, access_log_intake TO ferroehr_clinical;
        -- A tombstone locks the record it replaces through the view (PostgreSQL 18, SELECT,
        -- "The Locking Clause"); the view itself admits no update.
        GRANT UPDATE (content) ON access_log_record TO ferroehr_clinical;
        -- The intake raises a shared part's retain_until when an entry outlives it.
        GRANT UPDATE (retain_until) ON access_log_shared TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.access_log_chain_move(bigint[], bigint[], bigint[], text[],
            text[], boolean, boolean) TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.access_log_shared_collect(date, integer) TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the access-log grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
