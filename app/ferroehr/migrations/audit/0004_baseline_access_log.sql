-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the openEHR access log (draft org.openehr.rm.access_log 0.1.0).
--
-- Each entry is written once by the request that made the access; the
-- chainer links it under the head of the log (access_log_link) and signs
-- checkpoints of the log. A retention sweep replaces an entry past its end by
-- its tombstone, and the fold turns a run of tombstones past theirs into a
-- gap. The verifier walks the links and derives every column again from the
-- content.
--
-- No openEHR spec governs audit storage: our own design. The access log
-- package it realizes is a draft, which is no openEHR release.
--
-- Runs with search_path = audit, ext, public.

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

-- Reads the replaced entry's system_id from its row part, which keeps it
-- (access_log::stored_form::SHARED leaves it out of the shared part), with no
-- document of the entry built. The tombstone's own columns
-- are derived from its content, a whole canonical form.
CREATE FUNCTION audit.access_log_entry_tombstone_only() RETURNS trigger
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
COMMENT ON FUNCTION access_log_link_append() IS
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

-- A shared part's content never changes: the intake only raises its
-- retain_until.
CREATE FUNCTION audit.access_log_shared_unchanged() RETURNS trigger
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

CREATE FUNCTION audit.access_log_checkpoint_append_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.access_log_checkpoint is append-only'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE FUNCTION audit.access_log_checkpoint_names_a_link() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF audit.access_log_link_at(NEW.sequence) IS DISTINCT FROM NEW.digest THEN
        RAISE EXCEPTION 'an access-log checkpoint seals the link the log holds at its position (sequence %)',
            NEW.sequence
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION audit.access_log_checkpoint_reject_removal() RETURNS trigger
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

CREATE FUNCTION audit.access_log_chain_gap_append_only() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.access_log_chain_gap is append-only: a fold is never undone or rewritten'
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- ── the access log's own chain ───────────────────────────────────────────────
CREATE FUNCTION audit.access_log_genesis() RETURNS text
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT encode(sha256(convert_to('ferroehr.access_log.v1', 'UTF8')), 'hex') $$;
COMMENT ON FUNCTION access_log_genesis() IS
    'ACCESS_LOG_LINK.previous_digest of the first record of this system''s access log.';

-- ── the shared part ──────────────────────────────────────────────────────────
CREATE TABLE access_log_shared (
    digest       bytea NOT NULL,
    content      text  NOT NULL,
    -- The latest retention end of the entries that name the part.
    retain_until date  NOT NULL,
    CONSTRAINT pk_access_log_shared PRIMARY KEY (digest),
    CONSTRAINT ck_access_log_shared_digest
        CHECK (digest = sha256(convert_to(content, 'UTF8')))
);
CREATE INDEX ix_access_log_shared_retain_until ON access_log_shared (retain_until);
CREATE TRIGGER access_log_shared_unchanged
    BEFORE UPDATE ON access_log_shared
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_shared_unchanged();
CREATE TRIGGER access_log_shared_no_truncate
    BEFORE TRUNCATE ON access_log_shared
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_shared_unchanged();
COMMENT ON COLUMN access_log_shared.content IS
    'The shared members of the entries, canonicalised by RFC 8785.';
COMMENT ON COLUMN access_log_shared.digest IS
    'The SHA-256 of content: the key a row names it by.';
COMMENT ON TABLE access_log_shared IS
    'The part of access-log entries their caller shares (accessor, credential, roles, authorisations, purposes, retention and the caller''s access details), once per distinct canonical form; collected by the retention sweep once its retain_until has passed and no entry names it.';
COMMENT ON COLUMN access_log_shared.retain_until IS
    'The latest retain_until of the entries that name the part, infinity when one states none, and never before the day the intake last named it.';

