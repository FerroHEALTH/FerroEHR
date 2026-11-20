-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- ext: the node fragment readers and the expressions the node indexes read.
--
-- A node row stores only the part of its canonical JSON fragment that its
-- columns do not hold (crate::storage::slim, crate::storage::compact): `_type`
-- is `rm_type`, a lowercase `archetype_node_id` is `archetype`, a plain or
-- simply coded `name` is `name` with `name_code` and `name_terminology`, an
-- ELEMENT value of a common shape is held by the typed value columns, the
-- archetype header, language and encoding of the common shapes by
-- `rm_version`, `language` and `encoding`, and the compact markers say which
-- further keys a row left out. node_fragment and the readers it calls rebuild
-- the removed keys, so every reader sees the fragment the decomposer was
-- handed.
--
-- node_search_text, node_magnitude, node_code and node_number each yield the
-- one value an AQL comparison of an ELEMENT value reads, from whichever place
-- holds it (the typed value columns or the stored fragment), so one expression
-- index per store serves the comparison. They are IMMUTABLE, which an index
-- expression requires: they read their arguments through jsonb operators and
-- the numeric input function alone (PostgreSQL 18, "Function Volatility
-- Categories", https://www.postgresql.org/docs/18/xfunc-volatility.html).
--
-- The readers that build jsonb are STABLE, not IMMUTABLE: jsonb_build_object
-- is STABLE, and a LANGUAGE sql function is inlined into the calling statement
-- only when it calls nothing more volatile than it declares itself
-- (PostgreSQL 18, "Inlining of SQL Functions",
-- https://wiki.postgresql.org/wiki/Inlining_of_SQL_functions).
--
-- No openEHR spec governs storage layout or access paths: our own design.
--
-- Runs with search_path = ext.

-- The name a row's fragment no longer carries: a DV_TEXT, or a DV_CODED_TEXT
-- when the row carries a coded name; NULL for a row with no name.
CREATE FUNCTION node_name(name text, name_code text, name_terminology text)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT CASE
    WHEN name IS NULL THEN NULL
    WHEN name_code IS NULL THEN jsonb_build_object('_type', 'DV_TEXT', 'value', name)
    ELSE jsonb_build_object(
        '_type', 'DV_CODED_TEXT',
        'value', name,
        'defining_code', jsonb_build_object(
            '_type', 'CODE_PHRASE',
            'code_string', name_code,
            'terminology_id', jsonb_build_object('_type', 'TERMINOLOGY_ID', 'value', name_terminology)))
END
$$;
COMMENT ON FUNCTION node_name(text, text, text) IS
    'The name a node row''s stored fragment omits, rebuilt from its name columns (our own storage design).';

-- The ELEMENT value the typed value columns hold; NULL when they hold none.
CREATE FUNCTION node_value(
    value_type text, value_num numeric, value_text text, value_code text, value_terminology text)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT CASE value_type
    WHEN 'DV_QUANTITY' THEN
        jsonb_build_object('_type', value_type, 'magnitude', value_num, 'units', value_text)
    WHEN 'DV_COUNT' THEN jsonb_build_object('_type', value_type, 'magnitude', value_num)
    WHEN 'DV_CODED_TEXT' THEN jsonb_build_object(
        '_type', value_type,
        'value', value_text,
        'defining_code', jsonb_build_object(
            '_type', 'CODE_PHRASE',
            'code_string', value_code,
            'terminology_id', jsonb_build_object('_type', 'TERMINOLOGY_ID', 'value', value_terminology)))
    WHEN 'DV_ORDINAL' THEN jsonb_build_object(
        '_type', value_type,
        'value', value_num,
        'symbol', jsonb_build_object(
            '_type', 'DV_CODED_TEXT',
            'value', value_text,
            'defining_code', jsonb_build_object(
                '_type', 'CODE_PHRASE',
                'code_string', value_code,
                'terminology_id', jsonb_build_object('_type', 'TERMINOLOGY_ID', 'value', value_terminology))))
    WHEN 'DV_IDENTIFIER' THEN jsonb_build_object('_type', value_type, 'id', value_text)
    WHEN 'DV_BOOLEAN' THEN jsonb_build_object('_type', value_type, 'value', value_text::boolean)
    ELSE CASE WHEN value_type IS NOT NULL
        THEN jsonb_build_object('_type', value_type, 'value', value_text) END
END
$$;
COMMENT ON FUNCTION node_value(text, numeric, text, text, text) IS
    'The ELEMENT value a node row''s typed value columns hold, as canonical JSON (our own storage design).';

-- The archetype node id a row's fragment no longer carries: with an
-- rm_version the lowercase archetype recased (`openehr-ehr-observation.x.v1`
-- becomes `openEHR-EHR-OBSERVATION.x.v1`, crate::storage::slim::recased),
-- otherwise the archetype as stored. `upper` under the "C" collation maps
-- ASCII alone, whatever the database locale (PostgreSQL 18, §24.2.2.1).
CREATE FUNCTION node_archetype_node_id(archetype text, rm_version text)
RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
SELECT CASE
    WHEN rm_version IS NULL OR archetype IS NULL THEN archetype
    ELSE 'openEHR-'
        || upper(split_part(substr(archetype, 9), '.', 1) COLLATE "C")
        || substr(archetype, 9 + length(split_part(substr(archetype, 9), '.', 1)))
END
$$;
COMMENT ON FUNCTION node_archetype_node_id(text, text) IS
    'The archetype_node_id a node row''s stored fragment omits, rebuilt from its archetype and rm_version columns (our own storage design).';

CREATE FUNCTION node_archetype_details(archetype text, rm_version text, template_id text)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT CASE WHEN rm_version IS NOT NULL AND archetype IS NOT NULL THEN jsonb_build_object(
    '_type', 'ARCHETYPED',
    'archetype_id', jsonb_build_object(
        '_type', 'ARCHETYPE_ID', 'value', ext.node_archetype_node_id(archetype, rm_version)),
    'rm_version', rm_version)
    || CASE WHEN template_id IS NULL THEN '{}'::jsonb
            ELSE jsonb_build_object('template_id',
                jsonb_build_object('_type', 'TEMPLATE_ID', 'value', template_id)) END
END
$$;
COMMENT ON FUNCTION node_archetype_details(text, text, text) IS
    'The archetype_details a node row''s stored fragment omits, rebuilt from its archetype and rm_version columns and, on a root, the version''s template_id (our own storage design).';

-- The CODE_PHRASE of code in terminology; NULL without a code.
CREATE FUNCTION node_code_phrase(terminology text, code text)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT CASE WHEN code IS NOT NULL THEN jsonb_build_object(
    '_type', 'CODE_PHRASE',
    'terminology_id', jsonb_build_object('_type', 'TERMINOLOGY_ID', 'value', terminology),
    'code_string', code)
END
$$;
COMMENT ON FUNCTION node_code_phrase(text, text) IS
    'The CODE_PHRASE of a code in a terminology, for the language and encoding a node row''s stored fragment omits (our own storage design).';

-- The OBJECT_VERSION_ID of a version row, by the law the AQL engine and the
-- idx_version_hot_uid index use: `object_id '::' creating_system_id '::'
-- version_tree_id`, the tree id `N` on a trunk row and `N.B.V` on a branch
-- row (BASE base_types §OBJECT_VERSION_ID, §VERSION_TREE_ID).
CREATE FUNCTION version_uid(
    vo_id uuid, creating_system_id text, trunk_version integer,
    branch_number integer, branch_version integer)
RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$
SELECT vo_id::text || '::' || creating_system_id || '::' || trunk_version::text
    || CASE WHEN branch_number > 0
            THEN '.' || branch_number::text || '.' || branch_version::text
            ELSE '' END
$$;
COMMENT ON FUNCTION version_uid(uuid, text, integer, integer, integer) IS
    'The OBJECT_VERSION_ID of a version row from its identity columns, the text the AQL engine synthesizes (our own storage design).';

-- The DV_DATE_TIME a compact origin or time stands for: the stored value
-- string wrapped back into the object of exactly `_type` and `value`.
CREATE FUNCTION node_date_time(value jsonb)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT jsonb_build_object('_type', 'DV_DATE_TIME', 'value', value)
$$;
COMMENT ON FUNCTION node_date_time(jsonb) IS
    'The DV_DATE_TIME a node row''s compact origin or time stands for, as canonical JSON (our own storage design).';

CREATE FUNCTION node_fragment(
    data jsonb, rm_type text, archetype text,
    name text, name_code text, name_terminology text,
    value_type text, value_num numeric, value_text text, value_code text, value_terminology text,
    rm_version text, language text, encoding text,
    origin_compact boolean, time_compact boolean, party_self boolean, uid_derived boolean,
    version_uid text, template_id text)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT COALESCE(data, '{}'::jsonb)
    || jsonb_build_object('_type', rm_type)
    || CASE WHEN archetype IS NULL OR data ? 'archetype_node_id' THEN '{}'::jsonb
            ELSE jsonb_build_object('archetype_node_id', ext.node_archetype_node_id(archetype, rm_version)) END
    || CASE WHEN rm_version IS NULL OR archetype IS NULL OR data ? 'archetype_details' THEN '{}'::jsonb
            ELSE jsonb_build_object('archetype_details',
                ext.node_archetype_details(archetype, rm_version, template_id)) END
    || CASE WHEN language IS NULL OR data ? 'language' THEN '{}'::jsonb
            ELSE jsonb_build_object('language', ext.node_code_phrase('ISO_639-1', language)) END
    || CASE WHEN encoding IS NULL OR data ? 'encoding' THEN '{}'::jsonb
            ELSE jsonb_build_object('encoding', ext.node_code_phrase('IANA_character-sets', encoding)) END
    || CASE WHEN name IS NULL OR data ? 'name' THEN '{}'::jsonb
            ELSE jsonb_build_object('name', ext.node_name(name, name_code, name_terminology)) END
    || CASE WHEN value_type IS NULL THEN '{}'::jsonb
            ELSE jsonb_build_object('value',
                ext.node_value(value_type, value_num, value_text, value_code, value_terminology)) END
    || CASE WHEN origin_compact THEN jsonb_build_object('origin', ext.node_date_time(data -> 'origin'))
            ELSE '{}'::jsonb END
    || CASE WHEN time_compact THEN jsonb_build_object('time', ext.node_date_time(data -> 'time'))
            ELSE '{}'::jsonb END
    || CASE WHEN party_self THEN jsonb_build_object('subject', '{"_type": "PARTY_SELF"}'::jsonb)
            ELSE '{}'::jsonb END
    || CASE WHEN uid_derived AND version_uid IS NOT NULL
            THEN jsonb_build_object('uid',
                jsonb_build_object('_type', 'OBJECT_VERSION_ID', 'value', version_uid))
            ELSE '{}'::jsonb END
$$;
COMMENT ON FUNCTION node_fragment(jsonb, text, text, text, text, text, text, numeric, text, text, text, text, text, text, boolean, boolean, boolean, boolean, text, text) IS
    'The whole canonical fragment of a node row: its stored data with every key its columns, its markers and its version hold put back (our own storage design).';

-- The search text of an ELEMENT value.
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

-- The magnitude of an ELEMENT value.
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

-- The code of an ELEMENT value.
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

-- The number of an ELEMENT value's `value` member.
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
