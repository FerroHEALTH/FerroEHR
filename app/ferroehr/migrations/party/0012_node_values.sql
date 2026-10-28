-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- node: the typed value columns, and a fragment that keeps only what no
-- column holds (#4306).
--
-- RENDERED FROM app/ferroehr/migrations/templates/node_values.sql.in by
-- crate::storage::ddl_template, for both pseudonymisation domains; edit the
-- template, never this file.
--
-- A node row's `data` no longer repeats what the row carries as columns
-- (crate::storage::slim): `_type` is `rm_type`, a lowercase
-- `archetype_node_id` is `archetype`, a plain or simply coded `name` is
-- `name` with `name_code` and `name_terminology`, and an ELEMENT value of a
-- common shape moves to the typed value columns below. A fragment with
-- nothing left is NULL. The ext functions node_name, node_value and
-- node_fragment rebuild the omitted keys on read, so every reader sees the
-- canonical fragment. A row written before this migration keeps its whole
-- fragment and reads the same, because each reader prefers a key `data`
-- carries. No openEHR spec governs storage layout: our own design (RM common
-- master06-change_control_package.adoc §Overview admits any physical
-- representation).
--
-- No index is created on the typed columns: no measured AQL statement would
-- use one, and each costs bytes on every ELEMENT row. They are index-legal.

ALTER TABLE node ALTER COLUMN data DROP NOT NULL;

-- The RM type of an ELEMENT value the typed columns hold (`DV_QUANTITY`, …),
-- never an alias; NULL when `data` holds the value or the row is no ELEMENT.
ALTER TABLE node ADD COLUMN value_type text;
-- The number of a DV_QUANTITY or DV_COUNT magnitude, or a DV_ORDINAL value,
-- with the scale the canonical JSON carried.
ALTER TABLE node ADD COLUMN value_num numeric;
-- The text: a DV_QUANTITY's units, a DV_ORDINAL symbol's value, a
-- DV_IDENTIFIER's id, `true` or `false` for a DV_BOOLEAN, otherwise `value`.
ALTER TABLE node ADD COLUMN value_text text;
-- The code_string of a DV_CODED_TEXT, or of a DV_ORDINAL symbol.
ALTER TABLE node ADD COLUMN value_code text;
-- The terminology_id of that code.
ALTER TABLE node ADD COLUMN value_terminology text;

COMMENT ON COLUMN node.data IS 'The node''s canonical openEHR JSON fragment (ITS-JSON), structure children pruned, without the keys the row''s columns hold (_type, a lowercase archetype_node_id, a plain or simply coded name, a typed ELEMENT value); NULL when nothing remains. ext.node_fragment rebuilds the whole fragment. Our own storage design.';
COMMENT ON COLUMN node.value_type IS 'The RM type of the ELEMENT value the typed value columns hold; NULL when data holds the value. Our own storage design.';
COMMENT ON COLUMN node.value_num IS 'A DV_QUANTITY or DV_COUNT magnitude, or a DV_ORDINAL value, as the canonical JSON carried it.';
COMMENT ON COLUMN node.value_text IS 'A DV_QUANTITY''s units, a DV_ORDINAL symbol''s value, a DV_IDENTIFIER''s id, true or false for a DV_BOOLEAN, otherwise the value''s value.';
COMMENT ON COLUMN node.value_code IS 'The code_string of a DV_CODED_TEXT value or of a DV_ORDINAL symbol.';
COMMENT ON COLUMN node.value_terminology IS 'The terminology_id of value_code.';
