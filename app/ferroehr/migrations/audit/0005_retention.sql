-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: retention.
--
-- The repository keeps its records for a configured horizon and reaps what is
-- older. Reaping is the ONE deletion the append-only triggers admit, and it
-- records what it removed: a reaped span becomes an audit_chain_gap row
-- carrying the link across it, so a gap in the chain has a stated reason
-- instead of looking like tampering.
--
-- The horizon is a floor question as much as a ceiling one: national rules set
-- a minimum a log must be kept for, and one national rule sets a maximum. The
-- floor and the ceiling are configuration, not data, so they live beside the
-- horizon they bound rather than in this schema
-- (`crate::system_log::config::retention_floor_days` /
-- `retention_ceiling_days`, checked at boot by `AppConfig::validate_audit`):
-- SGB V § 309 Abs. 1 and Abs. 3 ask the § 307 controllers of a telematics
-- application to keep access logs for the three-year limitation period and to
-- delete them "unverzüglich" after it (docs/law/de/sgb-v/BJNR024820988.xml),
-- while the Dutch and Swiss rules set minima. This function reaps to whatever
-- horizon the boot-validated configuration hands it.
--
-- No openEHR spec governs audit retention: our own design/extension.
--
-- Runs with search_path = audit, ext, public.

-- ── retention reaping ────────────────────────────────────────────────────────

-- Deletes exactly the records older than the horizon — the retention rule is
-- unchanged — and records every removed span as a tombstone so the resulting
-- gaps stay accounted for. Adjacent spans are collapsed, so ordinary
-- oldest-first reaping converges on a single tombstone. Returns the number of
-- records removed.
CREATE FUNCTION audit.reap_audit_events(p_retention_days integer) RETURNS bigint
    LANGUAGE plpgsql SECURITY DEFINER
    SET search_path = audit, pg_catalog
    AS $$
DECLARE
    cutoff  timestamptz;
    removed bigint;
    left_from  bigint;
    right_from bigint;
BEGIN
    IF p_retention_days IS NULL OR p_retention_days <= 0 THEN
        RETURN 0;
    END IF;
    cutoff := now() - make_interval(days => p_retention_days);

    -- Serialize against the chain writer: a record must not be appended while
    -- the tombstones for this reap are being computed.
    PERFORM 1 FROM audit.audit_chain_state WHERE singleton FOR UPDATE;

    PERFORM set_config('ferroehr.audit_reaping', 'on', true);
    -- The island-numbering trick (position minus its ordinal is constant across
    -- a run of consecutive positions) turns the removed positions into spans.
    WITH gone AS (
        DELETE FROM audit.audit_event WHERE recorded_at < cutoff
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

    -- Collapse abutting tombstones so a repeatedly-reaped repository keeps one.
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

COMMENT ON FUNCTION audit.reap_audit_events(integer) IS
    'Retention reaping: removes records past the horizon and tombstones the chain positions they occupied — the only sanctioned deletion path.';
