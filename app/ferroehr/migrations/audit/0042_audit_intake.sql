-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: every audit record is written durably before the response that
-- reports the access (#5747), and the chainer links it into its chain with a
-- critical section that holds only what needs the head of the log (#5745).
--
-- The intake: a request writes its access-log entries into access_log_intake
-- and every other audit record into audit_event_intake, in its own
-- transaction, before it answers. Neither table is chained, so concurrent
-- requests never wait on a head. The writer supplies every column, the
-- derived ones and the entry's index row included, which the server built
-- a moment earlier.
--
-- The chainer: access_log_chain_take() locks the head of the access log
-- without waiting (SKIP LOCKED), so any replica may chain and one does at a
-- time; the server computes the links, and access_log_chain_move() inserts
-- the chained records, their index rows and the outbox rows of the FHIR Feed
-- and syslog, and removes the intake rows, in the transaction that also
-- writes the head and the signed checkpoint. audit_event_chain_move() moves
-- the other records into the system chain, whose own triggers link them.
-- The insert trigger of the access log no longer derives the columns from
-- the content: it checks the link against the head, and the verifier keeps
-- deriving every column again and comparing (0039).
--
-- The window before chaining: an intake row is append-only. The runtime role
-- may insert and read it and nothing else, the tables refuse every UPDATE,
-- DELETE and TRUNCATE, and only the two SECURITY DEFINER moves delete a row,
-- each by writing it into its chain in the same statement. Someone with the
-- owner's rights can still remove an intake row before it is chained, and the
-- chain does not show it; the chainer runs milliseconds after each insert.
--
-- syslog becomes a projection of the stored records, delivered from
-- syslog_outbox as the FHIR Feed is from fhir_feed_outbox.
--
-- No openEHR spec governs audit storage mechanics: our own design.
--
-- Runs with search_path = audit, ext, public.

-- ── the intake of the access log ────────────────────────────────────────────

CREATE TABLE access_log_intake (
    id                 bigint      NOT NULL GENERATED ALWAYS AS IDENTITY,
    taken_at           timestamptz NOT NULL DEFAULT now(),
    record_type        text        NOT NULL,
    uid                uuid        NOT NULL,
    time_recorded      timestamptz NOT NULL,
    content            text        NOT NULL,
    shared             bytea,
    content_digest     bytea       NOT NULL,
    ehr_id             text,
    party_id           text,
    retain_until       date,
    action_terminology text,
    action_code        text,
    result_terminology text,
    result_code        text,
    recorded_at        timestamptz,
    action             text,
    outcome            smallint,
    principal          text,
    patient_id         text,
    resource_id        text,
    request_id         text,
    client_ip          text,
    index_ehr_id       text,
    search_tokens      integer[],
    render_policy      bytea,
    CONSTRAINT pk_access_log_intake PRIMARY KEY (id),
    CONSTRAINT uq_access_log_intake_uid UNIQUE (uid),
    CONSTRAINT ck_access_log_intake_type CHECK (record_type = 'ACCESS_LOG_ENTRY'),
    CONSTRAINT ck_access_log_intake_digest CHECK (octet_length(content_digest) = 32),
    CONSTRAINT ck_access_log_intake_index
        CHECK ((render_policy IS NULL) = (recorded_at IS NULL)
               AND (render_policy IS NULL) = (action IS NULL)
               AND (render_policy IS NULL) = (outcome IS NULL)
               AND (render_policy IS NULL) = (search_tokens IS NULL)),
    CONSTRAINT ck_access_log_intake_action CHECK (action IN ('C', 'R', 'U', 'D', 'E')),
    CONSTRAINT ck_access_log_intake_outcome CHECK (outcome IN (0, 4, 8)),
    CONSTRAINT fk_access_log_intake_shared FOREIGN KEY (shared)
        REFERENCES access_log_shared (digest),
    CONSTRAINT fk_access_log_intake_policy FOREIGN KEY (render_policy)
        REFERENCES audit_render_policy (digest)
);

ALTER TABLE access_log_intake
    ALTER COLUMN content_digest SET STORAGE MAIN,
    ALTER COLUMN shared SET STORAGE MAIN;

CREATE INDEX ix_access_log_intake_shared ON access_log_intake (shared) WHERE shared IS NOT NULL;

COMMENT ON TABLE access_log_intake IS
    'The access-log entries written before the response that reports their access and not chained yet: append-only, left only by audit.access_log_chain_move(), which writes each into the chain.';
COMMENT ON COLUMN access_log_intake.taken_at IS
    'When the request wrote the entry, for the age of the oldest entry not chained yet.';
COMMENT ON COLUMN access_log_intake.content IS
    'The row part of the entry''s canonical form, as access_log_record.content keeps it.';
COMMENT ON COLUMN access_log_intake.content_digest IS
    'The SHA-256 of the whole canonical form, computed by the writer.';
COMMENT ON COLUMN access_log_intake.render_policy IS
    'The render policy the entry''s projection is written under, with the other index columns NULL when the writer indexes no projection of the entry.';

-- ── the intake of the system chain ──────────────────────────────────────────

CREATE TABLE audit_event_intake (LIKE audit_event INCLUDING DEFAULTS INCLUDING CONSTRAINTS);
ALTER TABLE audit_event_intake
    DROP COLUMN stored_at,
    DROP COLUMN delivered_syslog_at,
    DROP COLUMN delivered_fhir_feed_at,
    DROP COLUMN chain_seq,
    DROP COLUMN prev_hash,
    DROP COLUMN row_hash,
    DROP COLUMN digest_version,
    ADD COLUMN intake_id bigint NOT NULL GENERATED ALWAYS AS IDENTITY,
    ADD COLUMN taken_at timestamptz NOT NULL DEFAULT now(),
    ADD CONSTRAINT pk_audit_event_intake PRIMARY KEY (intake_id),
    ADD CONSTRAINT uq_audit_event_intake_id UNIQUE (id),
    ADD CONSTRAINT fk_audit_event_intake_policy FOREIGN KEY (render_policy)
        REFERENCES audit_render_policy (digest);

COMMENT ON TABLE audit_event_intake IS
    'The audit records of no access-log entry written before the response that reports them and not chained yet: append-only, left only by audit.audit_event_chain_move(), which writes each into the system chain.';

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
    RAISE EXCEPTION 'audit.% is append-only: a record leaves it only by being chained', TG_TABLE_NAME
        USING ERRCODE = 'restrict_violation';
END;
$$;

CREATE TRIGGER access_log_intake_append_only
    BEFORE UPDATE OR DELETE ON access_log_intake
    FOR EACH ROW EXECUTE FUNCTION audit.audit_intake_reject_change();
CREATE TRIGGER access_log_intake_no_truncate
    BEFORE TRUNCATE ON access_log_intake
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_intake_reject_change();
CREATE TRIGGER audit_event_intake_append_only
    BEFORE UPDATE OR DELETE ON audit_event_intake
    FOR EACH ROW EXECUTE FUNCTION audit.audit_intake_reject_change();
CREATE TRIGGER audit_event_intake_no_truncate
    BEFORE TRUNCATE ON audit_event_intake
    FOR EACH STATEMENT EXECUTE FUNCTION audit.audit_intake_reject_change();

-- ── the heads ───────────────────────────────────────────────────────────────

-- Every chainer round updates the head of the access log, and every move of
-- the system intake the system chain's: room on the page keeps each update
-- HOT, and the vacuum of a table that only ever holds one live row runs
-- after a fixed number of updates, so the row does not bloat (#5770).
ALTER TABLE access_log_chain_state SET (fillfactor = 10,
    autovacuum_vacuum_scale_factor = 0, autovacuum_vacuum_threshold = 1000);
ALTER TABLE audit_chain_state SET (fillfactor = 10,
    autovacuum_vacuum_scale_factor = 0, autovacuum_vacuum_threshold = 1000);

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

-- ── the chainer ─────────────────────────────────────────────────────────────

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

COMMENT ON FUNCTION audit.access_log_chain_take() IS
    'Lock the head of the access log without waiting and return its position and link digest; no row when another chainer holds it.';

-- Write the intake rows p_ids into the chain at the positions p_seqs with the
-- links the chainer computed, each record's index row, and the outbox rows
-- of the sinks that deliver from the store, and remove the intake rows. The
-- insert trigger checks every link against the head.
CREATE FUNCTION audit.access_log_chain_move(
    p_ids bigint[],
    p_seqs bigint[],
    p_previous text[],
    p_digests text[],
    p_fhir_feed boolean,
    p_syslog boolean)
    RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    moved bigint;
BEGIN
    INSERT INTO audit.access_log_record
           (chain_seq, record_type, uid, time_recorded, content, algorithm, previous_digest,
            content_digest, digest, ehr_id, party_id, retain_until, action_terminology,
            action_code, result_terminology, result_code, shared)
    SELECT l.seq, i.record_type, i.uid, i.time_recorded, i.content, 'SHA-256',
           decode(l.previous, 'hex'), i.content_digest, decode(l.digest, 'hex'), i.ehr_id, i.party_id, i.retain_until,
           i.action_terminology, i.action_code, i.result_terminology, i.result_code, i.shared
      FROM unnest(p_ids, p_seqs, p_previous, p_digests) AS l(id, seq, previous, digest)
      JOIN audit.access_log_intake i ON i.id = l.id
     ORDER BY l.seq;
    GET DIAGNOSTICS moved = ROW_COUNT;
    IF moved IS DISTINCT FROM cardinality(p_ids)::bigint THEN
        RAISE EXCEPTION 'the chainer named % intake rows and % are in the intake', cardinality(p_ids), moved
            USING ERRCODE = 'restrict_violation';
    END IF;
    INSERT INTO audit.access_log_search
           (chain_seq, recorded_at, action, outcome, principal, patient_id, resource_id,
            request_id, client_ip, ehr_id, search_tokens, render_policy)
    SELECT l.seq, i.recorded_at, i.action, i.outcome, i.principal, i.patient_id, i.resource_id,
           i.request_id, i.client_ip, i.index_ehr_id, i.search_tokens, i.render_policy
      FROM unnest(p_ids, p_seqs) AS l(id, seq)
      JOIN audit.access_log_intake i ON i.id = l.id
     WHERE i.render_policy IS NOT NULL
     ORDER BY l.seq;
    IF p_fhir_feed THEN
        INSERT INTO audit.fhir_feed_outbox (chain, chain_seq)
        SELECT 'access_log', l.seq
          FROM unnest(p_ids, p_seqs) AS l(id, seq)
          JOIN audit.access_log_intake i ON i.id = l.id
         WHERE i.render_policy IS NOT NULL
         ORDER BY l.seq;
    END IF;
    IF p_syslog THEN
        INSERT INTO audit.syslog_outbox (chain, chain_seq)
        SELECT 'access_log', l.seq
          FROM unnest(p_ids, p_seqs) AS l(id, seq)
          JOIN audit.access_log_intake i ON i.id = l.id
         WHERE i.render_policy IS NOT NULL
         ORDER BY l.seq;
    END IF;
    PERFORM set_config('ferroehr.audit_chaining', 'on', true);
    DELETE FROM audit.access_log_intake i WHERE i.id = ANY (p_ids);
    PERFORM set_config('ferroehr.audit_chaining', 'off', true);
    RETURN moved;
END;
$$;

COMMENT ON FUNCTION audit.access_log_chain_move(bigint[], bigint[], text[], text[], boolean, boolean) IS
    'The one path an entry enters the access log by: the intake rows named, at the positions and with the links given, with their index rows and outbox rows; the intake rows leave in the same statement.';

-- Move up to p_limit records of the system intake, oldest first, into the
-- system chain, whose triggers link them; rows another replica holds are
-- skipped.
CREATE FUNCTION audit.audit_event_chain_move(p_limit integer, p_fhir_feed boolean, p_syslog boolean)
    RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    moved bigint;
BEGIN
    PERFORM set_config('ferroehr.audit_chaining', 'on', true);
    WITH taken AS (
        DELETE FROM audit.audit_event_intake i
         WHERE i.intake_id IN (SELECT t.intake_id FROM audit.audit_event_intake t
                                ORDER BY t.intake_id LIMIT p_limit FOR UPDATE SKIP LOCKED)
        RETURNING i.*
    ),
    stored AS (
        INSERT INTO audit.audit_event (id, recorded_at, action, outcome, event_code, operation,
               principal, patient_id, resource_class, resource_id, client_ip, token_id, domain,
               purpose, legal_basis, result_count, request_id, roles, origins, origin_count,
               categories, category_basis, category_evidence, category_map_digest, acting_mode,
               assurance_level, assurance_value, professional_id, emergency_access,
               retention_ends, retention_stated, permit, issuer, client_id, ehr_id,
               subject_namespace, subject_lookup_failed, category_content, render_policy,
               search_tokens, binding, guest, party, party_groups, provider, representation,
               external_fields, agreement, actor_chain, facilities, binding_detail, summary,
               token_attributes)
        SELECT t.id, t.recorded_at, t.action, t.outcome, t.event_code, t.operation,
               t.principal, t.patient_id, t.resource_class, t.resource_id, t.client_ip,
               t.token_id, t.domain, t.purpose, t.legal_basis, t.result_count, t.request_id,
               t.roles, t.origins, t.origin_count, t.categories, t.category_basis,
               t.category_evidence, t.category_map_digest, t.acting_mode, t.assurance_level,
               t.assurance_value, t.professional_id, t.emergency_access, t.retention_ends,
               t.retention_stated, t.permit, t.issuer, t.client_id, t.ehr_id,
               t.subject_namespace, t.subject_lookup_failed, t.category_content,
               t.render_policy, t.search_tokens, t.binding, t.guest, t.party, t.party_groups,
               t.provider, t.representation, t.external_fields, t.agreement, t.actor_chain,
               t.facilities, t.binding_detail, t.summary, t.token_attributes
          FROM taken t
         ORDER BY t.intake_id
        RETURNING chain_seq
    ),
    fed AS (
        INSERT INTO audit.fhir_feed_outbox (chain, chain_seq)
        SELECT 'system', s.chain_seq FROM stored s WHERE p_fhir_feed ORDER BY s.chain_seq
        RETURNING 1
    ),
    logged AS (
        INSERT INTO audit.syslog_outbox (chain, chain_seq)
        SELECT 'system', s.chain_seq FROM stored s WHERE p_syslog ORDER BY s.chain_seq
        RETURNING 1
    )
    SELECT count(*) INTO moved FROM stored;
    PERFORM set_config('ferroehr.audit_chaining', 'off', true);
    RETURN moved;
END;
$$;

COMMENT ON FUNCTION audit.audit_event_chain_move(integer, boolean, boolean) IS
    'The one path a record of no access-log entry enters the system chain by: up to p_limit intake rows, oldest first, with their outbox rows; the intake rows leave in the same statement.';

-- The insert trigger checks the link the chainer computed against the head,
-- and derives nothing: the writer supplied every column, and the verifier
-- derives them again (0039).
CREATE OR REPLACE FUNCTION audit.access_log_record_chain_link() RETURNS trigger
    LANGUAGE plpgsql
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cursor_value text := current_setting('ferroehr.access_log_chain_head', true);
    head_seq     bigint;
    head_link    text;
BEGIN
    IF cursor_value IS NULL OR cursor_value = '' THEN
        RAISE EXCEPTION 'the access-log chain cursor is unset: refusing to append an unchained record'
            USING ERRCODE = 'restrict_violation';
    END IF;
    head_seq := split_part(cursor_value, ':', 1)::bigint;
    head_link := split_part(cursor_value, ':', 2);
    IF NEW.record_type IS DISTINCT FROM 'ACCESS_LOG_ENTRY' THEN
        RAISE EXCEPTION 'an access-log record is written as an ACCESS_LOG_ENTRY, not %', NEW.record_type
            USING ERRCODE = 'check_violation';
    END IF;
    IF NEW.chain_seq IS DISTINCT FROM head_seq + 1
       OR NEW.previous_digest IS DISTINCT FROM decode(head_link, 'hex')
       OR NEW.algorithm IS DISTINCT FROM 'SHA-256'
       OR NEW.digest IS DISTINCT FROM decode(audit.access_log_link_digest(
              head_link, NEW.chain_seq, encode(NEW.content_digest, 'hex')), 'hex')
    THEN
        RAISE EXCEPTION 'an access-log record is written with the link that follows position % of the log', head_seq
            USING ERRCODE = 'check_violation';
    END IF;
    NEW.stored_at := now();
    PERFORM set_config('ferroehr.access_log_chain_head',
                      NEW.chain_seq || ':' || encode(NEW.digest, 'hex'), true);
    RETURN NEW;
END;
$$;

-- A shared part an intake row names stays while the row waits, and a
-- tombstone or a reaped record leaves the syslog outbox as it leaves the
-- FHIR Feed's.
CREATE OR REPLACE FUNCTION audit.access_log_record_unindex() RETURNS trigger
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF NEW.record_type = 'ACCESS_LOG_TOMBSTONE' THEN
        DELETE FROM audit.access_log_search s WHERE s.chain_seq = NEW.chain_seq;
        DELETE FROM audit.fhir_feed_outbox o
         WHERE o.chain = 'access_log' AND o.chain_seq = NEW.chain_seq;
        DELETE FROM audit.syslog_outbox o
         WHERE o.chain = 'access_log' AND o.chain_seq = NEW.chain_seq;
        IF OLD.shared IS NOT NULL THEN
            DELETE FROM audit.access_log_shared s
             WHERE s.digest = OLD.shared
               AND NOT EXISTS (SELECT 1 FROM audit.access_log_record r WHERE r.shared = OLD.shared)
               AND NOT EXISTS (SELECT 1 FROM audit.access_log_intake i WHERE i.shared = OLD.shared);
        END IF;
    END IF;
    RETURN NEW;
END;
$$;

CREATE OR REPLACE FUNCTION audit.audit_event_leave_outbox() RETURNS trigger
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

-- ── grants ───────────────────────────────────────────────────────────────────

-- One request's entries in one statement (#5770): the search tokens and the
-- shared sets they name are stored or found, the shared parts inserted and the
-- intake rows written, so a request's intake write is one round trip that
-- commits on its own. A token or set another writer stores at the same moment
-- is read again, as the server's own statements did. The tokens, the sets and
-- the shared parts, the rows concurrent statements share, are each inserted in
-- the order of their key, so two statements wait for each other's rows in one
-- order and never deadlock (#5881). Our own design.
CREATE FUNCTION audit.access_log_intake_take(
    p_record_types text[], p_uids text[], p_times text[], p_contents text[],
    p_shared text[], p_digests text[], p_ehr_ids text[], p_party_ids text[],
    p_retain_until text[], p_action_terminologies text[], p_action_codes text[],
    p_result_terminologies text[], p_result_codes text[], p_recorded_at timestamptz[],
    p_actions text[], p_outcomes smallint[], p_principals text[], p_patient_ids text[],
    p_resource_ids text[], p_request_ids text[], p_client_ips text[],
    p_index_ehr_ids text[], p_search_tokens text[], p_policies bytea[],
    p_shared_digests text[], p_shared_contents text[],
    p_token_entries integer[], p_token_parameters text[], p_token_systems text[],
    p_token_codes text[], p_token_shares boolean[])
    RETURNS void
    LANGUAGE plpgsql
    SET search_path = pg_catalog
    AS $$
DECLARE
    v_digests bytea[];
    v_ids integer[];
    v_set_keys text[];
    v_own text[];
    v_set_ids integer[];
BEGIN
    IF cardinality(p_token_entries) > 0 THEN
        PERFORM audit.hold_search_tokens();
        v_digests := ARRAY(
            SELECT audit.search_token_digest(t.p, t.s, t.c)
              FROM unnest(p_token_parameters, p_token_systems, p_token_codes)
                   WITH ORDINALITY AS t(p, s, c, i)
             ORDER BY t.i);
        FOR v_attempt IN 1..4 LOOP
            v_ids := ARRAY(
                SELECT h.id
                  FROM unnest(v_digests) WITH ORDINALITY AS d(digest, i)
                  LEFT JOIN audit.audit_search_token h ON h.digest = d.digest
                 ORDER BY d.i);
            EXIT WHEN array_position(v_ids, NULL) IS NULL;
            IF v_attempt = 4 THEN
                RAISE EXCEPTION 'a search token of the intake could not be stored or read';
            END IF;
            INSERT INTO audit.audit_search_token (parameter, system, code, digest)
            SELECT DISTINCT ON (t.digest) t.p, t.s, t.c, t.digest
              FROM unnest(p_token_parameters, p_token_systems, p_token_codes, v_digests, v_ids)
                   AS t(p, s, c, digest, id)
             WHERE t.id IS NULL
             ORDER BY t.digest
            ON CONFLICT (digest) DO NOTHING;
        END LOOP;
        v_set_keys := ARRAY(
            SELECT (SELECT array_agg(t.id ORDER BY t.id)::text
                      FROM unnest(p_token_entries, v_ids, p_token_shares) AS t(e, id, shares)
                     WHERE t.e = g AND t.shares)
              FROM generate_subscripts(p_uids, 1) AS g
             ORDER BY g);
        v_own := ARRAY(
            SELECT (SELECT array_agg(t.id ORDER BY t.id)::text
                      FROM unnest(p_token_entries, v_ids, p_token_shares) AS t(e, id, shares)
                     WHERE t.e = g AND NOT t.shares)
              FROM generate_subscripts(p_uids, 1) AS g
             ORDER BY g);
        FOR v_attempt IN 1..4 LOOP
            v_set_ids := ARRAY(
                SELECT x.id
                  FROM unnest(v_set_keys) WITH ORDINALITY AS k(tokens, i)
                  LEFT JOIN audit.audit_search_set x ON x.tokens = k.tokens::integer[]
                 ORDER BY k.i);
            EXIT WHEN NOT EXISTS (
                SELECT FROM unnest(v_set_keys, v_set_ids) AS k(tokens, id)
                 WHERE k.tokens IS NOT NULL AND k.id IS NULL);
            IF v_attempt = 4 THEN
                RAISE EXCEPTION 'a shared search set of the intake could not be stored or read';
            END IF;
            INSERT INTO audit.audit_search_set (tokens, digest)
            SELECT s.tokens, audit.search_set_digest(s.tokens)
              FROM (SELECT DISTINCT k.tokens::integer[] AS tokens
                      FROM unnest(v_set_keys, v_set_ids) AS k(tokens, id)
                     WHERE k.tokens IS NOT NULL AND k.id IS NULL) AS s
             ORDER BY s.tokens
            ON CONFLICT (tokens) DO NOTHING;
        END LOOP;
    END IF;
    INSERT INTO audit.access_log_shared (digest, content)
    SELECT decode(t.digest, 'hex'), t.content
      FROM unnest(p_shared_digests, p_shared_contents) AS t(digest, content)
     ORDER BY decode(t.digest, 'hex')
    ON CONFLICT (digest) DO NOTHING;
    INSERT INTO audit.access_log_intake (record_type, uid, time_recorded, content, shared,
        content_digest, ehr_id, party_id, retain_until, action_terminology, action_code,
        result_terminology, result_code, recorded_at, action, outcome, principal, patient_id,
        resource_id, request_id, client_ip, index_ehr_id, search_tokens, render_policy)
    SELECT u.c1, u.c2::uuid, u.c3::timestamptz, u.c4, decode(u.c5, 'hex'), decode(u.c6, 'hex'),
           u.c7, u.c8, u.c9::date, u.c10, u.c11, u.c12, u.c13, u.c14, u.c15, u.c16, u.c17, u.c18,
           u.c19, u.c20, u.c21, u.c22,
           CASE
               WHEN u.c23 IS NULL THEN NULL
               WHEN u.c23 <> '' THEN u.c23::integer[]
               WHEN v_set_ids[u.n] IS NULL THEN coalesce(v_own[u.n], '{}')::integer[]
               ELSE ARRAY[-v_set_ids[u.n]] || coalesce(v_own[u.n], '{}')::integer[]
           END,
           u.c24
      FROM unnest(p_record_types, p_uids, p_times, p_contents, p_shared, p_digests, p_ehr_ids,
                  p_party_ids, p_retain_until, p_action_terminologies, p_action_codes,
                  p_result_terminologies, p_result_codes, p_recorded_at, p_actions, p_outcomes,
                  p_principals, p_patient_ids, p_resource_ids, p_request_ids, p_client_ips,
                  p_index_ehr_ids, p_search_tokens, p_policies) WITH ORDINALITY
           AS u(c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15, c16, c17, c18,
                c19, c20, c21, c22, c23, c24, n)
     ORDER BY u.n;
END
$$;

COMMENT ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], timestamptz[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[]) IS
    'One request''s access-log entries into the intake in one statement: their search tokens and shared sets stored or found, their shared parts, their rows (#5770).';

REVOKE ALL ON FUNCTION audit.audit_intake_reject_change() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], timestamptz[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[]) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_chain_take() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.access_log_chain_move(bigint[], bigint[], text[], text[], boolean, boolean)
    FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.audit_event_chain_move(integer, boolean, boolean) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE INSERT ON audit_event, access_log_search FROM ferroehr_clinical;
        REVOKE INSERT (content, shared, content_digest) ON access_log_record FROM ferroehr_clinical;
        REVOKE INSERT ON fhir_feed_outbox FROM ferroehr_clinical;
        GRANT SELECT, INSERT ON access_log_intake, audit_event_intake TO ferroehr_clinical;
        GRANT SELECT ON access_log_intake, audit_event_intake, syslog_outbox
            TO ferroehr_clinical_reader;
        GRANT SELECT, DELETE ON syslog_outbox TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.access_log_chain_take() TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION
            audit.access_log_chain_move(bigint[], bigint[], text[], text[], boolean, boolean)
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.audit_event_chain_move(integer, boolean, boolean)
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
            text[], text[], text[], text[], text[], text[], text[], timestamptz[], text[], smallint[],
            text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
            text[], text[], text[], boolean[])
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the audit intake grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