-- ── access_log_entry ─────────────────────────────────────────────────────────
-- The openEHR access log's records (draft org.openehr.rm.access_log 0.1.0):
-- each an ACCESS_LOG_ENTRY, or the ACCESS_LOG_TOMBSTONE that replaced one.
-- The request that made the access writes the entry here once, its only row,
-- with every column and index it keeps; the chainer then adds its narrow
-- access_log_link row under the head of the log, and access_log_record is the
-- view of the linked entries. Append-only except the replacement of an entry
-- by its tombstone.
--
-- The derived columns are read from content by the writer and checked again
-- by the verifier (audit.access_log_entry_derive). Every column a row keeps is
-- STORAGE MAIN under a toast_tuple_target of 128, so a row stays on its heap
-- page instead of spilling small values into TOAST (PostgreSQL 18, "TOAST").
--
-- No openEHR spec governs audit storage: our own design.
CREATE TABLE access_log_entry (
    -- The record's class, the _type of content.
    record_type        text        STORAGE MAIN NOT NULL,
    uid                uuid        NOT NULL,
    time_recorded      timestamptz NOT NULL,
    -- The record's own part, canonicalised by RFC 8785: with the shared part
    -- joined back, its canonical form without its link.
    content            text        STORAGE MAIN NOT NULL,
    -- ACCESS_LOG_LINK.content_digest, computed by the writer.
    content_digest     bytea       STORAGE MAIN NOT NULL,
    ehr_id             uuid,
    party_id           text        STORAGE MAIN,
    retain_until       date,
    time_removed       timestamptz,
    -- A tombstone's end, after which it folds into a gap.
    fold_after         timestamptz,
    -- The coded action and result of the entry.
    action_terminology text        STORAGE MAIN,
    action_code        text        STORAGE MAIN,
    result_terminology text        STORAGE MAIN,
    result_code        text        STORAGE MAIN,
    -- The digest of the entry's shared part in access_log_shared.
    shared             bytea       STORAGE MAIN,
    -- The projection's index columns, for the ITI-81 search; NULL when the
    -- entry is not indexed.
    event_action       text        STORAGE MAIN,
    event_outcome      smallint,
    principal          text        STORAGE MAIN,
    patient_id         text        STORAGE MAIN,
    resource_id        text        STORAGE MAIN,
    request_id         text        STORAGE MAIN,
    client_ip          text        STORAGE MAIN,
    classified_ehr_id  text        STORAGE MAIN,
    search_tokens      integer[]   STORAGE MAIN,
    render_policy_id   integer,
    -- The number the intake and the link name the entry by.
    id                 bigint      NOT NULL GENERATED ALWAYS AS IDENTITY,
    CONSTRAINT pk_access_log_entry PRIMARY KEY (id),
    CONSTRAINT uq_access_log_entry_uid UNIQUE (uid),
    CONSTRAINT ck_access_log_entry_type
        CHECK (record_type IN ('ACCESS_LOG_ENTRY', 'ACCESS_LOG_TOMBSTONE')),
    CONSTRAINT ck_access_log_entry_removed
        CHECK ((time_removed IS NOT NULL) = (record_type = 'ACCESS_LOG_TOMBSTONE')),
    CONSTRAINT ck_access_log_entry_event_action
        CHECK (event_action IN ('C', 'R', 'U', 'D', 'E')),
    CONSTRAINT ck_access_log_entry_event_outcome CHECK (event_outcome IN (0, 4, 8)),
    -- An entry is indexed whole or not at all, and only an entry is.
    CONSTRAINT ck_access_log_entry_index
        CHECK ((render_policy_id IS NULL) = (event_action IS NULL)
               AND (render_policy_id IS NULL) = (event_outcome IS NULL)
               AND (render_policy_id IS NULL) = (search_tokens IS NULL)
               AND (render_policy_id IS NOT NULL
                    OR num_nonnulls(principal, patient_id, resource_id, request_id,
                                    client_ip, classified_ehr_id) = 0)
               AND (render_policy_id IS NULL OR record_type = 'ACCESS_LOG_ENTRY'))
) WITH (toast_tuple_target = 128);
-- A person's lookup by time reads this index in its order, the entry id breaking a
-- tie, as the chain position did.
CREATE INDEX ix_access_log_entry_ehr ON access_log_entry (ehr_id, time_recorded, id)
    WHERE ehr_id IS NOT NULL;
CREATE INDEX ix_access_log_entry_time ON access_log_entry (time_recorded)
    INCLUDE (render_policy_id, id);
CREATE INDEX ix_access_log_entry_fold_after ON access_log_entry (fold_after)
    WHERE fold_after IS NOT NULL;
