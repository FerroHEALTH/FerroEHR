-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: an AQL equality on the code of an ELEMENT value enters through an
-- index (#6033).
--
-- RENDERED FROM app/ferroehr/migrations/templates/node_value_code.sql.in
-- by crate::storage::ddl_template, for both pseudonymisation domains; edit
-- the template, never this file.
--
-- A population query filtered on a code (`WHERE
-- .../items[at0005]/value/defining_code/code_string = $code`) had no access
-- path: the planner read every node of the archetype, walked to each ELEMENT
-- and compared the code last, whatever the filter kept. The AQL engine now
-- adds, to such an equality, the condition that some ELEMENT of the same
-- version carries the code (crate::aql::sql::anchor; ext.node_code), and this
-- index serves it. It leads with the node id, which the path step names, and
-- is partial on the ELEMENT rows that carry a code (PostgreSQL 18, "Partial
-- Indexes", https://www.postgresql.org/docs/18/indexes-partial.html). The
-- statistics object gives the planner the spread of the expression, which it
-- takes from no partial index (PostgreSQL 18, "Extended Statistics",
-- https://www.postgresql.org/docs/18/planner-stats.html). The ANALYZE fills
-- it now instead of at the next automatic analyze.
--
-- The index costs write-ahead log on every commit that carries a coded
-- value: 72 bytes an entry, 1.08% on the IPS composition (36 entries),
-- measured on #6033 before the index was adopted.
--
-- No openEHR spec governs indexes or planner statistics: our own design.

CREATE INDEX idx_node_hot_value_code
    ON node_hot (archetype, ext.node_code(value_type, value_code, data))
    WHERE rm_type = 'ELEMENT'
      AND ext.node_code(value_type, value_code, data) IS NOT NULL;

CREATE STATISTICS st_node_hot_value_code
    ON (ext.node_code(value_type, value_code, data)) FROM node_hot;

ANALYZE node_hot;
