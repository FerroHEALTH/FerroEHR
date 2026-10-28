-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The node fragment readers and the third storage generation (#4306).
--
-- A node row stores only the part of its canonical JSON fragment that its
-- columns do not hold (crate::storage::slim): `_type` is `rm_type`, a
-- lowercase `archetype_node_id` is `archetype`, a plain or simply coded
-- `name` is `name` with `name_code` and `name_terminology`, and an ELEMENT
-- value of a common shape is held by the typed value columns. These functions
-- rebuild the removed keys, so every reader sees the fragment the decomposer
-- was handed. No openEHR spec governs storage layout: our own design.
--
-- STABLE, not IMMUTABLE: jsonb_build_object is STABLE, and a LANGUAGE sql
-- function is inlined into the calling statement only when it calls nothing
-- more volatile than it declares itself (PostgreSQL 18, "Inlining of SQL
-- Functions", https://wiki.postgresql.org/wiki/Inlining_of_SQL_functions).

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

-- The whole canonical fragment of a node row: what it stores, with every key
-- its columns hold put back.
CREATE FUNCTION node_fragment(
    data jsonb, rm_type text, archetype text,
    name text, name_code text, name_terminology text,
    value_type text, value_num numeric, value_text text, value_code text, value_terminology text)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT COALESCE(data, '{}'::jsonb)
    || jsonb_build_object('_type', rm_type)
    || CASE WHEN archetype IS NULL OR data ? 'archetype_node_id' THEN '{}'::jsonb
            ELSE jsonb_build_object('archetype_node_id', archetype) END
    || CASE WHEN name IS NULL OR data ? 'name' THEN '{}'::jsonb
            ELSE jsonb_build_object('name', ext.node_name(name, name_code, name_terminology)) END
    || CASE WHEN value_type IS NULL THEN '{}'::jsonb
            ELSE jsonb_build_object('value',
                ext.node_value(value_type, value_num, value_text, value_code, value_terminology)) END
$$;

COMMENT ON FUNCTION node_name(text, text, text) IS
    'The name a node row''s stored fragment omits, rebuilt from its name columns (our own storage design).';
COMMENT ON FUNCTION node_value(text, numeric, text, text, text) IS
    'The ELEMENT value a node row''s typed value columns hold, as canonical JSON (our own storage design).';
COMMENT ON FUNCTION node_fragment(jsonb, text, text, text, text, text, text, numeric, text, text, text) IS
    'The whole canonical fragment of a node row: its stored data with every key its columns hold put back (our own storage design).';

CREATE OR REPLACE FUNCTION storage_generation() RETURNS text
LANGUAGE sql IMMUTABLE PARALLEL SAFE AS $$ SELECT 'generation-3'::text $$;
