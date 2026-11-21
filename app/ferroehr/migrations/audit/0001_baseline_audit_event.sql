-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the audit records of the system chain, their render policies and
-- their search tokens.
--
-- This is the v6.0.0 baseline of the audit set: every object of the schema is
-- created once, in its final shape, by the files 0001 to 0008. A later change
-- is a new file after them.
--
-- The audit schema is the local IHE ATNA Audit Record Repository, strictly
-- outside the EHR content (BASE architecture_overview master07-security.adoc
-- §Access logging: in-system access logs, never part of the EHR proper). A
-- record's FHIR R4 AuditEvent (IHE BALP shape) is rendered from its columns
-- under the render policy it names; its ITI-81 search tokens are shared rows
-- the record names by id.
--
-- No openEHR spec governs audit storage: our own design/extension.
--
-- Runs with search_path = audit, ext, public. The schema itself is created by
-- the server's bootstrap (crate::db).

COMMENT ON SCHEMA audit IS 'The local IHE ATNA Audit Record Repository (IHE ITI TF-1 §9): the node''s security-surveillance record, strictly outside the EHR content (BASE architecture_overview/master07-security.adoc §Access logging).';

-- Comparing the whole row minus the two delivery stamps fails CLOSED: a column
-- added later is immutable unless this exclusion list says otherwise.
CREATE FUNCTION audit.audit_event_reject_mutation() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF to_jsonb(NEW) - 'delivered_syslog_at' - 'delivered_fhir_feed_at'
       IS DISTINCT FROM
       to_jsonb(OLD) - 'delivered_syslog_at' - 'delivered_fhir_feed_at'
    THEN
        RAISE EXCEPTION
            'audit.audit_event is append-only: only delivered_syslog_at and delivered_fhir_feed_at may change (record %)',
            OLD.id
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN NEW;
END;
$$;

CREATE FUNCTION audit.audit_event_reject_deletion() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF coalesce(current_setting('ferroehr.audit_reaping', true), 'off') <> 'on' THEN
        RAISE EXCEPTION
            'audit.audit_event records are deleted only by audit.reap_audit_events_due() (record %)',
            OLD.id
            USING ERRCODE = 'restrict_violation';
    END IF;
    RETURN OLD;
END;
$$;

