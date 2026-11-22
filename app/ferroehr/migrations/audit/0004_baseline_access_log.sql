-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the openEHR access log (draft org.openehr.rm.access_log 0.1.0).
--
-- Each entry is written once by the request that made the access, and read,
-- sealed and delivered from that one row (#6242). The sealer closes windows
-- of entries by id up to the frontier and signs each window's seal. A
-- retention sweep replaces an entry past its end by its tombstone, which keeps
-- its content digest, and the fold removes a sealed window whose tombstones
-- are all past their end. The verifier recomputes each window from its rows.
--
-- No openEHR spec governs audit storage: our own design. The access log
-- package it realizes is a draft, which is no openEHR release.
--
-- Runs with search_path = audit, ext, public.

-- An entry leaves only as a tombstone of a folded window, or while held, when
-- the reservation it is held under never committed.
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
       AND EXISTS (SELECT 1 FROM audit.access_log_hold h WHERE h.entry_id = OLD.id)
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

CREATE FUNCTION audit.access_log_seal_reject_change() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'UPDATE' AND TG_TABLE_NAME = 'access_log_seal'
       AND OLD.published_at IS NULL AND NEW.published_at IS NOT NULL
       AND (NEW.window_no, NEW.first_id, NEW.last_id, NEW.entries, NEW.previous_digest,
            NEW.digest, NEW.sealed_at, NEW.proof, NEW.system_id)
           IS NOT DISTINCT FROM (OLD.window_no, OLD.first_id, OLD.last_id, OLD.entries,
                                 OLD.previous_digest, OLD.digest, OLD.sealed_at, OLD.proof,
                                 OLD.system_id)
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'audit.% is append-only: a seal is marked published once and never removed', TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- ── the seals ────────────────────────────────────────────────────────────────
-- What the first sealed window follows: the SHA-256 of the seal text's
-- version tag (access_log::seal).
CREATE FUNCTION audit.access_log_seal_genesis() RETURNS bytea
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT sha256(convert_to('ferroehr.access_log.seal.v1', 'UTF8')) $$;
COMMENT ON FUNCTION access_log_seal_genesis() IS
    'The previous digest of the first sealed window of the access log.';

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
-- with every column and index it keeps, and every reader, the sealer and the
-- delivery read it here (#6242); access_log_record is the view of the entries
-- no write holds. Append-only except the replacement of an entry by its
-- tombstone.
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
    -- The SHA-256 of the whole canonical form, computed by the writer.
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
    -- The number the seals, holds and delivery cursors name the entry by.
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
    'The SHA-256 of the whole canonical form when the entry was written, computed by the writer; a tombstone keeps it, and its window''s seal covers it.';
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
    'The openEHR access log''s records (draft org.openehr.rm.access_log 0.1.0), each an ACCESS_LOG_ENTRY or the ACCESS_LOG_TOMBSTONE that replaced one, written once by the request that made the access and read, sealed and delivered from this row; access_log_record is the view of those no write holds. Append-only except the replacement of an entry by its tombstone.';
COMMENT ON COLUMN access_log_entry.id IS
    'The number the seals, the holds and the delivery cursors name the entry by, drawn after its transaction took its xid.';

-- ── the holds ────────────────────────────────────────────────────────────────
-- A write's entries are held until its commit is confirmed (#5762): written in
-- the entries' own statement, removed by the release once the write's
-- transaction committed, or with the entries by the discard of one that never
-- did. A held entry is in no read, no seal and no delivery.
CREATE TABLE access_log_hold (
    entry_id    bigint      NOT NULL,
    reserved_by uuid        NOT NULL,
    taken_at    timestamptz NOT NULL DEFAULT now(),
    CONSTRAINT pk_access_log_hold PRIMARY KEY (entry_id)
);
CREATE INDEX ix_access_log_hold_reserved ON access_log_hold (reserved_by);
COMMENT ON TABLE access_log_hold IS
    'The access-log entries a write holds until its commit is confirmed (#5762); removed by the release, or with its entries by the discard.';

-- ── the seals ────────────────────────────────────────────────────────────────
-- One row per sealed window: every entry id from first_id to last_id, the
-- window after the one before it with no id between, and the SHA-256 of the
-- text access_log::seal documents, signed when the keyring holds a key.
CREATE TABLE access_log_seal (
    window_no       bigint      NOT NULL,
    first_id        bigint      NOT NULL,
    last_id         bigint      NOT NULL,
    entries         integer     NOT NULL,
    previous_digest bytea       NOT NULL,
    digest          bytea       NOT NULL,
    sealed_at       timestamptz NOT NULL,
    proof           text,
    system_id       text,
    published_at    timestamptz,
    CONSTRAINT pk_access_log_seal PRIMARY KEY (window_no),
    CONSTRAINT uq_access_log_seal_last UNIQUE (last_id),
    CONSTRAINT ck_access_log_seal_span CHECK (last_id >= first_id AND first_id >= 1),
    CONSTRAINT ck_access_log_seal_entries CHECK (entries >= 0),
    CONSTRAINT ck_access_log_seal_digests
        CHECK (octet_length(previous_digest) = 32 AND octet_length(digest) = 32),
    CONSTRAINT ck_access_log_seal_proof
        CHECK ((proof IS NULL) = (system_id IS NULL) AND (proof IS NULL OR proof <> ''))
);
CREATE TRIGGER access_log_seal_append_only
    BEFORE UPDATE OR DELETE ON access_log_seal
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_seal_reject_change();
CREATE TRIGGER access_log_seal_no_truncate
    BEFORE TRUNCATE ON access_log_seal
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_seal_reject_change();
COMMENT ON TABLE access_log_seal IS
    'The sealed windows of the access log, append-only: each covers every entry id from first_id to last_id and follows the window before it.';
COMMENT ON COLUMN access_log_seal.digest IS
    'The SHA-256 of the window''s text (access_log::seal): the version tag, the window''s fields, the previous digest, and each entry''s id and content digest in id order.';
COMMENT ON COLUMN access_log_seal.proof IS
    'The armored OpenPGP signature of the ACCESS_LOG_CHECKPOINT of the log up to last_id over digest, named by system_id and sealed_at; NULL when the keyring held no key.';

-- The sealer's place: the last entry id and window sealed and the window's
-- digest. Its row lock makes one sealer run at a time across replicas.
CREATE TABLE access_log_seal_state (
    singleton boolean NOT NULL DEFAULT true,
    last_id   bigint  NOT NULL DEFAULT 0,
    window_no bigint  NOT NULL DEFAULT 0,
    digest    bytea   NOT NULL,
    CONSTRAINT pk_access_log_seal_state PRIMARY KEY (singleton),
    CONSTRAINT ck_access_log_seal_state_singleton CHECK (singleton)
);
INSERT INTO access_log_seal_state (singleton, last_id, window_no, digest)
VALUES (true, 0, 0, audit.access_log_seal_genesis());

-- A window whose tombstones were all past their end, removed by the fold; its
-- seal stays, and the verifier takes the window as folded.
CREATE TABLE access_log_seal_fold (
    window_no   bigint      NOT NULL,
    grounds     jsonb       NOT NULL,
    time_folded timestamptz NOT NULL,
    CONSTRAINT pk_access_log_seal_fold PRIMARY KEY (window_no),
    CONSTRAINT ck_access_log_seal_fold_grounds
        CHECK (jsonb_typeof(grounds) = 'array' AND jsonb_array_length(grounds) > 0)
);
CREATE TRIGGER access_log_seal_fold_append_only
    BEFORE UPDATE OR DELETE ON access_log_seal_fold
    FOR EACH ROW EXECUTE FUNCTION audit.access_log_seal_reject_change();
CREATE TRIGGER access_log_seal_fold_no_truncate
    BEFORE TRUNCATE ON access_log_seal_fold
    FOR EACH STATEMENT EXECUTE FUNCTION audit.access_log_seal_reject_change();
COMMENT ON TABLE access_log_seal_fold IS
    'The sealed windows the fold removed, one row each, append-only: the grounds of their tombstones.';

CREATE TABLE access_log_seal_verified (
    singleton        boolean     NOT NULL DEFAULT true,
    through_window   bigint      NOT NULL,
    verified_at      timestamptz NOT NULL,
    full_verified_at timestamptz,
    CONSTRAINT pk_access_log_seal_verified PRIMARY KEY (singleton),
    CONSTRAINT ck_access_log_seal_verified_singleton CHECK (singleton)
);
COMMENT ON TABLE access_log_seal_verified IS
    'The newest window a verification found intact with every window before it: where an incremental verification starts.';

-- The entries no write holds, as every reader of the log reads them; an
-- entry is named by its id.
CREATE VIEW access_log_record AS
    SELECT e.id AS chain_seq, e.record_type, e.uid, e.time_recorded, e.content,
           e.content_digest, e.ehr_id, e.party_id,
           e.retain_until, e.time_removed, e.fold_after, e.action_terminology, e.action_code,
           e.result_terminology, e.result_code, e.shared, e.event_action, e.event_outcome,
           e.principal, e.patient_id, e.resource_id, e.request_id, e.client_ip,
           e.classified_ehr_id, e.search_tokens, e.render_policy_id, e.id AS entry_id
      FROM access_log_entry e
     WHERE NOT EXISTS (SELECT 1 FROM access_log_hold h WHERE h.entry_id = e.id);
COMMENT ON VIEW access_log_record IS
    'The openEHR access log (draft org.openehr.rm.access_log 0.1.0): every ACCESS_LOG_ENTRY or ACCESS_LOG_TOMBSTONE no write holds, named by its id. Every reader of the log reads it here.';

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
    'The content digest of a canonical form: the single definition the writer''s check and the verifier both call.';

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
-- Remove up to p_limit shared parts whose retain_until is more than a day
-- before p_today and the database's own date and that no entry names. An
-- entry that names a part has a retain_until at or before the part's, or the
-- part's is infinity, so the check reads only the entries the retention sweep
-- has not tombstoned yet, through the retain_until index. The intake keeps a
-- part it names until at least its own date and takes no lock on it (#6241):
-- the day of grace covers an intake statement that saw the part before
-- midnight and commits after it, and an intake that has to raise a part this
-- removes finds it gone and is refused as a serialization failure.
CREATE FUNCTION audit.access_log_shared_collect(p_today date, p_limit integer) RETURNS bigint
    LANGUAGE sql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    WITH due AS (
        SELECT s.digest, s.retain_until
          FROM audit.access_log_shared s
         WHERE s.retain_until < least(p_today, current_date) - 1
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

-- Removes the entries of the sealed windows whose records are all tombstones
-- past their end, oldest window first, up to p_limit windows, recording each
-- as folded with the grounds its tombstones named.
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
    PERFORM pg_advisory_xact_lock(hashtext('audit.access_log_seal_fold'), 0);
    PERFORM set_config('ferroehr.access_log_folding', 'on', true);
    RETURN QUERY
    WITH expired AS (
        SELECT e.id
          FROM audit.access_log_entry e
         WHERE e.fold_after <= p_now
         ORDER BY e.fold_after
         LIMIT p_limit * 64
    ),
    candidates AS (
        SELECT DISTINCT w.window_no, w.first_id, w.last_id, w.entries
          FROM expired x
         CROSS JOIN LATERAL (SELECT s.window_no, s.first_id, s.last_id, s.entries
                               FROM audit.access_log_seal s
                              WHERE s.last_id >= x.id
                              ORDER BY s.last_id LIMIT 1) AS w
         WHERE w.first_id <= x.id
    ),
    due AS (
        SELECT c.window_no, c.first_id, c.last_id
          FROM candidates c
         WHERE c.entries > 0
           AND NOT EXISTS (SELECT 1 FROM audit.access_log_seal_fold f
                            WHERE f.window_no = c.window_no)
           AND (SELECT count(*) FROM audit.access_log_entry e
                 WHERE e.id BETWEEN c.first_id AND c.last_id) = c.entries
           AND NOT EXISTS (SELECT 1 FROM audit.access_log_entry e
                            WHERE e.id BETWEEN c.first_id AND c.last_id
                              AND (e.fold_after IS NULL OR e.fold_after > p_now))
         ORDER BY c.window_no
         LIMIT p_limit
    ),
    grounded AS (
        SELECT d.window_no,
               coalesce(jsonb_agg(DISTINCT g.ground ORDER BY g.ground)
                        FILTER (WHERE g.ground IS NOT NULL), '[]'::jsonb) AS grounds
          FROM due d
          LEFT JOIN audit.access_log_entry e ON e.id BETWEEN d.first_id AND d.last_id
          LEFT JOIN LATERAL jsonb_array_elements(e.content::jsonb -> 'grounds') AS g(ground) ON true
         GROUP BY d.window_no
    ),
    written AS (
        INSERT INTO audit.access_log_seal_fold (window_no, grounds, time_folded)
        SELECT g.window_no, g.grounds, p_now
          FROM grounded g
         WHERE jsonb_array_length(g.grounds) > 0
         ORDER BY g.window_no
        RETURNING 1
    ),
    removed AS (
        DELETE FROM audit.access_log_entry e
         USING due d
         WHERE e.id BETWEEN d.first_id AND d.last_id
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM removed), (SELECT count(*) FROM written);
    PERFORM set_config('ferroehr.access_log_folding', 'off', true);
END;
$$;
COMMENT ON FUNCTION fold_access_log_tombstones(timestamptz, integer) IS
    'The one path a tombstone leaves the access log by: up to p_limit sealed windows whose records are all tombstones past their end lose their entries, each recorded in access_log_seal_fold with the grounds its tombstones named; the seals stay.';

-- How far the verification is behind the sealer and the newest records: the
-- entries past the last verified window and when the oldest of them was
-- recorded.
CREATE FUNCTION audit.chain_verification_lag(p_chain text)
    RETURNS TABLE (entries bigint, oldest timestamptz)
    LANGUAGE sql STABLE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
    SELECT (SELECT count(*) FROM audit.access_log_entry e WHERE e.id > b.through_id),
           (SELECT e.time_recorded FROM audit.access_log_entry e
             WHERE e.id > b.through_id ORDER BY e.id LIMIT 1)
      FROM (SELECT coalesce((SELECT s.last_id FROM audit.access_log_seal s
                              JOIN audit.access_log_seal_verified v
                                ON v.singleton AND s.window_no = v.through_window), 0)
                   AS through_id) AS b
     WHERE p_chain = 'access_log'
    UNION ALL
    SELECT greatest(coalesce(s.head_seq, 0) - coalesce(c.through_seq, 0), 0),
           (SELECT e.recorded_at FROM audit.audit_event_link l
              JOIN audit.audit_event e ON e.id = l.event_id
             WHERE l.chain_seq > coalesce(c.through_seq, 0)
             ORDER BY l.chain_seq LIMIT 1)
      FROM (SELECT 1) AS one
      LEFT JOIN audit.audit_chain_state s ON s.singleton
      LEFT JOIN audit.audit_chain_checkpoint c ON c.singleton
     WHERE p_chain = 'system'
$$;

