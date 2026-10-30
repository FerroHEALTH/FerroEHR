-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the archetype columns' statistics (#4437), the counterpart of
-- clinical migration 0021.
--
-- An AQL archetype predicate binds rm_type, arch_entity, arch_major and
-- arch_concept together, and each archetype fixes the other three columns.
-- The planner multiplies the four selectivities by default and, on a store of
-- 50 000 persons, expected 144 person_identifier CLUSTERs where there are
-- 50 000. The statistics object records the dependency (PostgreSQL 18,
-- "Extended Statistics", https://www.postgresql.org/docs/18/planner-stats.html),
-- and the ANALYZE fills it now instead of at the next automatic analyze.
--
-- No openEHR spec governs planner statistics: our own design.
--
-- Runs with search_path = party, ext, public.

CREATE STATISTICS st_node_hot_archetype (dependencies, mcv)
    ON rm_type, arch_entity, arch_major, arch_concept FROM node_hot;

ANALYZE node_hot;
