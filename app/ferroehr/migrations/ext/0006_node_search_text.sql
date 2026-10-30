-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The search text of an ELEMENT value (#4437).
--
-- An AQL lookup by an ELEMENT value (`WHERE id/items[at0001]/value/id = $v`)
-- read every node of the archetype and tested each one, because the value
-- lives in one of two places: in the typed value columns when the value has
-- a common shape (crate::storage::slim), otherwise inside the stored `data`
-- fragment. This function yields the one text an equality on a text leaf of
-- the value compares against, from whichever place holds it, so one
-- expression index per store (idx_node_hot_value_search) serves the lookup.
--
-- The text per value type is the one the typed value columns would hold
-- (crate::storage::slim::TypedValue, `text`): a DV_IDENTIFIER's id, a
-- DV_QUANTITY's units, a DV_ORDINAL's or DV_SCALE's symbol value, and
-- otherwise the `value` member (a DV_TEXT or DV_CODED_TEXT value, a date or
-- time string, a DV_BOOLEAN as `true` or `false`). A row whose columns hold
-- the value answers from `value_text`; a row that keeps the value in `data`
-- answers from the fragment by the value's `_type`. The AQL engine emits the
-- function on exactly the leaf paths whose read is that text
-- (crate::aql::sql::condition), and the storage parity test holds the two
-- branches to the same answer.
--
-- IMMUTABLE, which an index expression requires: it reads its arguments
-- through jsonb operators alone (PostgreSQL 18, "Function Volatility
-- Categories", https://www.postgresql.org/docs/18/xfunc-volatility.html).
--
-- No openEHR spec governs storage layout or access paths: our own design.
--
-- Runs with search_path = ext.

CREATE FUNCTION node_search_text(value_type text, value_text text, data jsonb)
RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
SELECT CASE
    WHEN value_type IS NOT NULL THEN value_text
    ELSE CASE data -> 'value' ->> '_type'
        WHEN 'DV_IDENTIFIER' THEN data -> 'value' ->> 'id'
        WHEN 'DV_QUANTITY' THEN data -> 'value' ->> 'units'
        WHEN 'DV_ORDINAL' THEN data -> 'value' -> 'symbol' ->> 'value'
        WHEN 'DV_SCALE' THEN data -> 'value' -> 'symbol' ->> 'value'
        ELSE data -> 'value' ->> 'value'
    END
END
$$;

COMMENT ON FUNCTION node_search_text(text, text, jsonb) IS
    'The text an AQL equality on a text leaf of an ELEMENT value compares against, from the typed value columns or the stored fragment; the expression idx_node_hot_value_search indexes (our own storage design).';
