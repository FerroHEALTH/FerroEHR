-- SPDX-FileCopyrightText: Vernum Projecten B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the grants over the finished relations.
--
-- One file per schema carries every grant, so the relation files stay
-- declarations of shape and there is exactly one file to read to see who may
-- do what. The audit repository is not a pseudonymisation domain: it is
-- granted to the clinical pair, which records an event, stamps it forwarded,
-- runs the retention reaper and verifies the chain, and to nobody else. What
-- the grants withhold is as deliberate as what they give — no role may UPDATE
-- a record's content or DELETE one; the reaper is the only sanctioned removal
-- path and it is a function.
--
-- BASE architecture_overview master07 §Access logging requires the trail; no
-- openEHR spec governs database roles — our own design/extension.

-- ── least-privilege grants ───────────────────────────────────────────────────

-- The `audit` schema had no grants at all, so the layered role architecture
-- (created by the ehr/ext baselines) could not reach it: only the owner could
-- write audit records. These grants give the runtime role exactly what the
-- server does — insert a record, stamp it delivered, read it back for ITI-81 —
-- and nothing that can rewrite or remove one.
DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT USAGE ON SCHEMA audit TO ferroehr_clinical, ferroehr_clinical_reader;

        -- Revoke first: a table-level REVOKE also removes column-level grants,
        -- so the column grant below has to come after it (PostgreSQL 18 docs,
        -- REVOKE: https://www.postgresql.org/docs/18/sql-revoke.html).
        REVOKE ALL ON audit_event
            FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT, INSERT ON audit_event TO ferroehr_clinical;
        GRANT UPDATE (delivered_syslog_at, delivered_fhir_feed_at)
            ON audit_event TO ferroehr_clinical;
        GRANT SELECT ON audit_event TO ferroehr_clinical_reader;

        REVOKE ALL ON audit_chain_state, audit_chain_gap
            FROM ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT ON audit_chain_state, audit_chain_gap
            TO ferroehr_clinical, ferroehr_clinical_reader;

        GRANT EXECUTE ON FUNCTION audit.verify_audit_chain()
            TO ferroehr_clinical, ferroehr_clinical_reader;

    ELSE
        RAISE NOTICE 'skipping audit chain grants (roles absent — see the ext role block NOTICE)';
    END IF;
END
$grants$;

DO $grants$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        REVOKE ALL ON FUNCTION audit.reap_audit_events(integer) FROM PUBLIC;
        GRANT EXECUTE ON FUNCTION audit.reap_audit_events(integer)
            TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping audit retention grants (roles absent — see the ext role block NOTICE)';
    END IF;
END
$grants$;
