-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: an AQL equality of `value/value` with a number-shaped string enters
-- through an index (#6090).
--
-- RENDERED FROM app/ferroehr/migrations/templates/node_value_number.sql.in
-- by crate::storage::ddl_template, for both pseudonymisation domains; edit
-- the template, never this file.
--
-- A query filtered on `.../value/value = '19500101'` or a `matches` list with
-- such a member had no access path, since the value of a DV_ORDINAL or a
-- DV_SCALE is a number no text index holds. The AQL engine now adds, to such
-- an equality, the condition that some ELEMENT of the same version carries
-- the text as its search text or as its number (crate::aql::sql::anchor;
-- ext.node_search_text, ext.node_number), and this index serves the second.
-- It leads with the node id, which the path step names, and is partial on the
-- ELEMENT rows whose value member is a number (PostgreSQL 18, "Partial
-- Indexes", https://www.postgresql.org/docs/18/indexes-partial.html).
--
-- No openEHR spec governs indexes: our own design.

CREATE INDEX idx_node_hot_value_number
    ON node_hot (archetype, ext.node_number(value_type, value_num, data))
    WHERE rm_type = 'ELEMENT'
      AND ext.node_number(value_type, value_num, data) IS NOT NULL;

ANALYZE node_hot;
