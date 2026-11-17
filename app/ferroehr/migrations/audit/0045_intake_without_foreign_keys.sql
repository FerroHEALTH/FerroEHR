-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: an intake row names its shared part and its render policy with no
-- foreign key, so writing an entry locks no parent row (#5887).
--
-- fk_access_log_intake_shared and fk_access_log_intake_policy checked every
-- intake row with a FOR KEY SHARE lock on a parent row, one nested statement
-- per entry. Every entry of one caller names the same shared part and every
-- entry under one render policy the same policy, so concurrent intake
-- statements shared locks on the same few rows, which PostgreSQL keeps as
-- multixacts. What each key guarded holds without it:
--
-- The render policy: audit_render_policy refuses every UPDATE, DELETE and
-- TRUNCATE, so the key only refused a writer naming a policy it never
-- stored. The intake statement refuses that itself, in the statement that
-- writes the rows: it joins each row's policy and calls
-- audit.access_log_intake_unknown for one that is not stored, which raises
-- foreign_key_violation as the key did.
--
-- The shared part: the key also kept a tombstone from removing a part an
-- intake statement named before it committed, since the removal in
-- audit.access_log_record_unindex checks only the rows it can see. The
-- statement now stores the batch's parts, and then the statement that writes
-- the rows locks each distinct part they name once, FOR KEY SHARE, as the key
-- locked it once per row. The lock is taken in a statement of its own after
-- the parts are stored, so it sees a part another writer stored meanwhile.
-- The removal locks the part FOR UPDATE before it checks, so it waits for the
-- write's commit and its check then sees the write's rows. A row
-- naming a part the lock did not find is refused: as a serialization failure
-- when the batch carried the part, which a tombstone removed in between, and
-- as a foreign_key_violation otherwise. The verifier still derives every
-- record from its shared part (0039).
--
-- No openEHR spec governs audit storage mechanics: our own design.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE access_log_intake
    DROP CONSTRAINT fk_access_log_intake_shared,
    DROP CONSTRAINT fk_access_log_intake_policy;

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

COMMENT ON FUNCTION audit.access_log_intake_unknown(text, text, boolean) IS
    'Raises foreign_key_violation for an intake row naming a shared part or a render policy the store does not hold, and serialization_failure for a part of the batch removed while it was written (#5887).';

REVOKE ALL ON FUNCTION audit.access_log_intake_unknown(text, text, boolean) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT EXECUTE ON FUNCTION audit.access_log_intake_unknown(text, text, boolean) TO ferroehr_clinical;
        -- FOR KEY SHARE asks for UPDATE on a column of the table (PostgreSQL 18,
        -- SELECT, "The Locking Clause"); access_log_shared_unchanged refuses
        -- every UPDATE, so the grant changes no row.
        GRANT UPDATE (digest) ON access_log_shared TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the intake refusal grant (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;

-- As 0044, with each row's shared part and render policy checked in the
-- statement that writes it, and each distinct stored part locked once.
CREATE OR REPLACE FUNCTION audit.access_log_intake_take(
    p_record_types text[], p_uids text[], p_times text[], p_contents text[],
    p_shared text[], p_digests text[], p_ehr_ids text[], p_party_ids text[],
    p_retain_until text[], p_action_terminologies text[], p_action_codes text[],
    p_result_terminologies text[], p_result_codes text[], p_recorded_at timestamptz[],
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
        INSERT INTO audit.access_log_shared (digest, content)
        SELECT decode(t.digest, 'hex'), t.content
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
    )
    INSERT INTO audit.access_log_intake (record_type, uid, time_recorded, content, shared,
        content_digest, ehr_id, party_id, retain_until, action_terminology, action_code,
        result_terminology, result_code, recorded_at, action, outcome, principal, patient_id,
        resource_id, request_id, client_ip, index_ehr_id, search_tokens, render_policy,
        reserved_by)
    SELECT u.c1, u.c2::uuid, u.c3::timestamptz, u.c4,
           CASE
               WHEN u.c5 IS NOT NULL AND held.digest IS NULL
               THEN audit.access_log_intake_unknown('shared part', u.c5, batch.digest IS NOT NULL)
               ELSE decode(u.c5, 'hex')
           END,
           decode(u.c6, 'hex'),
           u.c7, u.c8, u.c9::date, u.c10, u.c11, u.c12, u.c13, u.c14, u.c15, u.c16, u.c17, u.c18,
           u.c19, u.c20, u.c21, u.c22,
           CASE
               WHEN u.c23 IS NULL THEN NULL
               WHEN u.c23 <> '' THEN u.c23::integer[]
               WHEN x.id IS NULL THEN coalesce(g.own, '{}')::integer[]
               ELSE ARRAY[-x.id] || coalesce(g.own, '{}')::integer[]
           END,
           CASE
               WHEN u.c24 IS NOT NULL AND policy.digest IS NULL
               THEN audit.access_log_intake_unknown('render policy', encode(u.c24, 'hex'), false)
               ELSE u.c24
           END,
           p_reserved_by
      FROM unnest(p_record_types, p_uids, p_times, p_contents, p_shared, p_digests, p_ehr_ids,
                  p_party_ids, p_retain_until, p_action_terminologies, p_action_codes,
                  p_result_terminologies, p_result_codes, p_recorded_at, p_actions, p_outcomes,
                  p_principals, p_patient_ids, p_resource_ids, p_request_ids, p_client_ips,
                  p_index_ehr_ids, p_search_tokens, p_policies) WITH ORDINALITY
           AS u(c1, c2, c3, c4, c5, c6, c7, c8, c9, c10, c11, c12, c13, c14, c15, c16, c17, c18,
                c19, c20, c21, c22, c23, c24, n)
      LEFT JOIN unnest(v_entries, v_shared, v_own) AS g(e, shared, own) ON g.e = u.n
      LEFT JOIN audit.audit_search_set x ON x.tokens = g.shared::integer[]
      LEFT JOIN held ON held.digest = decode(u.c5, 'hex')
      LEFT JOIN unnest(p_shared_digests) AS batch(digest) ON batch.digest = u.c5
      LEFT JOIN audit.audit_render_policy policy ON policy.digest = u.c24
     ORDER BY u.n;
END
$$;



-- As 0042, with the shared part locked before the check that no record and
-- no intake row names it. The lock waits for an intake write that holds the
-- part, and the check that follows runs with a snapshot taken after that
-- write committed, so it sees the write's rows; a DELETE alone would only
-- check its row again once the lock is free, with its first snapshot
-- (PostgreSQL 18 docs, "Transaction Isolation", Read Committed).
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
            PERFORM FROM audit.access_log_shared s WHERE s.digest = OLD.shared FOR UPDATE;
            DELETE FROM audit.access_log_shared s
             WHERE s.digest = OLD.shared
               AND NOT EXISTS (SELECT 1 FROM audit.access_log_record r WHERE r.shared = OLD.shared)
               AND NOT EXISTS (SELECT 1 FROM audit.access_log_intake i WHERE i.shared = OLD.shared);
        END IF;
    END IF;
    RETURN NEW;
END;
$$;
