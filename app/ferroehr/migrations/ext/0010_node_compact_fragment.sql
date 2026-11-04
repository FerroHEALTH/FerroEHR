-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- The node compact readers (#4904).
--
-- A node row stores a DV_DATE_TIME origin or time of the plain shape as its
-- value string, leaves out a bare PARTY_SELF subject, and the root row leaves
-- out the uid its version row holds and keeps its archetype header, language
-- and encoding in the columns a row below the root used since 0007, with the
-- version's template_id put back into the header (crate::storage::compact,
-- crate::storage::slim). Four marker columns say which keys a row left out;
-- these functions put them back, and node_fragment takes the markers and the
-- version's uid and template_id too, so every reader still sees the fragment
-- the decomposer was handed. No openEHR spec governs storage layout: our own
-- design.
--
-- Runs with search_path = ext.

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

-- The DV_DATE_TIME a compact origin or time stands for: the stored value
-- string wrapped back into the object of exactly `_type` and `value`.
CREATE FUNCTION node_date_time(value jsonb)
RETURNS jsonb
LANGUAGE sql STABLE PARALLEL SAFE AS $$
SELECT jsonb_build_object('_type', 'DV_DATE_TIME', 'value', value)
$$;

-- The archetype_details a row's fragment no longer carries, now with the
-- template_id a root's header names; NULL when the row keeps them in data.
-- Every caller passes the template, so the form without it goes.
DROP FUNCTION node_archetype_details(text, text);

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

-- The whole canonical fragment of a node row, now with the compact markers
-- and, for a root row, the version's uid and template_id (NULL on every other
-- row). Every caller passes them, so the form without them goes.
DROP FUNCTION node_fragment(
    jsonb, text, text, text, text, text, text, numeric, text, text, text, text, text, text);

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

COMMENT ON FUNCTION version_uid(uuid, text, integer, integer, integer) IS
    'The OBJECT_VERSION_ID of a version row from its identity columns, the text the AQL engine synthesizes (our own storage design).';
COMMENT ON FUNCTION node_date_time(jsonb) IS
    'The DV_DATE_TIME a node row''s compact origin or time stands for, as canonical JSON (our own storage design).';
COMMENT ON FUNCTION node_archetype_details(text, text, text) IS
    'The archetype_details a node row''s stored fragment omits, rebuilt from its archetype and rm_version columns and, on a root, the version''s template_id (our own storage design).';
COMMENT ON FUNCTION node_fragment(jsonb, text, text, text, text, text, text, numeric, text, text, text, text, text, text, boolean, boolean, boolean, boolean, text, text) IS
    'The whole canonical fragment of a node row: its stored data with every key its columns, its markers and its version hold put back (our own storage design).';
