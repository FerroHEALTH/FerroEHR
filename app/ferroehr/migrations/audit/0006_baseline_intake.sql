-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the intakes and the syslog outbox.
--
-- Every audit record is written durably before the response that reports the
-- access: a request writes its access-log entries and every other audit
-- record in its own transaction, before it answers, into tables that are not
-- chained, so concurrent requests never wait on a head. The chainer links
-- them with a critical section that holds only what needs the head of the
-- log. A write's records are held under a reservation id until the write's
-- own transaction commits it; the reconciler releases or discards what a
-- process that died left held.
--
-- No openEHR spec governs audit storage mechanics: our own design.
--
-- Runs with search_path = audit, ext, public.

-- ── what the intake refuses ─────────────────────────────────────────────────
CREATE FUNCTION audit.audit_intake_reject_change() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF TG_OP = 'DELETE'
       AND coalesce(current_setting('ferroehr.audit_chaining', true), 'off') = 'on'
    THEN
        RETURN OLD;
    END IF;
    IF TG_OP = 'DELETE' AND OLD.reserved_by IS NOT NULL
       AND coalesce(current_setting('ferroehr.audit_discarding', true), 'off') = 'on'
    THEN
        RETURN OLD;
    END IF;
    IF TG_OP = 'UPDATE' AND OLD.reserved_by IS NOT NULL AND NEW.reserved_by IS NULL
       AND coalesce(current_setting('ferroehr.audit_releasing', true), 'off') = 'on'
       AND to_jsonb(NEW) - 'reserved_by' = to_jsonb(OLD) - 'reserved_by'
    THEN
        RETURN NEW;
    END IF;
    RAISE EXCEPTION 'audit.% is append-only: a record leaves it only by being chained', TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
END;
$$;

-- ── audit_event_intake ───────────────────────────────────────────────────────
-- The intake of the system chain: an audit record of no access-log entry is
-- written here, in its own transaction, before the response that reports it.
-- The table is not chained, so concurrent requests never wait on a head;
-- audit.audit_event_chain_move() moves each row into audit_event, whose own
-- triggers link it. An intake row is append-only: the runtime role may insert
-- and read it and nothing else, and the triggers below refuse every UPDATE,
-- DELETE and TRUNCATE. A write's records are held under a reservation id
-- (reserved_by) until the write's own transaction commits it
-- (audit_intake_release, audit_intake_discard). It carries the columns and
-- the checks of audit_event, less the stamps and links the chain fills, and
-- keeps the checks' names so a refusal reads the same in both tables.
--
-- vacuum_truncate is off: the chainer empties the table milliseconds after
-- each insert, and truncating its tail at every vacuum would take an ACCESS
-- EXCLUSIVE lock the intake writers queue behind (PostgreSQL 18, CREATE
-- TABLE, "vacuum_truncate").
CREATE TABLE audit_event_intake (
    id                     uuid        NOT NULL DEFAULT uuidv7(),
    -- The event time (FHIR AuditEvent.recorded / DICOM EventDateTime).
    recorded_at            timestamptz NOT NULL,
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
    -- The purposes of use and professional roles the access token states.
    token_attributes       jsonb,
    -- The intake's own key, when the row was taken, and the reservation
    -- that holds it.
    intake_id              bigint      NOT NULL GENERATED ALWAYS AS IDENTITY,
    taken_at               timestamptz NOT NULL DEFAULT now(),
    reserved_by            uuid,
    CONSTRAINT pk_audit_event_intake PRIMARY KEY (intake_id),
    CONSTRAINT uq_audit_event_intake_id UNIQUE (id),
    CONSTRAINT fk_audit_event_intake_policy FOREIGN KEY (render_policy)
        REFERENCES audit_render_policy (digest),
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
            'refused-scope-beyond-agreement', 'refused-correlation-missing'))
) WITH (vacuum_truncate = off);
CREATE INDEX ix_audit_event_intake_reserved ON audit_event_intake (reserved_by)
    WHERE reserved_by IS NOT NULL;
CREATE TRIGGER audit_event_intake_append_only
    BEFORE UPDATE OR DELETE ON audit_event_intake
    FOR EACH ROW EXECUTE FUNCTION audit.audit_intake_reject_change();
CREATE TRIGGER audit_event_intake_no_truncate
    BEFORE TRUNCATE ON audit_event_intake
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_intake_reject_change();
COMMENT ON TABLE audit_event_intake IS
    'The audit records of no access-log entry written before the response that reports them and not chained yet: append-only, left only by audit.audit_event_chain_move(), which writes each into the system chain.';

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
CREATE TRIGGER access_log_intake_append_only
    BEFORE UPDATE OR DELETE ON access_log_intake
    FOR EACH ROW EXECUTE FUNCTION audit.audit_intake_reject_change();
