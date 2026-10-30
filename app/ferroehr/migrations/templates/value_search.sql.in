-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: an AQL lookup by an ELEMENT value enters through an index (#4437).
--
-- RENDERED FROM app/ferroehr/migrations/templates/value_search.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- A lookup by a value (`WHERE id/items[at0001]/value/id = $v`) had no access
-- path: the planner read every node of the archetype, probed its version,
-- head and root, and tested the value last, about 16 buffers a person, 350 ms
-- at 50 000 persons. The AQL engine now adds, to such an equality, the
-- condition that some ELEMENT of the same version carries the value as its
-- search text (crate::aql::sql::condition; ext.node_search_text), and this
-- index serves it. The index is partial on the ELEMENT rows, the only rows
-- with a value, and the statistics object gives the planner the distinct
-- values of the expression, which it takes from no partial index
-- (PostgreSQL 18, "Extended Statistics",
-- https://www.postgresql.org/docs/18/planner-stats.html). The ANALYZE fills
-- it now instead of at the next automatic analyze.
--
-- No openEHR spec governs indexes or planner statistics: our own design.

CREATE INDEX idx_node_hot_value_search
    ON node_hot (ext.node_search_text(value_type, value_text, data))
    WHERE rm_type = 'ELEMENT';

CREATE STATISTICS st_node_hot_value_search
    ON (ext.node_search_text(value_type, value_text, data)) FROM node_hot;

ANALYZE node_hot;
