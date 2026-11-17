-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the intake statement builds every entry's search tokens in one
-- grouped pass, so its cost grows with the tokens of the batch and not with
-- its entries times its tokens (#5889).
--
-- The statement of 0042 built each entry's shared set and own tokens with a
-- correlated subquery per entry over every token of the batch, and read the
-- per-entry results back by array subscript, which walks a text array or a
-- nullable array from its start. At the capacity load a gathered batch of
-- population requests holds hundreds of entries with about twenty tokens
-- each, and the statement took about 290 ms of PostgreSQL time a call, which
-- every request of the batch waited for (#5885).
--
-- Now the server sends each token's digest (audit.search_token_digest, the
-- same bytes, computed in system_log::search::digest), and one statement
-- finds the tokens, groups them by entry with their shared and own ids, and
-- counts the tokens and the shared sets the store does not hold. The intake
-- rows are written with the shared parts in one statement that joins the
-- groups by entry and the sets by their tokens. A token or set the store
-- does not hold is inserted, and the statement that finds them runs again;
-- a concurrent writer's row is waited for by ON CONFLICT and found then. The
-- tokens, the sets and the shared parts are each still inserted in the order
-- of their key (#5881). When every token and set is held the call runs three
-- statements: the token hold, the grouped pass and the write.
--
-- No openEHR spec governs audit storage mechanics: our own design.
--
-- Runs with search_path = audit, ext, public.

DROP FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], timestamptz[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[], uuid);

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
    WITH shared_parts AS (
        INSERT INTO audit.access_log_shared (digest, content)
        SELECT decode(t.digest, 'hex'), t.content
          FROM unnest(p_shared_digests, p_shared_contents) AS t(digest, content)
         ORDER BY decode(t.digest, 'hex')
        ON CONFLICT (digest) DO NOTHING
        RETURNING 1
    )
    INSERT INTO audit.access_log_intake (record_type, uid, time_recorded, content, shared,
        content_digest, ehr_id, party_id, retain_until, action_terminology, action_code,
        result_terminology, result_code, recorded_at, action, outcome, principal, patient_id,
        resource_id, request_id, client_ip, index_ehr_id, search_tokens, render_policy,
        reserved_by)
    SELECT u.c1, u.c2::uuid, u.c3::timestamptz, u.c4, decode(u.c5, 'hex'), decode(u.c6, 'hex'),
           u.c7, u.c8, u.c9::date, u.c10, u.c11, u.c12, u.c13, u.c14, u.c15, u.c16, u.c17, u.c18,
           u.c19, u.c20, u.c21, u.c22,
           CASE
               WHEN u.c23 IS NULL THEN NULL
               WHEN u.c23 <> '' THEN u.c23::integer[]
               WHEN x.id IS NULL THEN coalesce(g.own, '{}')::integer[]
               ELSE ARRAY[-x.id] || coalesce(g.own, '{}')::integer[]
           END,
           u.c24,
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
     ORDER BY u.n;
END
$$;

COMMENT ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], timestamptz[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[], uuid, bytea[]) IS
    'One batch''s access-log entries into the intake: their search tokens, found by the digests the server computed, grouped by entry in one pass, their shared sets and shared parts stored or found, and their rows (#5770, #5889).';

REVOKE ALL ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
    text[], text[], text[], text[], text[], text[], text[], timestamptz[], text[], smallint[],
    text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
    text[], text[], text[], boolean[], uuid, bytea[]) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT EXECUTE ON FUNCTION audit.access_log_intake_take(text[], text[], text[], text[], text[], text[],
            text[], text[], text[], text[], text[], text[], text[], timestamptz[], text[], smallint[],
            text[], text[], text[], text[], text[], text[], text[], bytea[], text[], text[], integer[],
            text[], text[], text[], boolean[], uuid, bytea[])
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the audit intake grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
