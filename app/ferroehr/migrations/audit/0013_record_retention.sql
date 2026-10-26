-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: each record keeps its own retention end, and the reaper deletes by it.
--
-- Regulation (EU) 2025/327 Annex II 3.4 (docs/law/eu/ehds/text.html) asks for
-- retention periods that take the categories and the origins of the data into
-- account, so two access records written the same day can end on different
-- dates. ehds-logging (FerroAUDIT) states each access record's end; every
-- other record keeps the store horizon. A record written from now on stores
-- the first date it may be deleted on (retention_ends, NULL with
-- retention_stated for a record kept without end). A record written before
-- this migration states nothing and keeps being reaped by the store horizon.
-- Both columns join the chain digest as version 3, so an end cannot be moved
-- without the chain check seeing it.
--
-- No openEHR spec governs audit storage: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

ALTER TABLE audit_event
    ADD COLUMN retention_ends date,
    ADD COLUMN retention_stated boolean NOT NULL DEFAULT false,
    ADD CONSTRAINT ck_audit_event_retention_stated
        CHECK (retention_ends IS NULL OR retention_stated);

COMMENT ON COLUMN audit_event.retention_ends IS
    'The first UTC date the record may be deleted on (Annex II 3.4): the access record''s own end, or the store horizon''s for any other record; NULL when it is kept without end or states no end.';
COMMENT ON COLUMN audit_event.retention_stated IS
    'Whether the record states its retention end; false for a record written before 0013, which the store horizon reaps.';

CREATE INDEX ix_audit_event_retention_ends ON audit_event (retention_ends)
    WHERE retention_ends IS NOT NULL;

CREATE OR REPLACE FUNCTION audit.audit_chain_digest_version() RETURNS smallint
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$ SELECT 3::smallint $$;

CREATE OR REPLACE FUNCTION audit.audit_event_digest_columns(p_version smallint) RETURNS text[]
    LANGUAGE sql IMMUTABLE PARALLEL SAFE
    SET search_path = pg_catalog
    AS $$
    SELECT CASE p_version
        WHEN 2 THEN ARRAY[
            'id', 'recorded_at', 'stored_at', 'action', 'outcome', 'event_code',
            'operation', 'principal', 'patient_id', 'resource_class',
            'resource_id', 'client_ip', 'token_id', 'fhir',
            'chain_seq', 'prev_hash', 'chain_digest_version',
            'domain', 'purpose', 'legal_basis', 'result_count', 'request_id',
            'organisation', 'roles', 'origins', 'origin_count',
            'categories', 'category_basis', 'category_evidence',
            'category_map_digest',
            'acting_mode', 'assurance_level', 'assurance_value',
            'professional_id',
            'emergency_access'
        ]
        WHEN 3 THEN ARRAY[
            'id', 'recorded_at', 'stored_at', 'action', 'outcome', 'event_code',
            'operation', 'principal', 'patient_id', 'resource_class',
            'resource_id', 'client_ip', 'token_id', 'fhir',
            'chain_seq', 'prev_hash', 'chain_digest_version',
            'domain', 'purpose', 'legal_basis', 'result_count', 'request_id',
            'organisation', 'roles', 'origins', 'origin_count',
            'categories', 'category_basis', 'category_evidence',
            'category_map_digest',
            'acting_mode', 'assurance_level', 'assurance_value',
            'professional_id',
            'emergency_access',
            'retention_ends', 'retention_stated'
        ]
    END
$$;

-- The one reaping body for records that state their end, beside the
-- horizon-keyed reaping of records that do not: one transaction, the same
-- serialisation against the chain writer, and the same tombstones.
CREATE FUNCTION audit.reap_audit_events_due(p_today date, p_unstated_cutoff timestamptz)
    RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    removed bigint;
    left_from  bigint;
    right_from bigint;
BEGIN
    IF p_today IS NULL THEN
        RETURN 0;
    END IF;
    PERFORM 1 FROM audit.audit_chain_state WHERE singleton FOR UPDATE;
    PERFORM set_config('ferroehr.audit_reaping', 'on', true);
    WITH gone AS (
        DELETE FROM audit.audit_event
         WHERE (retention_stated AND retention_ends IS NOT NULL AND retention_ends <= p_today)
            OR (NOT retention_stated AND p_unstated_cutoff IS NOT NULL
                AND recorded_at < p_unstated_cutoff)
        RETURNING chain_seq, row_hash
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
    SELECT count(*) INTO removed FROM gone;
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

COMMENT ON FUNCTION audit.reap_audit_events_due(date, timestamptz) IS
    'Retention reaping by each record''s own end: removes every record whose end is on or before p_today, and every record that states no end and is older than p_unstated_cutoff (NULL keeps those), tombstoning the chain positions they occupied.';

-- Functions are executable by PUBLIC unless revoked (PostgreSQL 18, GRANT,
-- "Notes", https://www.postgresql.org/docs/18/sql-grant.html).
REVOKE ALL ON FUNCTION audit.reap_audit_events_due(date, timestamptz) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT EXECUTE ON FUNCTION audit.reap_audit_events_due(date, timestamptz)
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping audit due-retention grants (roles absent — see the ext role block NOTICE)';
    END IF;
END
$grants$;