CREATE TRIGGER access_log_intake_no_truncate
    BEFORE TRUNCATE ON access_log_intake
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_intake_reject_change();
COMMENT ON TABLE access_log_intake IS
    'One row per intake statement: the access_log_entry ids it wrote, which no link chains yet. Append-only; left only by audit.access_log_chain_move(), which links its entries in the same statement, or by the discard of a reservation that never committed.';
COMMENT ON COLUMN access_log_intake.taken_at IS
    'When the request wrote the entries, for the age of the oldest not chained yet.';
COMMENT ON COLUMN access_log_intake.entries IS
    'The access_log_entry ids, in the order the chainer links them.';
COMMENT ON COLUMN access_log_intake.reserved_by IS
    'The reservation a write''s records are held under until its commit is confirmed (#5762); the chainer takes no held row. NULL for records that are not held.';

-- ── the syslog outbox ───────────────────────────────────────────────────────
CREATE TABLE syslog_outbox (
    id        bigint NOT NULL GENERATED ALWAYS AS IDENTITY,
    chain     text   NOT NULL,
    chain_seq bigint NOT NULL,
    CONSTRAINT pk_syslog_outbox PRIMARY KEY (id),
    CONSTRAINT uq_syslog_outbox_record UNIQUE (chain, chain_seq),
    CONSTRAINT ck_syslog_outbox_chain CHECK (chain IN ('system', 'access_log'))
);
COMMENT ON TABLE syslog_outbox IS
    'The records the ITI-20 syslog route still owes, each by its chain and position: written when the chainer stores the record while syslog is configured, removed when the repository accepted it. The frame is rendered from the stored record when it is read.';

-- Refuses an intake row naming a shared part or a render policy the store
-- does not hold, as the foreign keys did; called only for such a row. A part
-- the batch itself carried was removed by a tombstone between the statement
-- that stored it and the one that locks it: that is a serialization failure,
-- which the writer runs again (#5890), and the next run stores the part anew.
CREATE FUNCTION audit.access_log_intake_unknown(p_what text, p_digest text, p_removed boolean)
    RETURNS bytea
    LANGUAGE plpgsql VOLATILE
    SET search_path = pg_catalog
    AS $$
BEGIN
    IF p_removed THEN
        RAISE EXCEPTION 'the % % of an access-log intake row was removed while it was written', p_what, p_digest
            USING ERRCODE = 'serialization_failure';
    END IF;
    RAISE EXCEPTION 'an access-log intake row names the % %, which the store does not hold', p_what, p_digest
        USING ERRCODE = 'foreign_key_violation';
END;
$$;
COMMENT ON FUNCTION access_log_intake_unknown(text, text, boolean) IS
    'Raises foreign_key_violation for an intake row naming a shared part or a render policy the store does not hold, and serialization_failure for a part of the batch removed while it was written (#5887).';

-- Writes each entry once into access_log_entry and one intake row naming
-- them; a shared part's retain_until follows its entries'.
CREATE FUNCTION audit.access_log_intake_take(
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
        SELECT s.digest
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
    -- The raise is judged on the part's row, which a concurrent raise this
    -- statement waited on is read again at (READ COMMITTED), so it never
    -- writes an end the part already has (#6192).
    kept AS (
        UPDATE audit.access_log_shared s
           SET retain_until = n.until
          FROM needed n
          JOIN held h ON h.digest = n.digest
         WHERE s.digest = n.digest AND s.retain_until < n.until
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
COMMENT ON FUNCTION access_log_intake_take(text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], smallint[], text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[], text[], text[], text[], boolean[], uuid, bytea[]) IS
    'One batch''s access-log entries, each written once: their search tokens, found by the digests the server computed, grouped by entry in one pass, their shared sets and shared parts stored or found, checked and kept as long as the entries, their rows in access_log_entry, and one intake row naming them for the chainer (#5770, #5887, #5889, #5968).';

-- Releases the held intake rows of committed reservations, counting the
-- entries they name.
CREATE FUNCTION audit.audit_intake_release(p_ids uuid[]) RETURNS bigint
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
COMMENT ON FUNCTION audit_intake_release(uuid[]) IS
    'Release the records held under the reservations named: their writes committed, so the chainer takes them.';

-- Discards the held intake rows of reservations that never committed,
-- removing the entries they name, which no link chains.
CREATE FUNCTION audit.audit_intake_discard(p_ids uuid[]) RETURNS bigint
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
COMMENT ON FUNCTION audit_intake_discard(uuid[]) IS
    'Discard the records held under the reservations named, whose writes never committed.';
