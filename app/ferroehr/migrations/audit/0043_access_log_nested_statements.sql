-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the statements the database runs inside the access-log writes, per
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
--
-- Runs with search_path = audit, ext, public.

CREATE OR REPLACE FUNCTION audit.search_token_digest(p_parameter text, p_system text, p_code text)
    RETURNS bytea
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT pg_catalog.substr(pg_catalog.sha256(pg_catalog.convert_to(
               pg_catalog.jsonb_build_array(p_parameter, p_system, p_code)::pg_catalog.text,
               'UTF8')), 1, 16)
$$;

CREATE OR REPLACE FUNCTION audit.search_set_digest(p_tokens integer[]) RETURNS bytea
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT pg_catalog.sha256(pg_catalog.convert_to(p_tokens::pg_catalog.text, 'UTF8'))
$$;

CREATE OR REPLACE FUNCTION audit.access_log_content_digest(p_content text) RETURNS text
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(p_content, 'UTF8')), 'hex')
$$;

CREATE OR REPLACE FUNCTION audit.access_log_link_digest(p_previous text, p_sequence bigint, p_content_digest text)
    RETURNS text
    LANGUAGE sql STABLE PARALLEL SAFE
    AS $$
    SELECT pg_catalog.encode(pg_catalog.sha256(pg_catalog.convert_to(
               p_previous OPERATOR(pg_catalog.||) E'\n'
               OPERATOR(pg_catalog.||) p_sequence::pg_catalog.text
               OPERATOR(pg_catalog.||) E'\n'
               OPERATOR(pg_catalog.||) p_content_digest, 'UTF8')), 'hex')
$$;

-- The shared hold the writers take on the search tokens, inlined into the
-- statement that takes it.
CREATE OR REPLACE FUNCTION audit.hold_search_tokens() RETURNS void
    LANGUAGE sql VOLATILE
    AS $$
    SELECT pg_catalog.pg_advisory_xact_lock_shared(pg_catalog.hashtext('audit.audit_search_token'), 0)
$$;

-- ── the foreign keys the chain and the append-only tables already hold ──────

-- Each of these checked every chained record against a parent row with a
-- FOR KEY SHARE lock, one nested statement per record, on parents every
-- record of one caller or one policy names, so concurrent chainer rounds
-- and readers share locks on the same few rows. What each guarded holds
-- without it:
--
-- fk_access_log_record_shared: a record enters the log only through
--   audit.access_log_chain_move, from an intake row whose own foreign key
--   holds its shared part; the part leaves only through
--   audit.access_log_record_unindex, when neither a record nor an intake row
--   names it; and the verifier derives every record from its shared part
--   again (audit.access_log_record_derive), so a part removed by someone
--   with the owner's rights is a finding.
-- fk_access_log_search_record: an index row is written only by
--   audit.access_log_chain_move beside the record it indexes, and a record
--   leaves only as a tombstone, whose update removes its index row first.
-- fk_access_log_search_policy: the index row copies the render policy of
--   the intake row, whose foreign key checked it, and audit_render_policy
--   refuses every UPDATE, DELETE and TRUNCATE.
ALTER TABLE access_log_record DROP CONSTRAINT fk_access_log_record_shared;
ALTER TABLE access_log_search
    DROP CONSTRAINT fk_access_log_search_record,
    DROP CONSTRAINT fk_access_log_search_policy;

-- ── the link check of each chained record ───────────────────────────────────

-- As 0042, with the head cursor moved by an assignment, which PL/pgSQL
-- evaluates as a simple expression, in place of a PERFORM, which runs a
-- statement of its own for every record (PostgreSQL 18 docs, PL/pgSQL,
-- "Executing SQL Commands" and "Plan Caching").
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
    cursor_value := set_config('ferroehr.access_log_chain_head',
                               NEW.chain_seq || ':' || encode(NEW.digest, 'hex'), true);
    RETURN NEW;
END;
$$;
