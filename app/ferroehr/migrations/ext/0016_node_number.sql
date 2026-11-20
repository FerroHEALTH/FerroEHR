-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The number of an ELEMENT value's `value` member (#6090).
--
-- An AQL equality on `value/value` with a string that reads as a number
-- (`= '19500101'`) had no access path: on a DV_ORDINAL or a DV_SCALE the
-- member is a number (RM data_types DV_ORDINAL.value: Integer, DV_SCALE.value:
-- Real), the comparison matches the text it renders as, and
-- ext.node_search_text yields the symbol of such a value instead. The number
-- lives in one of two places: in value_num when the typed value columns hold
-- a DV_ORDINAL (crate::storage::slim), otherwise inside the stored `data`
-- fragment. This function yields that one number, so one expression index
-- per store (idx_node_hot_value_number) serves the equality.
--
-- The branches are the ones the AQL engine reads the leaf through
-- (crate::aql::sql::value, `typed_reads`): value_num on a row whose columns
-- hold a DV_ORDINAL, the fragment's `value.value` when it is a JSON number on
-- a row that keeps its value in `data`, and NULL on every other row. A
-- number the engine renders as text renders as this numeric does, so an
-- equality with a text holds only where this number equals the text read as
-- a number.
--
-- IMMUTABLE, which an index expression requires: it reads its arguments
-- through jsonb operators and the numeric input function alone (PostgreSQL
-- 18, "Function Volatility Categories",
-- https://www.postgresql.org/docs/18/xfunc-volatility.html).
--
-- No openEHR spec governs storage layout or access paths: our own design.
--
-- Runs with search_path = ext.

CREATE FUNCTION node_number(value_type text, value_num numeric, data jsonb)
RETURNS numeric
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
SELECT CASE
    WHEN value_type = 'DV_ORDINAL' THEN value_num
    WHEN value_type IS NULL
         AND jsonb_typeof(data -> 'value' -> 'value') = 'number'
        THEN (data -> 'value' ->> 'value')::numeric
END
$$;

COMMENT ON FUNCTION node_number(text, numeric, jsonb) IS
    'The number an AQL equality on the value member of an ELEMENT value reads when that member is a number, from value_num or the stored fragment; the expression idx_node_hot_value_number indexes (our own storage design).';
