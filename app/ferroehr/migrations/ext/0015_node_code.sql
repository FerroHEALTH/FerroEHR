-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The code of an ELEMENT value (#6033).
--
-- An AQL filter on the code of a coded value (`WHERE
-- o/data[at0001]/events[at0002]/data[at0003]/items[at0005]/value/defining_code/code_string
-- = $code`) read every node of the archetype and compared the code last,
-- because the code lives in one of two places: in value_code when the typed
-- value columns hold the value (crate::storage::slim), otherwise inside the
-- stored `data` fragment. This function yields the one code the comparison
-- reads, from whichever place holds it, so one expression index per store
-- (idx_node_hot_value_code) serves the filter.
--
-- The branches are the ones the AQL engine reads the two leaves through
-- (crate::aql::sql::value, `typed_reads`): value_code on a row whose columns
-- hold a DV_CODED_TEXT (its `defining_code.code_string`) or a DV_ORDINAL (its
-- `symbol.defining_code.code_string`), the same member of the fragment on a
-- row that keeps its value in `data`, and NULL on every other row. No RM
-- class carries both a `defining_code` and a `symbol` (RM data_types
-- DV_CODED_TEXT, DV_ORDINAL, DV_SCALE), so the fragment branch reads whichever
-- the value has.
--
-- IMMUTABLE, which an index expression requires: it reads its arguments
-- through jsonb operators alone (PostgreSQL 18, "Function Volatility
-- Categories", https://www.postgresql.org/docs/18/xfunc-volatility.html).
--
-- No openEHR spec governs storage layout or access paths: our own design.
--
-- Runs with search_path = ext.

CREATE FUNCTION node_code(value_type text, value_code text, data jsonb)
RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
SELECT CASE
    WHEN value_type IN ('DV_CODED_TEXT', 'DV_ORDINAL') THEN value_code
    WHEN value_type IS NULL
        THEN coalesce(
            data -> 'value' -> 'defining_code' ->> 'code_string',
            data -> 'value' -> 'symbol' -> 'defining_code' ->> 'code_string')
END
$$;

COMMENT ON FUNCTION node_code(text, text, jsonb) IS
    'The code an AQL comparison of the code of an ELEMENT value reads, from value_code or the stored fragment; the expression idx_node_hot_value_code indexes (our own storage design).';