-- Each index serves a read (#6202): ehr the person's trail and an AQL
-- condition on the EHR; time the ITI-81 page and total and the trail across
-- EHRs; fold_after the fold and retain_until the retention sweep; party the
-- party filter of the ITS-REST log and an AQL condition on the accessing
-- party; patient and principal the ITI-81 patient and agent, newest first;
-- tokens the ITI-81 token parameters, address, entity and an
-- entity.identifier answered from the request or resource id
-- (audit.search_keys), and the token retention.
CREATE INDEX ix_access_log_entry_party ON access_log_entry (party_id, time_recorded)
    WHERE party_id IS NOT NULL;
CREATE INDEX ix_access_log_entry_retain_until ON access_log_entry (retain_until)
    WHERE retain_until IS NOT NULL;
CREATE INDEX ix_access_log_entry_patient ON access_log_entry (patient_id, time_recorded)
    WHERE patient_id IS NOT NULL;
CREATE INDEX ix_access_log_entry_principal ON access_log_entry (principal, time_recorded)
    WHERE principal IS NOT NULL;
CREATE INDEX ix_access_log_entry_tokens
    ON access_log_entry USING gin (audit.search_keys(search_tokens, request_id, resource_id));
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
COMMENT ON COLUMN access_log_entry.record_type IS
    'The record''s class, the _type of content.';
COMMENT ON COLUMN access_log_entry.party_id IS
    'The id of the party accessor.party references, derived from content; NULL when it references none.';
COMMENT ON COLUMN access_log_entry.retain_until IS
    'The entry''s retention.retain_until, derived from content; NULL on a tombstone and when the entry states none.';
COMMENT ON COLUMN access_log_entry.time_removed IS
    'A tombstone''s time_removed, derived from content; NULL on an entry.';
COMMENT ON COLUMN access_log_entry.fold_after IS
    'A tombstone''s end: time_removed plus the time its entry was kept (time_removed minus time_recorded), derived from content; NULL on an entry. After it the tombstone folds into a gap.';
COMMENT ON COLUMN access_log_entry.action_terminology IS
    'The terminology id of the entry''s action.defining_code, derived from content; NULL on a tombstone.';
COMMENT ON COLUMN access_log_entry.action_code IS
    'The code_string of the entry''s action.defining_code, derived from content; NULL on a tombstone.';
COMMENT ON COLUMN access_log_entry.result_terminology IS
    'The terminology id of the entry''s outcome.result.defining_code, derived from content; NULL on a tombstone.';
COMMENT ON COLUMN access_log_entry.result_code IS
    'The code_string of the entry''s outcome.result.defining_code, derived from content; NULL on a tombstone.';
COMMENT ON COLUMN access_log_entry.shared IS
    'The digest of the entry''s shared part in access_log_shared; NULL on a tombstone and on a record that keeps its whole form on its row.';
COMMENT ON COLUMN access_log_entry.content IS
    'The record''s own part: its members the shared part does not hold and its own access details, canonicalised by RFC 8785; with the shared part joined back, its canonical form without its link.';
COMMENT ON COLUMN access_log_entry.uid IS
    'The record''s uid, derived from content.';
COMMENT ON COLUMN access_log_entry.content_digest IS
    'ACCESS_LOG_LINK.content_digest: the SHA-256 of the whole canonical form when the entry was written, computed by the writer; a tombstone keeps it.';
COMMENT ON COLUMN access_log_entry.ehr_id IS
    'The entry''s ehr_id when it is a UUID in its lower-case hyphenated form, which the row part then leaves out; NULL on a tombstone and for any other id, which the row part keeps.';
COMMENT ON COLUMN access_log_entry.event_action IS
    'The DICOM EventActionCode (PS3.15 A.5.1) the entry''s projection writes; NULL when the entry is not indexed.';
COMMENT ON COLUMN access_log_entry.event_outcome IS
    'The DICOM EventOutcomeIndicator (PS3.15 A.5.1) the entry''s projection writes; NULL when the entry is not indexed.';
COMMENT ON COLUMN access_log_entry.principal IS
    'The caller the entry''s projection names, for the ITI-81 search.';
COMMENT ON COLUMN access_log_entry.patient_id IS
    'The subject of care of the entry''s EHR, as the entry''s access details name it, for the ITI-81 search.';
COMMENT ON COLUMN access_log_entry.resource_id IS
    'The object the access reached, for the ITI-81 search.';
COMMENT ON COLUMN access_log_entry.request_id IS
    'The request the access belongs to, for the ITI-81 search.';
COMMENT ON COLUMN access_log_entry.client_ip IS
    'The caller''s network address, for the ITI-81 search.';
COMMENT ON COLUMN access_log_entry.classified_ehr_id IS
    'The EHR of a classified access, which its EHDS access record names; NULL for every other entry.';
COMMENT ON COLUMN access_log_entry.search_tokens IS
    'The ids of the search tokens of the entry''s projection, or of their shared set; NULL when the entry is not indexed.';
COMMENT ON COLUMN access_log_entry.render_policy_id IS
    'The id of the render policy the entry''s projection is written under; NULL on a tombstone and on an entry its writer indexes no projection of, which the ITI-81 search does not find.';
COMMENT ON TABLE access_log_entry IS
    'The openEHR access log''s records (draft org.openehr.rm.access_log 0.1.0), each an ACCESS_LOG_ENTRY or the ACCESS_LOG_TOMBSTONE that replaced one, written once by the request that made the access; its link is its access_log_link row, and access_log_record is the view of the linked ones. Append-only except the replacement of an entry by its tombstone.';
COMMENT ON COLUMN access_log_entry.id IS
    'The number the intake and the link name the entry by.';

-- ── the link ────────────────────────────────────────────────────────────────
CREATE TABLE access_log_link (
    chain_seq       bigint      NOT NULL,
    entry_id        bigint      NOT NULL,
    algorithm       text        NOT NULL,
    previous_digest bytea       STORAGE MAIN NOT NULL,
    digest          bytea       STORAGE MAIN NOT NULL,
    stored_at       timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_log_link PRIMARY KEY (chain_seq),
    CONSTRAINT uq_access_log_link_entry UNIQUE (entry_id),
    CONSTRAINT ck_access_log_link_algorithm CHECK (algorithm = 'SHA-256'),
    CONSTRAINT ck_access_log_link_digests
        CHECK (octet_length(previous_digest) = 32 AND octet_length(digest) = 32)
);
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
COMMENT ON TABLE access_log_link IS
    'The ACCESS_LOG_LINK of each chained access-log entry: its position, the digest of the link before it and its own digest, written by the chainer under the head of the log.';
COMMENT ON COLUMN access_log_link.chain_seq IS 'ACCESS_LOG_LINK.sequence.';
COMMENT ON COLUMN access_log_link.entry_id IS 'The access_log_entry the link chains.';
COMMENT ON COLUMN access_log_link.previous_digest IS 'ACCESS_LOG_LINK.previous_digest.';
COMMENT ON COLUMN access_log_link.digest IS
    'ACCESS_LOG_LINK.digest: the SHA-256 of previous_digest, a line feed, the decimal chain_seq, a line feed and the entry''s content_digest, each in lower-case hexadecimal.';
COMMENT ON COLUMN access_log_link.stored_at IS 'When the chainer linked the entry.';

CREATE TABLE access_log_checkpoint (
    sequence    bigint      NOT NULL,
    system_id   text        NOT NULL,
    digest      text        NOT NULL,
    time_sealed timestamptz NOT NULL,
    proof       text        NOT NULL,
    -- When the publication wrote the checkpoint outside the database, set
    -- once; the fold keeps a published checkpoint.
    published_at timestamptz,
    CONSTRAINT pk_access_log_checkpoint PRIMARY KEY (sequence),
    CONSTRAINT ck_access_log_checkpoint_digest CHECK (digest ~ '^[0-9a-f]{64}$'),
    CONSTRAINT ck_access_log_checkpoint_proof CHECK (proof <> '')
);
CREATE TRIGGER access_log_checkpoint_append_only
    BEFORE UPDATE OR DELETE ON access_log_checkpoint
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_checkpoint_reject_removal();
CREATE TRIGGER access_log_checkpoint_no_truncate
    BEFORE TRUNCATE ON access_log_checkpoint
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_checkpoint_append_only();
CREATE TRIGGER access_log_checkpoint_names_a_link
    BEFORE INSERT ON access_log_checkpoint
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_checkpoint_names_a_link();
COMMENT ON TABLE access_log_checkpoint IS
    'The signed ACCESS_LOG_CHECKPOINTs of this system''s chain, one per append that wrote a link while the keyring held a key, append-only.';
COMMENT ON COLUMN access_log_checkpoint.time_sealed IS
    'ACCESS_LOG_CHECKPOINT.time_sealed, to the microsecond the canonical form names.';
COMMENT ON COLUMN access_log_checkpoint.proof IS
    'ACCESS_LOG_CHECKPOINT.proof: the armored OpenPGP signature over the checkpoint''s canonical form without its proof.';
COMMENT ON COLUMN access_log_checkpoint.published_at IS
    'When the publication wrote the checkpoint outside the database, set once; the fold keeps a published checkpoint.';

-- Every update of the head row stays HOT with room on the page, and the
-- vacuum of a table that only ever holds one live row runs after a fixed
-- number of updates, so the row does not bloat.
CREATE TABLE access_log_chain_state (
    singleton   boolean     NOT NULL DEFAULT true,
    head_seq    bigint      NOT NULL DEFAULT 0,
    head_digest text        NOT NULL,
    updated_at  timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_log_chain_state PRIMARY KEY (singleton),
    CONSTRAINT ck_access_log_chain_state_singleton CHECK (singleton),
    CONSTRAINT ck_access_log_chain_state_digest CHECK (head_digest ~ '^[0-9a-f]{64}$')
) WITH (fillfactor = 10, autovacuum_vacuum_scale_factor = 0,
       autovacuum_vacuum_threshold = 1000);
INSERT INTO access_log_chain_state (singleton, head_seq, head_digest)
VALUES (true, 0, audit.access_log_genesis());
COMMENT ON TABLE access_log_chain_state IS
    'The head of this system''s access log: one row, written only by the SECURITY DEFINER chain triggers; its row lock, taken by the first insert of a transaction and held to its end, serialises the writers of the log.';

CREATE TABLE access_log_chain_verified (
    singleton        boolean     NOT NULL DEFAULT true,
    through_seq      bigint      NOT NULL,
    through_digest   text        NOT NULL,
    verified_at      timestamptz NOT NULL,
    full_verified_at timestamptz,
    CONSTRAINT pk_access_log_chain_verified PRIMARY KEY (singleton),
    CONSTRAINT ck_access_log_chain_verified_singleton CHECK (singleton)
);
COMMENT ON TABLE access_log_chain_verified IS
    'The newest access-log link a verification found intact, with every link before it: where an incremental verification of the access log starts. One row, written only by audit.verify_access_log_chain_step().';

-- ── the gaps ─────────────────────────────────────────────────────────────────
CREATE TABLE access_log_chain_gap (
    first_seq   bigint      NOT NULL,
    last_seq    bigint      NOT NULL,
    link        text        NOT NULL,
    grounds     jsonb       NOT NULL,
    time_folded timestamptz NOT NULL,
    -- The published checkpoints inside the folded run: for each, the
    -- position, the link the log held there and the SHA-256 of its proof.
    seals       jsonb       NOT NULL DEFAULT '[]'::jsonb,
    CONSTRAINT pk_access_log_chain_gap PRIMARY KEY (first_seq),
    CONSTRAINT uq_access_log_chain_gap_last UNIQUE (last_seq),
    CONSTRAINT ck_access_log_chain_gap_span CHECK (last_seq >= first_seq AND first_seq >= 1),
    CONSTRAINT ck_access_log_chain_gap_link CHECK (link ~ '^[0-9a-f]{64}$'),
    CONSTRAINT ck_access_log_chain_gap_grounds
        CHECK (jsonb_typeof(grounds) = 'array' AND jsonb_array_length(grounds) > 0),
    CONSTRAINT ck_access_log_chain_gap_seals CHECK (jsonb_typeof(seals) = 'array')
);
CREATE TRIGGER access_log_chain_gap_append_only
    BEFORE UPDATE OR DELETE ON access_log_chain_gap
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_chain_gap_append_only();
CREATE TRIGGER access_log_chain_gap_no_truncate
    BEFORE TRUNCATE ON access_log_chain_gap
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_chain_gap_append_only();
COMMENT ON TABLE access_log_chain_gap IS
    'The runs of expired tombstones removed from this system''s access log, one row per fold, append-only: the positions the run held and the link of its last record, which the record after it links to.';
COMMENT ON COLUMN access_log_chain_gap.link IS
    'ACCESS_LOG_LINK.digest of the run''s last record.';
COMMENT ON COLUMN access_log_chain_gap.grounds IS
    'Every distinct ground the run''s tombstones named, as DV_CODED_TEXT ITS-JSON, ordered.';
COMMENT ON COLUMN access_log_chain_gap.seals IS
    'Each published checkpoint inside the run, before its last position: its sequence, the link digest the log held there and the SHA-256 of its proof, in lower-case hexadecimal.';

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

-- The UUID p_text is in its lower-case hyphenated form; NULL for any other
-- text, which a uuid column could not give back as it was written.
CREATE FUNCTION audit.access_log_canonical_uuid(p_text text) RETURNS uuid
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    AS $$
    SELECT CASE WHEN p_text OPERATOR(pg_catalog.~)
                     '^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$'
                THEN p_text::pg_catalog.uuid END
$$;
COMMENT ON FUNCTION access_log_canonical_uuid(text) IS
    'A UUID written in its lower-case hyphenated form, as a uuid; NULL for any other text.';

-- The instant p_at as the writer leaves it out of a row part: UTC, a
-- four-digit year, the seconds and the microseconds without trailing zeros.
-- One expression that names p_at once, so the planner inlines it.
CREATE FUNCTION audit.access_log_instant_text(p_at timestamptz) RETURNS text
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT pg_catalog.rtrim(
               pg_catalog.rtrim(
                   pg_catalog.to_char(pg_catalog.timezone('UTC', p_at),
                                      'YYYY-MM-DD"T"HH24:MI:SS.US'),
                   '0'),
               '.') OPERATOR(pg_catalog.||) 'Z'
$$;
COMMENT ON FUNCTION access_log_instant_text(timestamptz) IS
    'An instant as an access-log row part leaves it out: UTC, the microseconds without trailing zeros, Z.';

CREATE FUNCTION audit.access_log_content_digest(p_content text) RETURNS text
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(p_content, 'UTF8')), 'hex')
$$;
COMMENT ON FUNCTION access_log_content_digest(text) IS
    'ACCESS_LOG_LINK.content_digest of a canonical form: the single definition the insert trigger and the verifier both call.';

