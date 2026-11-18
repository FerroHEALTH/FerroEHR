-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The magnitude of an ELEMENT value (#6022).
--
-- An AQL range filter on a magnitude (`WHERE
-- o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/magnitude
-- > $x`) read every node of the archetype and compared the number last,
-- because the number lives in one of two places: in value_num when the value
-- is a DV_QUANTITY of a magnitude and units alone or a DV_COUNT
-- (crate::storage::slim), otherwise inside the stored `data` fragment, as a
-- DV_QUANTITY with a precision or a normal range does. This function yields
-- the one number the comparison reads, from whichever place holds it, so one
-- expression index per store (idx_node_hot_value_magnitude) serves the
-- filter.
--
-- The branches are the ones the AQL engine reads the leaf through
-- (crate::aql::sql::value, `typed_reads`): value_num on a row whose columns
-- hold a DV_QUANTITY or a DV_COUNT, the fragment's `value.magnitude` on a
-- row that keeps its value in `data`, and NULL on every other row. A stored
-- magnitude is a JSON number (RM data_types DV_QUANTITY.magnitude: Real,
-- DV_COUNT.magnitude: Integer64; ITS-JSON writes both as numbers), and the
-- function reads no other JSON type, so no row can fail an insert on it.
--
-- IMMUTABLE, which an index expression requires: it reads its arguments
-- through jsonb operators and the numeric input function alone (PostgreSQL
-- 18, "Function Volatility Categories",
-- https://www.postgresql.org/docs/18/xfunc-volatility.html).
--
-- No openEHR spec governs storage layout or access paths: our own design.
--
-- Runs with search_path = ext.

CREATE FUNCTION node_magnitude(value_type text, value_num numeric, data jsonb)
RETURNS numeric
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
SELECT CASE
    WHEN value_type IN ('DV_QUANTITY', 'DV_COUNT') THEN value_num
    WHEN value_type IS NULL
         AND jsonb_typeof(data -> 'value' -> 'magnitude') = 'number'
        THEN (data -> 'value' ->> 'magnitude')::numeric
END
$$;

COMMENT ON FUNCTION node_magnitude(text, numeric, jsonb) IS
    'The number an AQL comparison of the magnitude of an ELEMENT value reads, from value_num or the stored fragment; the expression idx_node_hot_value_magnitude indexes (our own storage design).';
