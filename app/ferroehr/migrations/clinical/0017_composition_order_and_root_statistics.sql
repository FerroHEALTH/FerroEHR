-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the population-wide composition ordering and the root statistics.
--
-- An AQL query over the whole population that orders compositions by
-- c/context/start_time/value and takes the first rows read and sorted every
-- composition before the limit, because idx_node_hot_context_start leads with
-- ehr_id and serves one EHR at a time. idx_node_hot_composition_start orders
-- the COMPOSITION roots by start time alone, so a backward scan of it feeds the
-- limit directly (PostgreSQL 18, "Indexes and ORDER BY",
-- https://www.postgresql.org/docs/18/indexes-ordering.html). COMPOSITION
-- occurs only at a root, so the rm_type predicate is exactly what the query
-- proves.
--
-- The AQL engine binds a COMPOSITION, EHR_STATUS or EHR_ACCESS root at
-- num = 0. Each of those rm_types occurs only at num = 0, a dependency the
-- planner cannot see by default: it multiplies the two selectivities and
-- expects a handful of root rows where there is one per version. The
-- statistics object records the dependency (PostgreSQL 18, "Extended
-- Statistics", https://www.postgresql.org/docs/18/planner-stats.html), and
-- the ANALYZE fills it now instead of at the next automatic analyze.
--
-- No openEHR spec governs indexes or planner statistics: our own design.
--
-- Runs with search_path = clinical, ext, public.

CREATE INDEX idx_node_hot_composition_start ON node_hot (context_start)
    WHERE rm_type = 'COMPOSITION';

CREATE STATISTICS st_node_hot_rm_type_num (dependencies, mcv)
    ON rm_type, num FROM node_hot;

ANALYZE node_hot;
