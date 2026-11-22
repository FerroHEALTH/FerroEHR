-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the access log's write, the holds' release and discard, and the
-- syslog outbox.
--
-- Every audit record is written durably before the response that reports the
-- access: a request writes its access-log entries and every other audit
-- record in its own transaction, before it answers, so concurrent requests
-- never wait on a head. An access-log entry is written once into
-- access_log_entry and a system record once into audit_event, each sealed
-- later by window (#6242). A write's records are held under a reservation id
-- until the write's own transaction commits it; the reconciler releases or
-- discards what a process that died left held.
--
-- No openEHR spec governs audit storage mechanics: our own design.
--
-- Runs with search_path = audit, ext, public.

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
    'The records the ITI-20 syslog route still owes, each by its chain and position: written when a frontier round owes a record while syslog is configured, removed when the repository accepted it. The frame is rendered from the stored record when it is read.';

-- The access-log entries owed to the sinks that deliver from the store
-- (#6242): every entry up to through_id that its writer indexed is owed by
-- its outbox rows; the frontier round moves it.
CREATE TABLE access_log_owed (
    singleton  boolean NOT NULL DEFAULT true,
    through_id bigint  NOT NULL DEFAULT 0,
    CONSTRAINT pk_access_log_owed PRIMARY KEY (singleton),
    CONSTRAINT ck_access_log_owed_singleton CHECK (singleton)
) WITH (fillfactor = 10);
INSERT INTO access_log_owed (singleton, through_id) VALUES (true, 0);

-- Owes the indexed entries past through_id up to p_upto and below every
-- entry a write holds to the sinks named, and moves through_id; returns how
-- many entries it owed and the through_id it left, none and 0 while another
-- replica's round holds the row.
CREATE FUNCTION audit.access_log_owe(p_upto bigint, p_fhir_feed boolean, p_syslog boolean)
    RETURNS TABLE (owed_count bigint, owed_through bigint)
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    v_from  bigint;
    v_upto  bigint;
    v_owed  bigint;
BEGIN
    SELECT o.through_id INTO v_from FROM audit.access_log_owed o
     WHERE o.singleton FOR UPDATE SKIP LOCKED;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 0::bigint, 0::bigint;
        RETURN;
    END IF;
    v_upto := least(p_upto, coalesce((SELECT min(h.entry_id) - 1 FROM audit.access_log_hold h), p_upto));
    IF v_upto <= v_from THEN
        RETURN QUERY SELECT 0::bigint, v_from;
        RETURN;
    END IF;
    WITH owed AS (
        SELECT e.id FROM audit.access_log_entry e
         WHERE e.id > v_from AND e.id <= v_upto AND e.render_policy_id IS NOT NULL
    ),
    fed AS (
        INSERT INTO audit.fhir_feed_outbox (chain, chain_seq)
        SELECT 'access_log', o.id FROM owed o WHERE p_fhir_feed ORDER BY o.id
        ON CONFLICT (chain, chain_seq) DO NOTHING
        RETURNING 1
    ),
    logged AS (
        INSERT INTO audit.syslog_outbox (chain, chain_seq)
        SELECT 'access_log', o.id FROM owed o WHERE p_syslog ORDER BY o.id
        ON CONFLICT (chain, chain_seq) DO NOTHING
        RETURNING 1
    )
    SELECT count(*) INTO v_owed FROM owed, (SELECT count(*) FROM fed) f, (SELECT count(*) FROM logged) l;
    UPDATE audit.access_log_owed SET through_id = v_upto WHERE singleton;
    RETURN QUERY SELECT v_owed, v_upto;
END;
$$;
COMMENT ON FUNCTION access_log_owe(bigint, boolean, boolean) IS
    'Owes the indexed access-log entries past the last owed one, up to the frontier and below every held entry, to the configured sinks (#6242).';

-- The system records owed to the sinks that deliver from the store
-- (#6242): every record up to through_id is owed by its outbox rows; the
-- frontier round moves it.
CREATE TABLE audit_event_owed (
    singleton  boolean NOT NULL DEFAULT true,
    through_id bigint  NOT NULL DEFAULT 0,
    CONSTRAINT pk_audit_event_owed PRIMARY KEY (singleton),
    CONSTRAINT ck_audit_event_owed_singleton CHECK (singleton)
) WITH (fillfactor = 10);
INSERT INTO audit_event_owed (singleton, through_id) VALUES (true, 0);

-- Owes the system records past through_id up to p_upto and below every
-- record a write holds to the sinks named, and moves through_id; returns how
-- many records it owed and the through_id it left, none and 0 while another
-- replica's round holds the row.
CREATE FUNCTION audit.audit_event_owe(p_upto bigint, p_fhir_feed boolean, p_syslog boolean)
    RETURNS TABLE (owed_count bigint, owed_through bigint)
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    v_from  bigint;
    v_upto  bigint;
    v_owed  bigint;
BEGIN
    SELECT o.through_id INTO v_from FROM audit.audit_event_owed o
     WHERE o.singleton FOR UPDATE SKIP LOCKED;
    IF NOT FOUND THEN
        RETURN QUERY SELECT 0::bigint, 0::bigint;
        RETURN;
    END IF;
    v_upto := least(p_upto, coalesce((SELECT min(h.seq) - 1 FROM audit.audit_event_hold h), p_upto));
    IF v_upto <= v_from THEN
        RETURN QUERY SELECT 0::bigint, v_from;
        RETURN;
    END IF;
    WITH owed AS (
        SELECT e.seq FROM audit.audit_event e WHERE e.seq > v_from AND e.seq <= v_upto
    ),
    fed AS (
        INSERT INTO audit.fhir_feed_outbox (chain, chain_seq)
        SELECT 'system', o.seq FROM owed o WHERE p_fhir_feed ORDER BY o.seq
        ON CONFLICT (chain, chain_seq) DO NOTHING
        RETURNING 1
    ),
    logged AS (
        INSERT INTO audit.syslog_outbox (chain, chain_seq)
        SELECT 'system', o.seq FROM owed o WHERE p_syslog ORDER BY o.seq
        ON CONFLICT (chain, chain_seq) DO NOTHING
        RETURNING 1
    )
    SELECT count(*) INTO v_owed FROM owed, (SELECT count(*) FROM fed) f, (SELECT count(*) FROM logged) l;
    UPDATE audit.audit_event_owed SET through_id = v_upto WHERE singleton;
    RETURN QUERY SELECT v_owed, v_upto;
END;
$$;
COMMENT ON FUNCTION audit_event_owe(bigint, boolean, boolean) IS
    'Owes the system records past the last owed one, up to the frontier and below every held record, to the configured sinks (#6242).';

-- Refuses an access-log entry naming a shared part or a render policy the store
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
        RAISE EXCEPTION 'the % % of an access-log entry was removed while it was written', p_what, p_digest
            USING ERRCODE = 'serialization_failure';
    END IF;
    RAISE EXCEPTION 'an access-log entry names the % %, which the store does not hold', p_what, p_digest
        USING ERRCODE = 'foreign_key_violation';
END;
$$;
COMMENT ON FUNCTION access_log_intake_unknown(text, text, boolean) IS
    'Raises foreign_key_violation for an access-log entry naming a shared part or a render policy the store does not hold, and serialization_failure for a part of the batch removed while it was written (#5887).';

-- Whether the shared part p_digest is gone, read with a snapshot of its own:
-- VOLATILE, so it sees a removal committed while the intake statement waited
-- on the part's row. Called only for a part the statement had to raise and
-- did not (#6241).
CREATE FUNCTION audit.access_log_intake_part_gone(p_digest text) RETURNS boolean
    LANGUAGE plpgsql VOLATILE
    SET search_path = pg_catalog
    AS $$
BEGIN
    RETURN NOT EXISTS (SELECT 1 FROM audit.access_log_shared s
                        WHERE s.digest = decode(p_digest, 'hex'));
END;
$$;
COMMENT ON FUNCTION access_log_intake_part_gone(text) IS
    'Whether a shared part is gone, read with a snapshot of its own; the intake asks it of a part it had to raise and did not (#6241).';

-- Holds the search tokens until the transaction ends and checks that the ids
-- a writer names from memory are of the store's generation: the guard of the
-- plain write (#6242), which runs it once per statement. NULL holds nothing.
CREATE FUNCTION audit.access_log_token_generation_holds(p_generation bigint)
    RETURNS boolean
    LANGUAGE plpgsql VOLATILE
    SET search_path = pg_catalog
    AS $$
BEGIN
    IF p_generation IS NULL THEN
        RETURN true;
    END IF;
    PERFORM audit.hold_search_tokens();
    IF p_generation IS DISTINCT FROM
       (SELECT g.generation FROM audit.audit_search_token_generation g WHERE g.singleton)
    THEN
        RAISE EXCEPTION 'the search token ids the writer named are of another generation'
            USING ERRCODE = 'FE001';
    END IF;
    RETURN true;
END;
$$;
COMMENT ON FUNCTION access_log_token_generation_holds(bigint) IS
    'Holds the search tokens and raises FE001 when the generation the writer named its token ids under is not the store''s; true otherwise (#6242).';

-- Writes each entry once into access_log_entry, stores or finds its search
-- tokens, shared set and shared part, and holds the entries of a
-- reservation; a shared part's retain_until follows its entries'.
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
    p_token_digests bytea[], p_token_generation bigint)
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
    -- Ids the writer names from memory hold while their generation is the
    -- store's: retention, which moves it, waits on the hold (#6242).
    IF p_token_generation IS NOT NULL THEN
        PERFORM audit.access_log_token_generation_holds(p_token_generation);
    END IF;
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
        ON CONFLICT (digest) DO UPDATE SET retain_until = excluded.retain_until
         WHERE audit.access_log_shared.retain_until < excluded.retain_until;
    END IF;
    -- No row lock is taken on a part an entry names (#6241). A part the batch
    -- carries is stored, or raised under the row lock its upsert takes, or
    -- already kept through today, and the collector removes a part only a day
    -- after its retain_until, so none of them goes before this transaction
    -- commits. A part the batch leaves out and the statement must raise is
    -- raised under its row lock below, and refused when the collector removed
    -- it while the raise waited.
    -- The transaction takes its xid before any row draws an id: the frontier
    -- rests on it (access_log::frontier, #6242).
    WITH own_xid AS MATERIALIZED (SELECT pg_catalog.pg_current_xact_id() AS taken),
    held AS (
        SELECT s.digest, s.retain_until
          FROM audit.access_log_shared s
         WHERE s.digest IN (SELECT DISTINCT decode(t.c, 'hex')
                              FROM unnest(p_shared) AS t(c)
                             WHERE t.c IS NOT NULL)
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
        RETURNING s.digest
    ),
    written AS (
        INSERT INTO audit.access_log_entry (record_type, uid, time_recorded, content, shared,
            content_digest, ehr_id, party_id, retain_until, action_terminology, action_code,
            result_terminology, result_code, event_action, event_outcome, principal, patient_id,
            resource_id, request_id, client_ip, classified_ehr_id, search_tokens,
            render_policy_id)
        SELECT u.c1, u.c2::uuid, u.c3::timestamptz, u.c4,
               CASE
                   WHEN u.c5 IS NOT NULL
                        AND (held.digest IS NULL
                             OR (held.retain_until < need.until AND raised.digest IS NULL
                                 AND audit.access_log_intake_part_gone(u.c5)))
                   THEN audit.access_log_intake_unknown('shared part', u.c5,
                            batch.digest IS NOT NULL OR held.digest IS NOT NULL)
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
          FROM own_xid CROSS JOIN unnest(p_record_types, p_uids, p_times, p_contents, p_shared, p_digests, p_ehr_ids,
                      p_party_ids, p_retain_until, p_action_terminologies, p_action_codes,
                      p_result_terminologies, p_result_codes, p_actions, p_outcomes,
                      p_principals, p_patient_ids, p_resource_ids, p_request_ids, p_client_ips,
                      p_index_ehr_ids, p_search_tokens, p_policies) WITH ORDINALITY
               AS u(c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c15, c16, c17, c18,
                    c19, c20, c21, c22, c23, c24, n)
          LEFT JOIN unnest(v_entries, v_shared, v_own) AS g(e, shared, own) ON g.e = u.n
          LEFT JOIN audit.audit_search_set x ON x.tokens = g.shared::integer[]
          LEFT JOIN held ON held.digest = decode(u.c5, 'hex')
          LEFT JOIN needed need ON need.digest = decode(u.c5, 'hex')
          LEFT JOIN kept raised ON raised.digest = decode(u.c5, 'hex')
          LEFT JOIN unnest(p_shared_digests) AS batch(digest) ON batch.digest = u.c5
          LEFT JOIN audit.audit_render_policy policy ON policy.digest = u.c24
         ORDER BY u.n
        RETURNING id
    )
    INSERT INTO audit.access_log_hold (entry_id, reserved_by)
    SELECT w.id, p_reserved_by
      FROM written w
     WHERE p_reserved_by IS NOT NULL
     ORDER BY w.id;
END
$$;
COMMENT ON FUNCTION access_log_intake_take(text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], text[], smallint[], text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[], text[], text[], text[], boolean[], uuid, bytea[], bigint) IS
    'One batch''s access-log entries, each written once: their search tokens, found by the digests the server computed, grouped by entry in one pass, their shared sets and shared parts stored or found, checked and kept as long as the entries, their rows in access_log_entry, and a hold per entry of a reservation until its write commits; the ids a writer names from memory are checked against their generation (#5770, #5887, #5889, #5968, #6242).';

-- Releases the records held under committed reservations, counting them.
CREATE FUNCTION audit.audit_intake_release(p_ids uuid[]) RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    v_entries bigint;
    v_others  bigint;
BEGIN
    WITH released AS (
        DELETE FROM audit.access_log_hold WHERE reserved_by = ANY (p_ids)
        RETURNING 1
    )
    SELECT count(*) INTO v_entries FROM released r;
    WITH released AS (
        DELETE FROM audit.audit_event_hold WHERE reserved_by = ANY (p_ids)
        RETURNING 1
    )
    SELECT count(*) INTO v_others FROM released r;
    RETURN v_entries + v_others;
END;
$$;
COMMENT ON FUNCTION audit_intake_release(uuid[]) IS
    'Release the records held under the reservations named: their writes committed, so the reads, the sealer and the deliveries take them.';

-- Discards the records held under reservations that never committed, which
-- no seal covers.
CREATE FUNCTION audit.audit_intake_discard(p_ids uuid[]) RETURNS bigint
    LANGUAGE plpgsql VOLATILE SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    v_entries bigint;
    v_others  bigint;
BEGIN
    PERFORM set_config('ferroehr.audit_discarding', 'on', true);
    WITH removed AS (
        DELETE FROM audit.access_log_entry e
         USING audit.access_log_hold h
         WHERE h.entry_id = e.id AND h.reserved_by = ANY (p_ids)
        RETURNING 1
    ),
    dropped AS (
        DELETE FROM audit.access_log_hold WHERE reserved_by = ANY (p_ids)
        RETURNING 1
    )
    SELECT count(*) INTO v_entries FROM removed, (SELECT count(*) FROM dropped) AS d;
    WITH removed AS (
        DELETE FROM audit.audit_event e
         USING audit.audit_event_hold h
         WHERE h.seq = e.seq AND h.reserved_by = ANY (p_ids)
        RETURNING 1
    ),
    dropped AS (
        DELETE FROM audit.audit_event_hold WHERE reserved_by = ANY (p_ids)
        RETURNING 1
    )
    SELECT count(*) INTO v_others FROM removed, (SELECT count(*) FROM dropped) AS d;
    PERFORM set_config('ferroehr.audit_discarding', 'off', true);
    RETURN v_entries + v_others;
END;
$$;
COMMENT ON FUNCTION audit_intake_discard(uuid[]) IS
    'Discard the records held under the reservations named, whose writes never committed.';
