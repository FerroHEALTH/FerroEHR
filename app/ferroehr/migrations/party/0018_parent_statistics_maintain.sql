-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the serving role may ANALYZE the partitioned parents (#4453).
--
-- Autovacuum never analyzes a partitioned table (PostgreSQL 18, §24.1.3
-- Updating Planner Statistics), so the server refreshes the statistics of
-- node, version and vo_attestation hourly (crate::db::parent_statistics), on
-- the pool of the role that serves the store. ANALYZE needs MAINTAIN on the
-- table, which a serving role that does not own it lacks (PostgreSQL 18,
-- "Privileges").
--
-- MAINTAIN also allows VACUUM, CLUSTER, REINDEX and LOCK TABLE on these three
-- tables and on no other. The role can already write every row of them, so
-- the added reach is the locks and the index rebuild those commands take; it
-- is granted because a planner left without these statistics walked whole
-- indexes (#4438), and an out-of-band job is one more thing a deployment can
-- forget. The readers get nothing.
--
-- No openEHR spec governs database grants: our own operational design.
--
-- Runs with search_path = party, ext, public.
DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        GRANT MAINTAIN ON party.node, party.version, party.vo_attestation
            TO ferroehr_party;
    ELSE
        RAISE NOTICE 'skipping the party MAINTAIN grant (roles absent — see the ext role block NOTICE)';
    END IF;
END $$;
