-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: an entity.identifier that carries the record's request id or resource
-- id is answered from that column, and retention removes the search tokens no
-- record names any more.
--
-- 0017 stored every search token of a record's AuditEvent once in
-- audit_search_token. The request id (BALP entity:transaction) is unique to its
-- request and the resource id of a create is a new version uid, so each record
-- added a token row of its own, about 200 bytes per audited request (#4524),
-- and the reject-change trigger kept those rows after retention removed their
-- records.
--
-- RESTful ATNA ITI-81 §3.81.4.1.2.2 searches entity.identifier, which matches
-- AuditEvent.entity.what.identifier, so both values stay searchable. A record
-- now names, in its shared search set, a token of the parameter
-- 'entity.identifier=request_id' or 'entity.identifier=resource_id' with the
-- system the document writes the identifier in and no code: it says the
-- document carries that column's value as an entity.identifier in that system.
-- A search reads the value from the column, through the request id index below
-- and the resource id index of 0017. Records written before this migration keep
-- their tokens and are found through them, and their chain digests do not change.
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
--
-- Runs with search_path = audit, ext, public.

CREATE INDEX ix_audit_event_request_id ON audit_event (hashtextextended(request_id, 0));

COMMENT ON COLUMN audit_search_token.parameter IS
    'The ITI-81 parameter of the token, or entity.identifier=request_id or entity.identifier=resource_id for a token that says the document carries that column''s value as an entity.identifier in the token''s system.';

-- ── the token lock ───────────────────────────────────────────────────────────

-- The two-key form keeps the lock apart from the single-key advisory locks the
-- clinical writers take (PostgreSQL 18 docs, "Advisory Lock Functions",
-- https://www.postgresql.org/docs/18/functions-admin.html#FUNCTIONS-ADVISORY-LOCKS).
CREATE FUNCTION audit.hold_search_tokens() RETURNS void
    LANGUAGE sql VOLATILE
    SET search_path = pg_catalog
    AS $$ SELECT pg_advisory_xact_lock_shared(hashtext('audit.audit_search_token'), 0) $$;

COMMENT ON FUNCTION audit.hold_search_tokens() IS
    'Holds the search tokens shared until the transaction ends, so retention deletes none the transaction is about to name.';

CREATE FUNCTION audit.lock_search_tokens() RETURNS void
    LANGUAGE sql VOLATILE
    SET search_path = pg_catalog
    AS $$ SELECT pg_advisory_xact_lock(hashtext('audit.audit_search_token'), 0) $$;

COMMENT ON FUNCTION audit.lock_search_tokens() IS
    'Holds the search tokens exclusively until the transaction ends: the retention side of audit.hold_search_tokens().';

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

DROP TRIGGER audit_search_token_reject_change ON audit_search_token;
CREATE TRIGGER audit_search_token_reject_change
    BEFORE UPDATE OR DELETE ON audit_search_token
    FOR EACH ROW EXECUTE FUNCTION audit.audit_search_token_reject_change();

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

COMMENT ON FUNCTION audit.release_search_tokens(integer[]) IS
    'Deletes the search tokens among p_tokens that no record and no search set names; the reaping bodies call it with the token ids of the records they removed.';

-- ── the reaping bodies ───────────────────────────────────────────────────────

CREATE OR REPLACE FUNCTION audit.reap_audit_events_before(p_cutoff timestamptz) RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    removed    bigint;
    released   integer[];
    left_from  bigint;
    right_from bigint;
BEGIN
    -- The token lock first, then the chain: a writer takes them in this order.
    PERFORM audit.lock_search_tokens();
    -- Serialize against the chain writer: a record must not be appended while
    -- the tombstones for this reap are being computed.
    PERFORM 1 FROM audit.audit_chain_state WHERE singleton FOR UPDATE;

    PERFORM set_config('ferroehr.audit_reaping', 'on', true);
    WITH gone AS (
        DELETE FROM audit.audit_event WHERE recorded_at < p_cutoff
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

CREATE OR REPLACE FUNCTION audit.reap_audit_events_due(p_today date, p_unstated_cutoff timestamptz)
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

-- ── grants ───────────────────────────────────────────────────────────────────

-- Functions are executable by PUBLIC unless revoked (PostgreSQL 18, GRANT,
-- "Notes", https://www.postgresql.org/docs/18/sql-grant.html).
REVOKE ALL ON FUNCTION audit.hold_search_tokens() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.lock_search_tokens() FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.release_search_tokens(integer[]) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT EXECUTE ON FUNCTION audit.hold_search_tokens() TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping audit search token lock grants (roles absent: see the ext role block NOTICE)';
    END IF;
END
$grants$;