-- No SET clause and every name by its pg_catalog name, so the planner
-- inlines it.
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
COMMENT ON FUNCTION access_log_row_part(jsonb, text, text) IS
    'An access-log row part with the members its request_id and resource_id columns hold given back: the channel''s request_id and the version_id of an only item that names no object, and without a member it names null.';

-- The document of an entry, with its row part read through
-- audit.access_log_row_part.
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
COMMENT ON FUNCTION access_log_document(text, text, text, uuid, timestamptz, uuid, text, text) IS
    'An access-log record as one jsonb, its canonical form without its link: the row part with the members its index columns hold, the shared part and the members the columns hold, the access details in the order of their codes. The SQL readers of a member both parts or the columns hold read it here; the server joins the parts in Rust.';

-- The columns of an entry derived from its content, with the index columns
-- the row part leaves members to.
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
COMMENT ON FUNCTION access_log_entry_derive(audit.access_log_entry) IS
    'The entry row with every column its content decides derived from the content again, which the tombstone trigger writes and the verifier compares.';

CREATE FUNCTION audit.access_log_link_digest(p_previous text, p_sequence bigint, p_content_digest text)
    RETURNS text
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
               p_previous OPERATOR(pg_catalog.||) E'\n'
               OPERATOR(pg_catalog.||) p_sequence::pg_catalog.text
               OPERATOR(pg_catalog.||) E'\n'
               OPERATOR(pg_catalog.||) p_content_digest, 'UTF8')), 'hex')
