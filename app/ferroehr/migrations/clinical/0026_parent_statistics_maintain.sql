-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the serving role may ANALYZE the partitioned parents (#4453).
--
-- Autovacuum never analyzes a partitioned table (PostgreSQL 18, §24.1.3
-- Updating Planner Statistics), so the server refreshes the statistics of
-- node, version and vo_attestation hourly, and of ehr until autovacuum first
-- analyzes it (crate::db::parent_statistics), on the pool of the role that
-- serves the store. ANALYZE needs MAINTAIN on the table, which a serving role
-- that does not own it lacks (PostgreSQL 18, "Privileges").
--
-- MAINTAIN also allows VACUUM, CLUSTER, REINDEX and LOCK TABLE on these four
-- tables and on no other. The role can already write every row of them, so
-- the added reach is the locks and the index rebuild those commands take; it
-- is granted because a planner left without these statistics walked whole
-- indexes (#4438), and an out-of-band job is one more thing a deployment can
-- forget. The readers get nothing.
--
-- No openEHR spec governs database grants: our own operational design.
--
-- Runs with search_path = clinical, ext, public.
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT MAINTAIN ON clinical.node, clinical.version, clinical.vo_attestation,
            clinical.ehr TO ferroehr_clinical;
    ELSE
        RAISE NOTICE 'skipping the clinical MAINTAIN grant (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
