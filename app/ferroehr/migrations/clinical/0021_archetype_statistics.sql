-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the archetype columns' statistics.
--
-- An AQL archetype predicate binds rm_type, arch_entity, arch_major and
-- arch_concept together, and each archetype fixes the other three columns: a
-- blood_pressure concept is always an openEHR-EHR-OBSERVATION of one major
-- version. The planner multiplies the four selectivities by default and, on a
-- CDR of 545 067 blood-pressure OBSERVATIONs, expected about 15 000, so it built
-- the whole archetype bitmap of idx_node_hot_arch_subsume once per version
-- instead of probing each version's own nodes. The statistics object
-- records the dependency (PostgreSQL 18, "Extended Statistics",
-- https://www.postgresql.org/docs/18/planner-stats.html), and the ANALYZE
-- fills it now instead of at the next automatic analyze.
--
-- No openEHR spec governs planner statistics: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE STATISTICS st_node_hot_archetype (dependencies, mcv)
    ON rm_type, arch_entity, arch_major, arch_concept FROM node_hot;

ANALYZE node_hot;