$$;
COMMENT ON FUNCTION access_log_link_digest(text, bigint, text) IS
    'ACCESS_LOG_LINK.digest: the single definition the insert trigger and the verifier both call.';

-- The link digest at p_sequence: the link's, or the gap's that ends there.
CREATE FUNCTION audit.access_log_link_at(p_sequence bigint) RETURNS text
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT coalesce(
        (SELECT encode(l.digest, 'hex') FROM audit.access_log_link l WHERE l.chain_seq = p_sequence),
        (SELECT g.link FROM audit.access_log_chain_gap g WHERE g.last_seq = p_sequence))
$$;
COMMENT ON FUNCTION access_log_link_at(bigint) IS
    'The link digest of the access log at a position: what a checkpoint of that position seals.';

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
COMMENT ON FUNCTION access_log_sealed_link(bigint, text) IS
    'The link a checkpoint at a position seals: the log''s link there, or the one a gap recorded for that checkpoint''s proof when it folded the position.';

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
COMMENT ON FUNCTION access_log_chain_move(bigint[], bigint[], bigint[], text[], text[], boolean, boolean) IS
    'The one path an access-log entry enters the chain by: its link at the position the chainer computed, its outbox rows, and the removal of the intake rows that named it, all of whose entries are linked in the same statement.';

