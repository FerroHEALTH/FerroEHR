-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The node header readers (#4525).
--
-- Below the versioned object's root, a node row keeps its archetype header,
-- language and encoding in the columns rm_version, language and encoding when
-- they have the common shape (crate::storage::slim::Header). These functions
-- rebuild them, and node_fragment takes the three columns too, so every
-- reader still sees the fragment the decomposer was handed. No openEHR spec
-- governs storage layout: our own design.
--
-- Runs with search_path = ext.

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

-- The archetype_details a row's fragment no longer carries; NULL when the
-- row keeps them in data.
CREATE FUNCTION node_archetype_details(archetype text, rm_version text)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT CASE WHEN rm_version IS NOT NULL AND archetype IS NOT NULL THEN jsonb_build_object(
    '_type', 'ARCHETYPED',
    'archetype_id', jsonb_build_object(
        '_type', 'ARCHETYPE_ID', 'value', ext.node_archetype_node_id(archetype, rm_version)),
    'rm_version', rm_version)
END
$$;

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

-- The whole canonical fragment of a node row, now with the header columns.
-- Every caller passes them, so the form without them goes.
DROP FUNCTION node_fragment(jsonb, text, text, text, text, text, text, numeric, text, text, text);

CREATE FUNCTION node_fragment(
    data jsonb, rm_type text, archetype text,
    name text, name_code text, name_terminology text,
    value_type text, value_num numeric, value_text text, value_code text, value_terminology text,
    rm_version text, language text, encoding text)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT COALESCE(data, '{}'::jsonb)
    || jsonb_build_object('_type', rm_type)
    || CASE WHEN archetype IS NULL OR data ? 'archetype_node_id' THEN '{}'::jsonb
            ELSE jsonb_build_object('archetype_node_id', ext.node_archetype_node_id(archetype, rm_version)) END
    || CASE WHEN rm_version IS NULL OR archetype IS NULL OR data ? 'archetype_details' THEN '{}'::jsonb
            ELSE jsonb_build_object('archetype_details', ext.node_archetype_details(archetype, rm_version)) END
    || CASE WHEN language IS NULL OR data ? 'language' THEN '{}'::jsonb
            ELSE jsonb_build_object('language', ext.node_code_phrase('ISO_639-1', language)) END
    || CASE WHEN encoding IS NULL OR data ? 'encoding' THEN '{}'::jsonb
            ELSE jsonb_build_object('encoding', ext.node_code_phrase('IANA_character-sets', encoding)) END
    || CASE WHEN name IS NULL OR data ? 'name' THEN '{}'::jsonb
            ELSE jsonb_build_object('name', ext.node_name(name, name_code, name_terminology)) END
    || CASE WHEN value_type IS NULL THEN '{}'::jsonb
            ELSE jsonb_build_object('value',
                ext.node_value(value_type, value_num, value_text, value_code, value_terminology)) END
$$;

COMMENT ON FUNCTION node_archetype_node_id(text, text) IS
    'The archetype_node_id a node row''s stored fragment omits, rebuilt from its archetype and rm_version columns (our own storage design).';
COMMENT ON FUNCTION node_archetype_details(text, text) IS
    'The archetype_details a node row''s stored fragment omits, rebuilt from its archetype and rm_version columns (our own storage design).';
COMMENT ON FUNCTION node_code_phrase(text, text) IS
    'The CODE_PHRASE of a code in a terminology, for the language and encoding a node row''s stored fragment omits (our own storage design).';
COMMENT ON FUNCTION node_fragment(jsonb, text, text, text, text, text, text, numeric, text, text, text, text, text, text) IS
    'The whole canonical fragment of a node row: its stored data with every key its columns hold put back (our own storage design).';