-- TRUNCATE bypasses row triggers entirely, so it needs its own statement-level
-- refusal (PostgreSQL 18 docs, CREATE TRIGGER:
-- https://www.postgresql.org/docs/18/sql-createtrigger.html).
CREATE FUNCTION audit.audit_event_reject_truncate() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.audit_event cannot be truncated: it is the tamper-evident audit trail'
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- The chain is written by three triggers, and the split is a throughput
-- decision rather than a stylistic one. Advancing the head by UPDATEing the
-- single state row PER RECORD costs quadratic time inside one transaction: each
-- update leaves another tuple version on the same page, and every subsequent
-- read walks the whole chain of them. Measured on PostgreSQL 18 over a 2000
-- record batch, that one statement was ~60% of the total insert time. So the
-- head is carried per STATEMENT in a transaction-local setting, and the state
-- row is read once at the start of a statement and written once at its end.
--
-- The setting is a cache, never an input: the BEFORE STATEMENT trigger
-- OVERWRITES it from the state row every time, so a caller who sets it by hand
-- has no effect on the chain a subsequent insert builds.
--
-- The FOR UPDATE on the state row is what makes the chain a total order:
-- concurrent inserting transactions serialize on it, and a rolled-back
-- transaction rolls its head advance back too, so chain_seq is gapless by
-- construction and a missing number means a deleted record.
--
-- SECURITY DEFINER on the two statement triggers so the runtime role can insert
-- records without holding any write privilege on the chain state (PostgreSQL 18
-- docs, CREATE FUNCTION §"Writing SECURITY DEFINER Functions Safely":
-- https://www.postgresql.org/docs/18/sql-createfunction.html).
CREATE FUNCTION audit.audit_event_chain_begin() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    head_position bigint;
    head_digest   bytea;
BEGIN
    SELECT head_seq, head_hash INTO head_position, head_digest
      FROM audit.audit_chain_state
     WHERE singleton
       FOR UPDATE;
    IF NOT FOUND THEN
        RAISE EXCEPTION 'audit chain state is missing: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    PERFORM set_config('ferroehr.audit_chain_head',
                       head_position || ':' || encode(head_digest, 'hex'), true);
    RETURN NULL;
END;
$$;

-- ── the chain writer ─────────────────────────────────────────────────────────
CREATE FUNCTION audit.audit_event_chain_link() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.audit_chain_head', true);
    preceding    smallint;
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RAISE EXCEPTION 'the audit chain cursor is unset: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW.chain_seq := split_part(cursor_value, ':', 1)::bigint + 1;
    NEW.prev_hash := decode(split_part(cursor_value, ':', 2), 'hex');
    SELECT e.digest_version INTO preceding
      FROM audit_event e
     WHERE e.chain_seq = NEW.chain_seq - 1;
    IF preceding IS NOT NULL AND NEW.digest_version < preceding THEN
        RAISE EXCEPTION 'audit record digest version % follows version %: refusing a downgrade',
            NEW.digest_version, preceding
            USING ERRCODE = 'restrict_violation';
    END IF;
    NEW.row_hash := audit.audit_event_row_digest(NEW);
    PERFORM set_config('ferroehr.audit_chain_head',
                       NEW.chain_seq || ':' || encode(NEW.row_hash, 'hex'), true);
    RETURN NEW;
END;
$$;

CREATE FUNCTION audit.audit_event_chain_commit() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.audit_chain_head', true);
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RETURN NULL;
    END IF;
    UPDATE audit.audit_chain_state
       SET head_seq = split_part(cursor_value, ':', 1)::bigint,
           head_hash = decode(split_part(cursor_value, ':', 2), 'hex'),
           updated_at = now()
     WHERE singleton;
    RETURN NULL;
END;
$$;

CREATE FUNCTION audit.audit_event_leave_outbox() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    DELETE FROM audit.fhir_feed_outbox o
     WHERE o.chain = 'system' AND o.chain_seq = OLD.chain_seq;
    DELETE FROM audit.syslog_outbox o
     WHERE o.chain = 'system' AND o.chain_seq = OLD.chain_seq;
    RETURN OLD;
END;
$$;
COMMENT ON FUNCTION audit_event_leave_outbox() IS
    'Removes a reaped audit_event record from the FHIR Feed''s outbox, as the foreign key that cascaded did.';

-- The statements the database runs inside the access-log writes, per
-- search token and per chained record, are taken out of the path (#5885).
-- The digest functions are inlined into the statements that call them, the
-- foreign keys the chain already holds are dropped, and the link check moves
-- the head cursor without a statement of its own.
--
-- PostgreSQL inlines a scalar SQL function into its caller only when the
-- function carries no SET clause and declares no stricter volatility than
-- its body has (PostgreSQL 18 docs, "Function Volatility Categories" and
-- "CREATE FUNCTION", the SET clause). Each function below
-- carried SET search_path = pg_catalog, and convert_to, jsonb_build_array and
-- the array output are STABLE, so none was inlined: at the capacity load the
-- search-token digest alone ran about 22 nested statements per request. Each
-- is redefined here with every name qualified by pg_catalog in place of the
-- SET clause and with the volatility of its body, STABLE. The digests
-- themselves are unchanged: the same bytes over the same input, which the
-- CHECK constraints and the verifiers still compare.
--
-- audit.audit_event_row_digest keeps its SET clauses: TimeZone and
-- bytea_output decide the text it hashes, so it cannot be inlined.
--
-- No openEHR spec governs audit storage mechanics: our own design.
CREATE FUNCTION audit.search_token_digest(p_parameter text, p_system text, p_code text)
    RETURNS bytea
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT pg_catalog.substr(pg_catalog.sha256(pg_catalog.convert_to(
               pg_catalog.jsonb_build_array(p_parameter, p_system, p_code)::pg_catalog.text,
               'UTF8')), 1, 16)
$$;
COMMENT ON FUNCTION search_token_digest(text, text, text) IS
    'The first 16 bytes of the SHA-256 over one search token: the ITI-81 parameter, the system and the code, NULL kept apart from the empty string.';

CREATE FUNCTION audit.search_set_digest(p_tokens integer[]) RETURNS bytea
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT pg_catalog.sha256(pg_catalog.convert_to(p_tokens::pg_catalog.text, 'UTF8'))
$$;
COMMENT ON FUNCTION search_set_digest(integer[]) IS
    'SHA-256 over the sorted token ids of one search set.';

CREATE FUNCTION audit.audit_render_policy_reject_change() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION 'audit.audit_render_policy is append-only: the records that name a policy are rendered under it'
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE FUNCTION audit.audit_search_reject_change() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    RAISE EXCEPTION '%.% is append-only: stored records name its rows', TG_TABLE_SCHEMA, TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- ── deleting a token ─────────────────────────────────────────────────────────
CREATE FUNCTION audit.audit_search_token_reject_change() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND coalesce(current_setting('ferroehr.audit_reaping', true), 'off') = 'on'
       AND NOT EXISTS (SELECT 1 FROM audit.audit_event e
                        WHERE e.search_tokens @> ARRAY[OLD.id])
       AND NOT EXISTS (SELECT 1 FROM audit.audit_search_set s
                        WHERE s.tokens @> ARRAY[OLD.id]) THEN
        RETURN OLD;
    END IF;
    RAISE EXCEPTION 'audit.audit_search_token is append-only: a token is deleted only by retention, once no record and no search set names it (token %)', OLD.id
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- ── the render policy ────────────────────────────────────────────────────────
CREATE TABLE audit_render_policy (
    -- SHA-256 over the policy's jsonb text, which jsonb writes canonically
    -- (sorted keys, no insignificant whitespace).
    digest    bytea       NOT NULL,
    policy    jsonb       NOT NULL,
    stored_at timestamptz NOT NULL DEFAULT now(),
    -- The number an access-log entry names its render policy by, in place of
    -- the digest.
    id        integer     NOT NULL GENERATED ALWAYS AS IDENTITY,
    CONSTRAINT pk_audit_render_policy PRIMARY KEY (digest),
    CONSTRAINT uq_audit_render_policy_id UNIQUE (id),
    CONSTRAINT ck_audit_render_policy_digest
        CHECK (digest = sha256(convert_to(policy::text, 'UTF8')))
);
CREATE TRIGGER audit_render_policy_reject_change
    BEFORE UPDATE OR DELETE ON audit_render_policy
    FOR EACH ROW EXECUTE FUNCTION audit.audit_render_policy_reject_change();
CREATE TRIGGER audit_render_policy_reject_truncate
    BEFORE TRUNCATE ON audit_render_policy
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_render_policy_reject_change();
COMMENT ON TABLE audit_render_policy IS
    'The configuration the FHIR AuditEvent of a record is rendered under (audit source, store horizon, category map, emergency purposes), one row per distinct configuration, keyed by the SHA-256 of its text; append-only.';
COMMENT ON COLUMN audit_render_policy.id IS
    'The number an access-log entry names its render policy by, in place of the digest.';

-- One row per distinct token. The digest is the key the writer deduplicates
-- on and what the verifier recomputes. The lookups hash the code and the
-- system, so a long identifier never meets the B-tree entry size limit, and
-- the addresses, short and matched by substring, are read from a partial index.
CREATE TABLE audit_search_token (
    id        integer GENERATED ALWAYS AS IDENTITY,
    -- The ITI-81 parameter: address, agent.identifier, entity.identifier,
    -- entity-role, entity-type, patient.identifier, source.identifier,
    -- subtype or type.
    parameter text    NOT NULL,
    -- The system of the coding or identifier; NULL when it names none.
    system    text,
    -- The code, the identifier value or the address; NULL when the element
    -- carries none.
    code      text,
    digest    bytea   NOT NULL,
    CONSTRAINT pk_audit_search_token PRIMARY KEY (id),
    CONSTRAINT uq_audit_search_token_digest UNIQUE (digest),
    CONSTRAINT ck_audit_search_token_digest
        CHECK (digest = audit.search_token_digest(parameter, system, code))
);
CREATE INDEX ix_audit_search_token_code
    ON audit_search_token (hashtextextended(code, 0));
CREATE INDEX ix_audit_search_token_system
    ON audit_search_token (hashtextextended(system, 0)) WHERE system IS NOT NULL;
CREATE INDEX ix_audit_search_token_address
    ON audit_search_token (code) INCLUDE (id) WHERE parameter = 'address';
CREATE TRIGGER audit_search_token_reject_truncate
    BEFORE TRUNCATE ON audit_search_token
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_search_reject_change();
CREATE TRIGGER audit_search_token_reject_change
    BEFORE UPDATE OR DELETE ON audit_search_token
    FOR EACH ROW EXECUTE FUNCTION audit.audit_search_token_reject_change();
COMMENT ON TABLE audit_search_token IS
    'Every distinct ITI-81 search token a stored record''s FHIR AuditEvent carries, once; append-only.';
COMMENT ON COLUMN audit_search_token.parameter IS
    'The ITI-81 parameter of the token, or entity.identifier=request_id or entity.identifier=resource_id for a token that says the document carries that column''s value as an entity.identifier in the token''s system.';

-- The tokens of type, subtype, entity-type, entity-role and source.identifier
-- take their values from code systems and the audit source, so their
-- combinations are few: a record names its combination by one set id.
CREATE TABLE audit_search_set (
    id     integer   GENERATED ALWAYS AS IDENTITY,
    -- The sorted ids of the set's tokens.
    tokens integer[] NOT NULL,
    digest bytea     NOT NULL,
    CONSTRAINT pk_audit_search_set PRIMARY KEY (id),
    CONSTRAINT uq_audit_search_set_tokens UNIQUE (tokens),
    CONSTRAINT ck_audit_search_set_digest CHECK (digest = audit.search_set_digest(tokens))
);
CREATE INDEX ix_audit_search_set_tokens ON audit_search_set USING gin (tokens);
CREATE TRIGGER audit_search_set_reject_change
    BEFORE UPDATE OR DELETE ON audit_search_set
    FOR EACH ROW EXECUTE FUNCTION audit.audit_search_reject_change();
CREATE TRIGGER audit_search_set_reject_truncate
    BEFORE TRUNCATE ON audit_search_set
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_search_reject_change();
COMMENT ON TABLE audit_search_set IS
    'Each distinct combination of the code-system search tokens of a record, once; append-only.';

-- ── audit_event ──────────────────────────────────────────────────────────────
-- One row per audit record. The FHIR R4 AuditEvent of the IHE BALP content
-- profiles, the document the RESTful-ATNA ITI-81 search serves, is rendered
-- from these columns under the record's render policy; nothing stores it.
--
-- Append-only: the only permitted mutations are the delivery stamp and
-- retention reaping, both enforced by the triggers below. chain_seq,
-- prev_hash and row_hash are the tamper-evidence hash chain, filled by the
-- insert trigger under the chain-state row lock.
CREATE TABLE audit_event (
    id                     uuid        NOT NULL DEFAULT uuidv7(),
    -- The event time (FHIR AuditEvent.recorded / DICOM EventDateTime).
    recorded_at            timestamptz NOT NULL,
    -- When the row was persisted (drain-side; ordering tiebreaker).
    stored_at              timestamptz NOT NULL DEFAULT now(),
    -- The DICOM EventActionCode (PS3.15 §A.5.1).
    action                 text        NOT NULL,
    -- The DICOM EventOutcomeIndicator (PS3.15 §A.5.1).
    outcome                smallint    NOT NULL,
    -- The DICOM EventID csd-code (110110/110112/110106/110107/110114/110100).
    event_code             text        NOT NULL,
    -- The concrete operation: the ITS-REST operation id, or the DCM
    -- login/logout EventTypeCode csd-code for authentication records.
    operation              text,
    -- The authenticated principal (Basic username / OAuth sub); NULL when the
    -- caller was unauthenticated.
    principal              text,
    -- The resolved EHR subject (patient) id, when the touched resource is
    -- patient-centric and resolution succeeded.
    patient_id             text,
    -- The resource class the operation touched (the event model's
    -- ObjectClass, lower-case).
    resource_class         text        NOT NULL,
    -- The touched resource id (version uid / template id / party uid /
    -- qualified stored-query name).
    resource_id            text,
    -- The requesting network address.
    client_ip              text,
    -- The bearer token jti (IHE BALP OAUTHaccessTokenUse.Minimal), never
    -- token contents.
    token_id               text,
    -- The delivery stamps: delivered_syslog_at is unused (the syslog sink
    -- delivers from its own spool), delivered_fhir_feed_at is when the FHIR
    -- Feed delivered the record (NULL = pending).
    delivered_syslog_at    timestamptz,
    delivered_fhir_feed_at timestamptz,
    -- The record's position in the hash chain (gapless), its predecessor's
    -- row_hash, and its own digest (audit.audit_event_row_digest).
    chain_seq              bigint      NOT NULL,
    prev_hash              bytea       NOT NULL,
    row_hash               bytea       NOT NULL,
    -- The pseudonymisation domain read or written.
    domain                 text,
    -- The purpose-of-use and legal-basis codes of the access.
    purpose                text,
    legal_basis            text,
    -- How many records the operation served, when it serves a countable set.
    result_count           bigint,
    -- The request correlation id (the x-request-id of the response).
    request_id             text,
    -- The roles the caller held, and the originating systems of the data
    -- served (EHDS Annex II 3.2(e)), capped, with their full count.
    roles                  jsonb,
    origins                jsonb,
    origin_count           bigint,
    -- The EHDS Art. 14(1) classification of the data: the categories, what
    -- the classification rests on, its evidence and the map in force.
    categories             jsonb,
    category_basis         text,
    category_evidence      jsonb,
    category_map_digest    text,
    -- How the credential acts, and its eIDAS level of assurance.
    acting_mode            text,
    assurance_level        text,
    assurance_value        text,
    professional_id        text,
    -- Whether the access was declared an emergency access (EHDS Art. 11(5)).
    emergency_access       boolean     NOT NULL DEFAULT false,
    -- The first date the record may be deleted on (EHDS Annex II 3.4), and
    -- whether the record states it.
    retention_ends         date,
    retention_stated       boolean     NOT NULL DEFAULT false,
    -- The data permit the caller declared it acts under (EHDS Art. 68).
    permit                 text,
    -- The token issuer and client the caller was authenticated by.
    issuer                 text,
    client_id              text,
    -- The EHR a classified access reached, the namespace of patient_id, and
    -- whether the subject lookup failed.
    ehr_id                 text,
    subject_namespace      text,
    subject_lookup_failed  boolean     NOT NULL DEFAULT false,
    -- The HL7_Confidentiality level the access is recorded for review at.
    review                 text        CHECK (review IN ('R', 'V')),
    -- What the category classification read.
    category_content       jsonb,
    -- The render policy the record is rendered under, and its ITI-81 search
    -- tokens (audit_search_token ids, its audit_search_set id negated).
    render_policy          bytea       NOT NULL,
    search_tokens          integer[]   NOT NULL,
    -- The outcome of the party binding and what it names.
    binding                text,
    party                  jsonb,
    party_groups           jsonb,
    provider               jsonb,
    guest                  boolean     NOT NULL DEFAULT false,
    representation         jsonb,
    external_fields        jsonb,
    agreement              jsonb,
    actor_chain            jsonb,
    facilities             jsonb,
    binding_detail         jsonb,
    summary                jsonb,
    -- The column list the row's chain digest hashes
    -- (audit.audit_event_digest_columns).
    digest_version         smallint    NOT NULL DEFAULT 5,
    -- The purposes of use and professional roles the access token states.
    token_attributes       jsonb,
    CONSTRAINT pk_audit_event PRIMARY KEY (id),
    CONSTRAINT uq_audit_event_chain_seq UNIQUE (chain_seq),
    CONSTRAINT fk_audit_event_render_policy
        FOREIGN KEY (render_policy) REFERENCES audit_render_policy (digest),
    CONSTRAINT ck_audit_event_action CHECK (action IN ('C', 'R', 'U', 'D', 'E')),
    CONSTRAINT ck_audit_event_outcome CHECK (outcome IN (0, 4, 8, 12)),
    CONSTRAINT ck_audit_event_domain
        CHECK (domain IS NULL OR domain IN ('clinical', 'party', 'linkage', 'system')),
    CONSTRAINT ck_audit_event_result_count
        CHECK (result_count IS NULL OR result_count >= 0),
    CONSTRAINT ck_audit_event_categories
        CHECK (categories IS NULL OR jsonb_typeof(categories) = 'array'),
    CONSTRAINT ck_audit_event_category_basis
        CHECK (category_basis IS NULL
               OR category_basis IN ('returned', 'written', 'queried', 'resource-kind',
                                     'template', 'archetype', 'query')),
    CONSTRAINT ck_audit_event_category_evidence
        CHECK (category_evidence IS NULL OR jsonb_typeof(category_evidence) = 'array'),
    -- A classification is the categories, the basis and the evidence together;
    -- a record never carries one without the others.
    CONSTRAINT ck_audit_event_category_whole
        CHECK ((categories IS NULL) = (category_basis IS NULL)
               AND (categories IS NULL) = (category_evidence IS NULL)),
    CONSTRAINT ck_audit_event_category_content
        CHECK ((category_content IS NULL) = (categories IS NULL)),
    CONSTRAINT ck_audit_event_acting_mode
        CHECK (acting_mode IS NULL
               OR acting_mode IN ('person', 'client', 'delegated', 'external', 'system')),
    CONSTRAINT ck_audit_event_assurance_level
        CHECK (assurance_level IS NULL
               OR assurance_level IN ('low', 'substantial', 'high')),
    CONSTRAINT ck_audit_event_retention_stated
        CHECK (retention_ends IS NULL OR retention_stated),
    CONSTRAINT ck_audit_event_binding
        CHECK (binding IS NULL OR binding IN (
            'bound', 'provisional', 'external', 'organisation', 'system', 'self',
            'refused-unbound', 'refused-ambiguous', 'refused-inactive', 'refused-abandoned',
            'refused-deprovisioned', 'refused-wrong-marker',
            'refused-representation-unrecorded', 'refused-key-claim-missing',
            'refused-groups-unavailable', 'refused-guest', 'refused-external-incomplete',
            'refused-trc-patient-mismatch', 'refused-trc-missing',
            'refused-system-personal-scope', 'refused-no-agreement',
            'refused-scope-beyond-agreement', 'refused-correlation-missing')),
    CONSTRAINT ck_audit_event_digest_version CHECK (digest_version IN (1, 2, 3, 4, 5))
);
CREATE INDEX ix_audit_event_retention_ends ON audit_event (retention_ends)
    WHERE retention_ends IS NOT NULL;
-- The token parameters match through this index. New entries go to the
-- pending list in one append and move into the posting trees in bulk; a
-- search reads the pending list whole, so its limit is 512 kB rather than the
-- 4 MB default (PostgreSQL 18, "GIN Fast Update Technique",
-- https://www.postgresql.org/docs/18/gin.html#GIN-FAST-UPDATE).
CREATE INDEX ix_audit_event_search_tokens ON audit_event USING gin (search_tokens)
    WITH (fastupdate = on, gin_pending_list_limit = 512);
-- entity matches resource_id through its hash, since a resource id can be a
-- stored query name longer than a B-tree entry holds.
CREATE INDEX ix_audit_event_resource_id ON audit_event (hashtextextended(resource_id, 0));
CREATE INDEX ix_audit_event_recorded_at ON audit_event (recorded_at);
CREATE INDEX ix_audit_event_patient ON audit_event (patient_id, recorded_at)
    WHERE patient_id IS NOT NULL;
CREATE INDEX ix_audit_event_principal ON audit_event (principal, recorded_at)
    WHERE principal IS NOT NULL;
-- An entity.identifier that carries the record's request id or resource id
-- is answered from that column (RESTful ATNA ITI-81 §3.81.4.1.2.2 searches
-- entity.identifier, which matches AuditEvent.entity.what.identifier): the
-- record names, in its shared search set, a token of the parameter
-- 'entity.identifier=request_id' or 'entity.identifier=resource_id' with the
-- system the document writes the identifier in and no code, and a search reads
-- the value from the column, through this index and ix_audit_event_resource_id.
-- A per-record token row for a value unique to its request would cost about
-- 200 bytes per audited request.
--
-- Retention: the reaping bodies collect the token ids the reaped records named,
-- and audit.release_search_tokens deletes those no remaining record and no
-- search set names. A token is deleted only inside reaping and only when nothing
-- names it, which the trigger checks itself; a token a live record names is
-- never deleted. A writer holds the token lock shared from storing its tokens to
-- its commit and the reaper holds it exclusively, so the reaper never deletes a
-- token a record in flight is about to name.
--
-- No openEHR or IHE specification governs how a repository indexes its records:
-- our own design.
CREATE INDEX ix_audit_event_request_id ON audit_event (hashtextextended(request_id, 0));
CREATE TRIGGER audit_event_chain_begin
    BEFORE INSERT ON audit_event
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_event_chain_begin();
CREATE TRIGGER audit_event_chain_link
    BEFORE INSERT ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit.audit_event_chain_link();
CREATE TRIGGER audit_event_chain_commit
    AFTER INSERT ON audit_event
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_event_chain_commit();
CREATE TRIGGER audit_event_reject_mutation
    BEFORE UPDATE ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit.audit_event_reject_mutation();
CREATE TRIGGER audit_event_reject_deletion
    BEFORE DELETE ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit.audit_event_reject_deletion();
CREATE TRIGGER audit_event_reject_truncate
    BEFORE TRUNCATE ON audit_event
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_event_reject_truncate();
CREATE TRIGGER audit_event_leave_outbox
    AFTER DELETE ON audit_event
    FOR EACH ROW EXECUTE FUNCTION audit.audit_event_leave_outbox();
COMMENT ON COLUMN audit_event.recorded_at IS 'The event time (FHIR AuditEvent.recorded / DICOM EventDateTime).';
COMMENT ON COLUMN audit_event.stored_at IS 'When the row was persisted (drain-side).';
COMMENT ON COLUMN audit_event.action IS 'DICOM EventActionCode (PS3.15 §A.5.1): C/R/U/D/E.';
COMMENT ON COLUMN audit_event.outcome IS 'DICOM EventOutcomeIndicator (PS3.15 §A.5.1): 0/4/8/12.';
COMMENT ON COLUMN audit_event.event_code IS 'DICOM EventID csd-code (PS3.15 §A.5.1).';
COMMENT ON COLUMN audit_event.operation IS 'The concrete operation: ITS-REST operation id, or the DCM login/logout code for authentication records.';
COMMENT ON COLUMN audit_event.principal IS 'The authenticated principal; NULL when unauthenticated.';
COMMENT ON COLUMN audit_event.patient_id IS 'The resolved EHR subject (patient) id, when patient-centric.';
COMMENT ON COLUMN audit_event.resource_class IS 'The touched resource class (the audit event model''s ObjectClass).';
COMMENT ON COLUMN audit_event.resource_id IS 'The touched resource id (version uid / template id / party uid / stored-query name).';
COMMENT ON COLUMN audit_event.client_ip IS 'The requesting network address.';
COMMENT ON COLUMN audit_event.token_id IS 'The bearer token jti (IHE BALP OAUTHaccessTokenUse.Minimal); never token contents.';
COMMENT ON COLUMN audit_event.delivered_fhir_feed_at IS 'When the FHIR-feed forwarding sink (ITI-20 ATX:FHIR Feed) delivered this record; NULL = pending.';
COMMENT ON COLUMN audit_event.chain_seq IS
    'Position in the tamper-evidence hash chain, assigned by the insert trigger under the chain-state row lock: gapless, so a missing number is a deleted record.';
COMMENT ON COLUMN audit_event.prev_hash IS
    'The row_hash of this record''s predecessor in the chain (the genesis digest for the first record).';
COMMENT ON COLUMN audit_event.domain IS
    'The pseudonymisation domain read or written: clinical | party | linkage | system (no domain). NULL when the operation names none.';
COMMENT ON COLUMN audit_event.purpose IS
    'The purpose-of-use code the caller declared, from the deployment allow-list; NULL when none was declared.';
COMMENT ON COLUMN audit_event.legal_basis IS
    'The legal basis code the deployment attributes to this access; NULL when the deployment configures none.';
COMMENT ON COLUMN audit_event.result_count IS
    'The number of records served (AQL result rows, search entries); NULL when the operation serves no countable set.';
COMMENT ON COLUMN audit_event.request_id IS
    'The request correlation id, matching the x-request-id header on the response.';
COMMENT ON COLUMN audit_event.roles IS
    'The roles the caller held at access time, as a JSON array of role names; NULL when no principal was authenticated and on records written before the column existed.';
COMMENT ON COLUMN audit_event.origins IS
    'The distinct originating systems of the data this access served, as a JSON array of system ids, capped; NULL when the operation served no version body or the record predates the column.';
COMMENT ON COLUMN audit_event.origin_count IS
    'The number of distinct origins the access served; greater than the array length when the array was capped.';
COMMENT ON COLUMN audit_event.category_map_digest IS
    'The digest of the [audit.categories] map in force when the record was written; NULL on records that predate the column.';
COMMENT ON COLUMN audit_event.assurance_level IS
    'The eIDAS level of assurance (low, substantial, high) the credential''s assurance claim maps to under [auth.oidc.assurance] levels; NULL when unmapped or not configured.';
COMMENT ON COLUMN audit_event.assurance_value IS
    'The raw assurance claim value (usually acr) the credential carried; NULL when absent.';
COMMENT ON COLUMN audit_event.professional_id IS
    'The identifier of the natural person who accessed the data (EHDS Annex II 3.2(b)): the person''s own claim, or the professional a client token acted for.';
COMMENT ON COLUMN audit_event.emergency_access IS
    'Whether the access was declared as an emergency access in the vital interest of the data subject (EHDS Art. 11(5)): the declared purpose of use is one of [audit] emergency_purpose_codes. Records the declaration only; no restriction is lifted by it.';
COMMENT ON COLUMN audit_event.categories IS
    'The EHDS Art. 14(1) categories of the data the access served or wrote (Annex II 3.2(c)), as a sorted JSON array: each category as its <system>|<code> token, none for data of no category, unclassified when the map could not classify all of it; records written before 0012 carry the earlier spellings; NULL when the operation touched no clinical content or the record predates the column.';
COMMENT ON COLUMN audit_event.category_basis IS
    'What the classification was read from: returned, written, queried or resource-kind; records written before 0012 carry template, archetype, query or resource-kind.';
COMMENT ON COLUMN audit_event.category_evidence IS
    'The template ids, archetype ids (in comparison form) and RM class names the classification read, as a JSON array, capped.';
COMMENT ON COLUMN audit_event.retention_ends IS
    'The first UTC date the record may be deleted on (Annex II 3.4): the access record''s own end, or the store horizon''s for any other record; NULL when it is kept without end or states no end.';
COMMENT ON COLUMN audit_event.retention_stated IS
    'Whether the record states its retention end; false for a record written before 0013, which the store horizon reaps.';
COMMENT ON COLUMN audit_event.delivered_syslog_at IS
    'Unused: the syslog sink delivers from its own spool and never stamps a record.';
COMMENT ON COLUMN audit_event.permit IS
    'The identifier of the data permit the caller declared it acts under (EHDS Art. 68), from [audit] permit_header; NULL when none was declared or the value was not an identifier.';
COMMENT ON TABLE audit_event IS
    'The local IHE ATNA Audit Record Repository: one row per audit record, whose FHIR R4 AuditEvent (IHE BALP shape) is rendered from the columns under the record''s render policy. Strictly outside the EHR content (BASE architecture_overview/master07-security.adoc §Access logging); append-only except per-sink delivery stamps and retention reaping.';
COMMENT ON COLUMN audit_event.issuer IS
    'The issuer (iss) of the bearer token that authenticated the caller; NULL for a Basic-authenticated or anonymous caller.';
COMMENT ON COLUMN audit_event.client_id IS
    'The client application the bearer token was issued to (client_id or azp); NULL when the token names none.';
COMMENT ON COLUMN audit_event.ehr_id IS
    'The EHR a classified access reached, which its EHDS access record names; NULL on every other record, so a linkage record never pairs a party with its EHR.';
COMMENT ON COLUMN audit_event.subject_namespace IS
    'The namespace of the subject id in patient_id, as EHR_STATUS.subject names it.';
COMMENT ON COLUMN audit_event.subject_lookup_failed IS
    'Whether the subject of ehr_id could not be looked up when the record was written.';
COMMENT ON COLUMN audit_event.review IS
    'The HL7_Confidentiality level (R or V) at which the access is recorded for review: a care relationship written to a record marked above N (draft access control, The Decision Procedure, Writing Relationships); NULL on every other record.';
COMMENT ON COLUMN audit_event.category_content IS
    'What the category classification read (the objects with their template and archetype ids, or the query constraints), from which the classification is computed again under the render policy''s category map; NULL exactly when categories is NULL.';
COMMENT ON COLUMN audit_event.render_policy IS
    'The digest of the audit_render_policy row the record is rendered under.';
COMMENT ON COLUMN audit_event.search_tokens IS
    'The ITI-81 search tokens of the record''s FHIR AuditEvent: the ids of its audit_search_token rows, and its audit_search_set id negated.';
COMMENT ON COLUMN audit_event.row_hash IS
    'SHA-256 over every column of the record but row_hash and the delivery stamps (audit.audit_event_row_digest); recomputed by audit.verify_audit_chain().';
COMMENT ON COLUMN audit_event.acting_mode IS
    'How the credential acts: a natural person (person), a client application for a person (client or delegated), a verified external accessor (external), or a client acting for nobody (system); NULL when the credential says nothing.';
COMMENT ON COLUMN audit_event.binding IS
    'The outcome of the party binding: how the caller was bound, or the refusal code; NULL on a record with no authenticated caller or one written before the column.';
COMMENT ON COLUMN audit_event.party IS
    'The demographic party the caller is bound to, as a PARTY_REF triple {namespace, type, id}.';
COMMENT ON COLUMN audit_event.party_groups IS
    'The ids of the GROUP parties the token names that the bound party belongs to.';
COMMENT ON COLUMN audit_event.provider IS
    'The healthcare provider the access is attributed to, {issuer, id} (EHDS Annex II 3.2(a)).';
COMMENT ON COLUMN audit_event.guest IS
    'Whether the bound caller is a guest its issuer admits (GDPR Art. 9(3), Art. 29).';
COMMENT ON COLUMN audit_event.representation IS
    'For a representative: whom they act for, on which asserted ground, and the subject_relationship code of their committer.';
COMMENT ON COLUMN audit_event.external_fields IS
    'The fields a federation token asserts about a verified external accessor, by field name.';
COMMENT ON COLUMN audit_event.agreement IS
    'The federation agreement an external accessor was admitted under: {name, version, legal_ground, accessor, access_class}, and for a chained request {correlation_id, correlation_source}.';
COMMENT ON COLUMN audit_event.actor_chain IS
    'The RFC 8693 actor chain of a delegated token, outermost first; recorded, never decided on.';
COMMENT ON COLUMN audit_event.facilities IS
    'The facility identifiers {issuer, id} of the compositions a system caller committed.';
COMMENT ON COLUMN audit_event.binding_detail IS
    'What a refusal names beside its code: the missing claim, the missing fields, the party, the asserted subject.';
COMMENT ON COLUMN audit_event.summary IS
    'For a summary record: the count of the refusals it stands for and the EHR ids they named.';
COMMENT ON COLUMN audit_event.digest_version IS
    'The column list the chain digest of this row hashes (audit.audit_event_digest_columns).';
COMMENT ON COLUMN audit_event.token_attributes IS
    'The purposes of use and the professional roles the access token states under its issuer''s [[auth.attributes]] table, {"purposes": [{system, code}], "roles": [{system, code}]}; NULL when it states none.';

CREATE FUNCTION audit.audit_event_digest_columns(p_version smallint) RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE p_version
        WHEN 1 THEN v1.columns
        WHEN 2 THEN v1.columns || v2.columns
        WHEN 3 THEN v1.columns || v2.columns || ARRAY['token_attributes']
        WHEN 4 THEN array_remove(v1.columns || v2.columns || ARRAY['token_attributes'],
                                 'organisation')
        WHEN 5 THEN array_remove(v1.columns || v2.columns || ARRAY['token_attributes', 'review'],
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
COMMENT ON FUNCTION audit_event_digest_columns(smallint) IS
    'The columns the chain digest of a version-p_version row hashes: every column but row_hash and the delivery stamps that version knows.';

CREATE FUNCTION audit.audit_event_row_digest(p_row audit.audit_event) RETURNS bytea
    LANGUAGE sql STABLE PARALLEL SAFE
    SET search_path = pg_catalog
    SET TimeZone = 'UTC'
    SET bytea_output = 'hex'
    AS $$
    SELECT sha256(convert_to(jsonb_build_array(
               'ferroehr.audit_chain'::text,
               jsonb_object_agg(listed.name, whole.doc -> listed.name)
           )::text, 'UTF8'))
      FROM to_jsonb(p_row) AS whole(doc),
           unnest(audit.audit_event_digest_columns(p_row.digest_version)) AS listed(name)
$$;
COMMENT ON FUNCTION audit_event_row_digest(audit.audit_event) IS
    'The chain digest of one stored record over the column list of its own digest_version: the single definition the insert trigger and the verifier both call.';

-- The runtime role registers the policy it writes under through this function
-- and holds no INSERT on the table, so it can add a policy and never alter one.
CREATE FUNCTION audit.register_render_policy(p_policy jsonb) RETURNS bytea
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    named bytea := sha256(convert_to(p_policy::text, 'UTF8'));
BEGIN
    INSERT INTO audit.audit_render_policy (digest, policy)
    VALUES (named, p_policy)
    ON CONFLICT (digest) DO NOTHING;
    RETURN named;
END;
$$;
COMMENT ON FUNCTION register_render_policy(jsonb) IS
    'Stores a render policy unless it is already stored, and returns its digest.';

CREATE FUNCTION audit.lock_search_tokens() RETURNS void
    LANGUAGE sql VOLATILE
    SET search_path = pg_catalog
    AS $$ SELECT pg_advisory_xact_lock(hashtext('audit.audit_search_token'), 0) $$;
COMMENT ON FUNCTION lock_search_tokens() IS
    'Holds the search tokens exclusively until the transaction ends: the retention side of audit.hold_search_tokens().';

-- The shared hold the writers take on the search tokens, inlined into the
-- statement that takes it.
CREATE FUNCTION audit.hold_search_tokens() RETURNS void
    LANGUAGE sql VOLATILE
    AS $$
    SELECT pg_catalog.pg_advisory_xact_lock_shared(pg_catalog.hashtext('audit.audit_search_token'), 0)
$$;
COMMENT ON FUNCTION hold_search_tokens() IS
    'Holds the search tokens shared until the transaction ends, so retention deletes none the transaction is about to name.';

-- Deletes the tokens among p_tokens that no record and no search set names.
-- Called by the reaping bodies with the reaping flag set.
CREATE FUNCTION audit.release_search_tokens(p_tokens integer[]) RETURNS bigint
    LANGUAGE sql VOLATILE
    SET search_path = audit, pg_catalog
    AS $$
    WITH released AS (
        DELETE FROM audit.audit_search_token t
         WHERE t.id = ANY (coalesce(p_tokens, '{}'))
           AND NOT EXISTS (SELECT 1 FROM audit.audit_event e
                            WHERE e.search_tokens @> ARRAY[t.id])
           AND NOT EXISTS (SELECT 1 FROM audit.audit_search_set s
                            WHERE s.tokens @> ARRAY[t.id])
        RETURNING 1
    )
    SELECT count(*) FROM released
$$;
COMMENT ON FUNCTION release_search_tokens(integer[]) IS
    'Deletes the search tokens among p_tokens that no record and no search set names; the reaping bodies call it with the token ids of the records they removed.';

CREATE FUNCTION audit.reap_audit_events_due(p_today date, p_unstated_cutoff timestamptz)
    RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    removed    bigint;
    released   integer[];
    left_from  bigint;
    right_from bigint;
BEGIN
    IF p_today IS NULL THEN
        RETURN 0;
    END IF;
    PERFORM audit.lock_search_tokens();
    PERFORM 1 FROM audit.audit_chain_state WHERE singleton FOR UPDATE;
    PERFORM set_config('ferroehr.audit_reaping', 'on', true);
    WITH gone AS (
        DELETE FROM audit.audit_event
         WHERE (retention_stated AND retention_ends IS NOT NULL AND retention_ends <= p_today)
            OR (NOT retention_stated AND p_unstated_cutoff IS NOT NULL
                AND recorded_at < p_unstated_cutoff)
        RETURNING chain_seq, row_hash, search_tokens
    ),
    islanded AS (
        SELECT chain_seq, row_hash,
               chain_seq - row_number() OVER (ORDER BY chain_seq) AS island
          FROM gone
    ),
    spans AS (
        SELECT min(chain_seq) AS from_seq, max(chain_seq) AS to_seq,
               (array_agg(row_hash ORDER BY chain_seq DESC))[1] AS link_hash
          FROM islanded GROUP BY island
    ),
    recorded AS (
        INSERT INTO audit.audit_chain_gap (from_seq, to_seq, link_hash)
        SELECT from_seq, to_seq, link_hash FROM spans
        RETURNING 1
    )
    SELECT (SELECT count(*) FROM gone),
           (SELECT array_agg(DISTINCT t.id) FROM gone g, unnest(g.search_tokens) AS t(id)
             WHERE t.id > 0)
      INTO removed, released;
    PERFORM audit.release_search_tokens(released);
    PERFORM set_config('ferroehr.audit_reaping', 'off', true);
    LOOP
        SELECT g1.from_seq, g2.from_seq INTO left_from, right_from
          FROM audit.audit_chain_gap g1
          JOIN audit.audit_chain_gap g2 ON g2.from_seq = g1.to_seq + 1
         LIMIT 1;
        EXIT WHEN left_from IS NULL;
        UPDATE audit.audit_chain_gap merged
           SET to_seq = absorbed.to_seq, link_hash = absorbed.link_hash
          FROM audit.audit_chain_gap absorbed
         WHERE merged.from_seq = left_from AND absorbed.from_seq = right_from;
        DELETE FROM audit.audit_chain_gap WHERE from_seq = right_from;
    END LOOP;
    RETURN coalesce(removed, 0);
END;
$$;
COMMENT ON FUNCTION reap_audit_events_due(date, timestamptz) IS
    'Retention reaping by each record''s own end: removes every record whose end is on or before p_today, and every record that states no end and is older than p_unstated_cutoff (NULL keeps those), tombstoning the chain positions they occupied.';