-- The head of the access log, locked until the transaction ends, or no row
-- while another transaction holds it: the chainer of another replica.
CREATE FUNCTION audit.access_log_chain_take()
    RETURNS TABLE (head_seq bigint, head_digest text)
    LANGUAGE sql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT s.head_seq, s.head_digest
      FROM audit.access_log_chain_state s
     WHERE s.singleton
       FOR UPDATE SKIP LOCKED
$$;
COMMENT ON FUNCTION access_log_chain_take() IS
    'Lock the head of the access log without waiting and return its position and link digest; no row when another chainer holds it.';

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
COMMENT ON FUNCTION access_log_shared_collect(date, integer) IS
    'Remove up to p_limit shared parts past their retain_until that no entry names; the retention sweep calls it until it removes none.';

-- The end of the tombstone whose canonical form is p_content; NULL for an
-- entry.
CREATE FUNCTION audit.access_log_tombstone_end(p_content text) RETURNS timestamptz
    LANGUAGE plpgsql STABLE
    SET search_path = pg_catalog
    SET TimeZone = 'UTC'
    AS $$
DECLARE
    doc      jsonb := p_content::jsonb;
    removed  timestamptz;
    recorded timestamptz;
BEGIN
    IF doc ->> '_type' IS DISTINCT FROM 'ACCESS_LOG_TOMBSTONE' THEN
        RETURN NULL;
    END IF;
    removed := (doc #>> '{time_removed,value}')::timestamptz;
    recorded := (doc #>> '{time_recorded,value}')::timestamptz;
    RETURN removed + (removed - recorded);
END;
$$;

-- Folds the tombstones past their end into gaps, removing each folded
-- tombstone's link and entry.
CREATE FUNCTION audit.fold_access_log_tombstones(p_now timestamptz, p_limit integer)
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
COMMENT ON FUNCTION fold_access_log_tombstones(timestamptz, integer) IS
    'The one path a tombstone leaves the access log by: up to p_limit tombstones past their end fold into one access_log_chain_gap per run; a published checkpoint inside a run stays and the gap records the link it seals, an unpublished one leaves with the run.';

-- The link the next step starts from: the stored link at the end of the
-- window just verified.
CREATE FUNCTION audit.access_log_verification_link(p_sequence bigint)
    RETURNS text
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT CASE WHEN p_sequence = 0 THEN audit.access_log_genesis()
                ELSE audit.access_log_link_at(p_sequence) END
$$;
COMMENT ON FUNCTION access_log_verification_link(bigint) IS
    'The stored link at p_sequence, the genesis at 0: where the next step of a verification walk starts.';

-- Where a walk starts: the chain head it walks to, and the link it starts
-- from, the genesis for a full walk and the cursor otherwise. The head is
-- NULL when the chain state row is missing.
CREATE FUNCTION audit.access_log_verification_start(p_full boolean)
    RETURNS TABLE (head_seq bigint, after_seq bigint, after_digest text)
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT (SELECT s.head_seq FROM audit.access_log_chain_state s WHERE s.singleton),
           coalesce(c.through_seq, 0),
           coalesce(c.through_digest, audit.access_log_genesis())
      FROM (SELECT 1) AS one
      LEFT JOIN audit.access_log_chain_verified c ON c.singleton AND NOT p_full
$$;
COMMENT ON FUNCTION access_log_verification_start(boolean) IS
    'Where a verification walk of the access log starts: the chain head it walks to, and the verified link it starts from (the genesis when p_full).';

-- The end of one verification step, read from the links.
CREATE FUNCTION audit.access_log_verification_step_end(
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
COMMENT ON FUNCTION access_log_verification_step_end(bigint, integer, bigint) IS
    'Where the verification window after p_after ends: its p_records-th record, or p_head when fewer are left.';

-- One step of a walk: the window (p_after, p_upto] linked from p_after_digest,
-- with the entries the server found modified (p_modified) and, on the last
-- step (p_last), the head check. When the step finds nothing and p_advance,
-- the cursor moves to p_upto, never backwards; a full walk that reaches its
-- last step intact also records when the whole chain was verified.
CREATE FUNCTION audit.verify_access_log_chain_step(
    p_after bigint, p_after_digest text, p_upto bigint, p_modified bigint[],
    p_advance boolean, p_last boolean, p_full boolean)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    damage audit.audit_chain_finding[];
    link   text;
BEGIN
    SELECT coalesce(array_agg(f ORDER BY f.chain_seq NULLS FIRST), '{}')
      INTO damage
      FROM (
          SELECT * FROM audit.verify_access_log_chain_window(p_after, p_after_digest, p_upto, p_last)
          UNION ALL
          SELECT r.chain_seq, r.uid, r.time_recorded,
                 'record content was modified after it was written'::text
            FROM audit.access_log_record r
           WHERE r.chain_seq = ANY (coalesce(p_modified, '{}'))
             AND r.chain_seq > p_after AND r.chain_seq <= p_upto
      ) AS f;

    link := audit.access_log_link_at(p_upto);
    IF cardinality(damage) = 0 AND p_advance AND link IS NOT NULL THEN
        INSERT INTO audit.access_log_chain_verified AS c
               (through_seq, through_digest, verified_at, full_verified_at)
        VALUES (p_upto, link, now(), CASE WHEN p_full AND p_last THEN now() END)
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
COMMENT ON FUNCTION verify_access_log_chain_step(bigint, text, bigint, bigint[], boolean, boolean, boolean) IS
    'One bounded step of a verification walk of the access log: verifies (p_after, p_upto] from p_after_digest, the head on the last step, and moves the cursor to p_upto when the step is intact and p_advance.';

-- The verification of a window of the access log, with no derivation of an
-- entry's columns: the server reads each record's parts, joins them, and checks the content digest and every
-- derived column (system_log::store::chain_walk), and passes the positions
-- it found changed to the verification step.
CREATE FUNCTION audit.verify_access_log_chain_window(
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
COMMENT ON FUNCTION verify_access_log_chain_window(bigint, text, bigint, boolean) IS
    'The verification of the access log from one link on, up to and including position p_upto (to the end when NULL), with the head check when p_check_head: the body of the full verification and of each verification step.';

CREATE FUNCTION audit.verify_access_log_chain_after(p_after_seq bigint, p_after_digest text)
    RETURNS SETOF audit.audit_chain_finding
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT * FROM audit.verify_access_log_chain_window(p_after_seq, p_after_digest, NULL, true)
$$;
COMMENT ON FUNCTION verify_access_log_chain_after(bigint, text) IS
    'The verification of the access log from one verified link to the end, with the head check: the body of audit.verify_access_log_chain().';

CREATE FUNCTION audit.verify_access_log_chain()
    RETURNS TABLE (chain_seq bigint, record_id uuid, recorded_at timestamptz, finding text)
    LANGUAGE sql STABLE
    SET search_path = audit, pg_catalog
    AS $$
    SELECT f.chain_seq, f.record_id, f.recorded_at, f.finding
      FROM audit.verify_access_log_chain_after(0, audit.access_log_genesis()) f
$$;
COMMENT ON FUNCTION verify_access_log_chain() IS
    'Tamper check over the whole access log: one row per damaged record or boundary, and nothing at all when the log is intact.';

-- How far a chain's verification cursor is behind its head: the positions
-- past it and when the oldest record past it was recorded.
CREATE FUNCTION audit.chain_verification_lag(p_chain text)
    RETURNS TABLE (entries bigint, oldest timestamptz)
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT greatest(coalesce(s.head_seq, 0) - coalesce(c.through_seq, 0), 0),
           (SELECT r.time_recorded FROM audit.access_log_record r
             WHERE r.chain_seq > coalesce(c.through_seq, 0)
             ORDER BY r.chain_seq LIMIT 1)
      FROM (SELECT 1) AS one
      LEFT JOIN audit.access_log_chain_state s ON s.singleton
      LEFT JOIN audit.access_log_chain_verified c ON c.singleton
     WHERE p_chain = 'access_log'
    UNION ALL
    SELECT greatest(coalesce(s.head_seq, 0) - coalesce(c.through_seq, 0), 0),
           (SELECT e.recorded_at FROM audit.audit_event e
             WHERE e.chain_seq > coalesce(c.through_seq, 0)
             ORDER BY e.chain_seq LIMIT 1)
      FROM (SELECT 1) AS one
      LEFT JOIN audit.audit_chain_state s ON s.singleton
      LEFT JOIN audit.audit_chain_checkpoint c ON c.singleton
     WHERE p_chain = 'system'
$$;
COMMENT ON FUNCTION chain_verification_lag(text) IS
    'How far the verification cursor of the chain p_chain (system or access_log) is behind its head: the positions past it, and when the oldest record past it was recorded.';
