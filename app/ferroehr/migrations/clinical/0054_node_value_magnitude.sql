-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: an AQL range filter on the magnitude of an ELEMENT value enters
-- through an index (#6022).
--
-- RENDERED FROM app/ferroehr/migrations/templates/node_value_magnitude.sql.in
-- by crate::storage::ddl_template, for both pseudonymisation domains; edit
-- the template, never this file.
--
-- A population query filtered on a magnitude (`WHERE
-- .../items[at0004]/value/magnitude > $x`) had no access path: the planner
-- read every node of the archetype, walked to each ELEMENT and compared the
-- number last, whatever the filter kept. The AQL engine now adds, to such a
-- comparison, the condition that some ELEMENT of the same version carries a
-- magnitude the comparison keeps (crate::aql::sql::anchor;
-- ext.node_magnitude), and this index serves it. It leads with the node id,
-- which the path step names, so a comparison reads one range of it, and is
-- partial on the ELEMENT rows that carry a magnitude (PostgreSQL 18,
-- "Partial Indexes", https://www.postgresql.org/docs/18/indexes-partial.html).
-- The statistics object gives the planner the spread of the expression,
-- which it takes from no partial index (PostgreSQL 18, "Extended
-- Statistics", https://www.postgresql.org/docs/18/planner-stats.html). The
-- ANALYZE fills it now instead of at the next automatic analyze.
--
-- No openEHR spec governs indexes or planner statistics: our own design.

CREATE INDEX idx_node_hot_value_magnitude
    ON node_hot (archetype, ext.node_magnitude(value_type, value_num, data))
    WHERE rm_type = 'ELEMENT'
      AND ext.node_magnitude(value_type, value_num, data) IS NOT NULL;

CREATE STATISTICS st_node_hot_value_magnitude
    ON (ext.node_magnitude(value_type, value_num, data)) FROM node_hot;

ANALYZE node_hot;
