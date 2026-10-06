-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: a retention horizon in calendar years.
--
-- The rules a horizon answers to are written in years: EHDS Art. 9(2) keeps
-- the access information "for at least three years from each date of access"
-- (docs/law/eu/ehds/text.html), and SGB V § 309 Abs. 1 and Abs. 3 keep a
-- telematics application's access log for the three-year limitation period and
-- then delete it "unverzüglich" (docs/law/de/sgb-v/BJNR024820988.xml). A
-- horizon counted in days can only approximate three calendar years, so a floor
-- and a ceiling of the same three years leave no day count that satisfies both.
-- [audit.store] retention_years states the horizon in years, and this file
-- gives the reaper the calendar arithmetic to honour it: subtracting an
-- interval of years from a timestamp moves the calendar date (PostgreSQL 18,
-- "Date/Time Functions and Operators",
-- https://www.postgresql.org/docs/18/functions-datetime.html).
--
-- One body does the reaping, keyed on a cutoff instant; the day-keyed function
-- the retention file ships and the year-keyed one below both delegate to it.
-- The cutoff-keyed body is granted to nobody: a caller choosing the cutoff could
-- reap records younger than any configured horizon.
--
-- No openEHR spec governs audit retention: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

-- Deletes the records older than `p_cutoff` and tombstones the chain positions
-- they occupied, collapsing abutting tombstones. Returns the number removed.
CREATE FUNCTION audit.reap_audit_events_before(p_cutoff timestamptz) RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    removed bigint;
    left_from  bigint;
    right_from bigint;
BEGIN
    -- Serialize against the chain writer: a record must not be appended while
    -- the tombstones for this reap are being computed.
    PERFORM 1 FROM audit.audit_chain_state WHERE singleton FOR UPDATE;

    PERFORM set_config('ferroehr.audit_reaping', 'on', true);
    WITH gone AS (
        DELETE FROM audit.audit_event WHERE recorded_at < p_cutoff
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

COMMENT ON FUNCTION audit.reap_audit_events_before(timestamptz) IS
    'The one reaping body: removes records older than the cutoff and tombstones their chain positions. Granted to nobody; reached only through the day- and year-keyed wrappers.';

CREATE OR REPLACE FUNCTION audit.reap_audit_events(p_retention_days integer) RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF p_retention_days IS NULL OR p_retention_days <= 0 THEN
        RETURN 0;
    END IF;
    RETURN audit.reap_audit_events_before(now() - make_interval(days => p_retention_days));
END;
$$;

CREATE FUNCTION audit.reap_audit_events_years(p_retention_years integer) RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
BEGIN
    IF p_retention_years IS NULL OR p_retention_years <= 0 THEN
        RETURN 0;
    END IF;
    RETURN audit.reap_audit_events_before(now() - make_interval(years => p_retention_years));
END;
$$;

COMMENT ON FUNCTION audit.reap_audit_events_years(integer) IS
    'Retention reaping on a horizon of calendar years: removes records older than the same calendar date that many years ago, tombstoning the chain positions they occupied.';

-- Functions are executable by PUBLIC unless revoked (PostgreSQL 18, GRANT,
-- "Notes", https://www.postgresql.org/docs/18/sql-grant.html).
REVOKE ALL ON FUNCTION audit.reap_audit_events_before(timestamptz) FROM PUBLIC;
REVOKE ALL ON FUNCTION audit.reap_audit_events_years(integer) FROM PUBLIC;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT EXECUTE ON FUNCTION audit.reap_audit_events_years(integer)
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping audit year-retention grants (roles absent — see the ext role block NOTICE)';
    END IF;
END
$grants$;
