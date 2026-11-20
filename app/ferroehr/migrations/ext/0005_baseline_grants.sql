-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- ext: the grants over the helper schema.
--
-- Every runtime role may use the schema and execute its functions: the
-- writers call the helpers on the write path (promoted-column population) and
-- the readers on the AQL path. The default privileges extend that to every
-- function a later file creates here. `ext.stamp_posture` is revoked from
-- PUBLIC; the clinical role alone may read the posture it writes, so the
-- subject guard can read it under a separated clinical credential.
--
-- The role grants run only when the runtime roles exist (see the role block
-- of 0001); without them the block is skipped with a NOTICE.
--
-- No openEHR spec governs database roles: our own design/extension.

REVOKE ALL ON FUNCTION stamp_posture(a_key text, a_value text) FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT USAGE ON SCHEMA ext TO ferroehr_clinical;
        GRANT USAGE ON SCHEMA ext TO ferroehr_clinical_reader;
        GRANT USAGE ON SCHEMA ext TO ferroehr_party;
        GRANT USAGE ON SCHEMA ext TO ferroehr_party_reader;
        GRANT EXECUTE ON FUNCTION
            storage_generation(),
            openehr_numeric(t text),
            openehr_date_days(v text),
            openehr_time_seconds(v text),
            openehr_tz_offset_seconds(v text),
            openehr_date_time_seconds(v text),
            openehr_duration_seconds(v text),
            openehr_magnitude(dv jsonb),
            openehr_timestamp(v text),
            aql_not_an_amount(found text),
            node_name(name text, name_code text, name_terminology text),
            node_value(value_type text, value_num numeric, value_text text, value_code text, value_terminology text),
            node_archetype_node_id(archetype text, rm_version text),
            node_archetype_details(archetype text, rm_version text, template_id text),
            node_code_phrase(terminology text, code text),
            version_uid(vo_id uuid, creating_system_id text, trunk_version integer, branch_number integer, branch_version integer),
            node_date_time(value jsonb),
            node_fragment(data jsonb, rm_type text, archetype text, name text, name_code text, name_terminology text, value_type text, value_num numeric, value_text text, value_code text, value_terminology text, rm_version text, language text, encoding text, origin_compact boolean, time_compact boolean, party_self boolean, uid_derived boolean, version_uid text, template_id text),
            node_search_text(value_type text, value_text text, data jsonb),
            node_magnitude(value_type text, value_num numeric, data jsonb),
            node_code(value_type text, value_code text, data jsonb),
            node_number(value_type text, value_num numeric, data jsonb),
            stamp_posture(a_key text, a_value text)
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION
            storage_generation(),
            openehr_numeric(t text),
            openehr_date_days(v text),
            openehr_time_seconds(v text),
            openehr_tz_offset_seconds(v text),
            openehr_date_time_seconds(v text),
            openehr_duration_seconds(v text),
            openehr_magnitude(dv jsonb),
            openehr_timestamp(v text),
            aql_not_an_amount(found text),
            node_name(name text, name_code text, name_terminology text),
            node_value(value_type text, value_num numeric, value_text text, value_code text, value_terminology text),
            node_archetype_node_id(archetype text, rm_version text),
            node_archetype_details(archetype text, rm_version text, template_id text),
            node_code_phrase(terminology text, code text),
            version_uid(vo_id uuid, creating_system_id text, trunk_version integer, branch_number integer, branch_version integer),
            node_date_time(value jsonb),
            node_fragment(data jsonb, rm_type text, archetype text, name text, name_code text, name_terminology text, value_type text, value_num numeric, value_text text, value_code text, value_terminology text, rm_version text, language text, encoding text, origin_compact boolean, time_compact boolean, party_self boolean, uid_derived boolean, version_uid text, template_id text),
            node_search_text(value_type text, value_text text, data jsonb),
            node_magnitude(value_type text, value_num numeric, data jsonb),
            node_code(value_type text, value_code text, data jsonb),
            node_number(value_type text, value_num numeric, data jsonb),
            stamp_posture(a_key text, a_value text)
            TO ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION
            storage_generation(),
            openehr_numeric(t text),
            openehr_date_days(v text),
            openehr_time_seconds(v text),
            openehr_tz_offset_seconds(v text),
            openehr_date_time_seconds(v text),
            openehr_duration_seconds(v text),
            openehr_magnitude(dv jsonb),
            openehr_timestamp(v text),
            aql_not_an_amount(found text),
            node_name(name text, name_code text, name_terminology text),
            node_value(value_type text, value_num numeric, value_text text, value_code text, value_terminology text),
            node_archetype_node_id(archetype text, rm_version text),
            node_archetype_details(archetype text, rm_version text, template_id text),
            node_code_phrase(terminology text, code text),
            version_uid(vo_id uuid, creating_system_id text, trunk_version integer, branch_number integer, branch_version integer),
            node_date_time(value jsonb),
            node_fragment(data jsonb, rm_type text, archetype text, name text, name_code text, name_terminology text, value_type text, value_num numeric, value_text text, value_code text, value_terminology text, rm_version text, language text, encoding text, origin_compact boolean, time_compact boolean, party_self boolean, uid_derived boolean, version_uid text, template_id text),
            node_search_text(value_type text, value_text text, data jsonb),
            node_magnitude(value_type text, value_num numeric, data jsonb),
            node_code(value_type text, value_code text, data jsonb),
            node_number(value_type text, value_num numeric, data jsonb),
            stamp_posture(a_key text, a_value text)
            TO ferroehr_party;
        GRANT EXECUTE ON FUNCTION
            storage_generation(),
            openehr_numeric(t text),
            openehr_date_days(v text),
            openehr_time_seconds(v text),
            openehr_tz_offset_seconds(v text),
            openehr_date_time_seconds(v text),
            openehr_duration_seconds(v text),
            openehr_magnitude(dv jsonb),
            openehr_timestamp(v text),
            aql_not_an_amount(found text),
            node_name(name text, name_code text, name_terminology text),
            node_value(value_type text, value_num numeric, value_text text, value_code text, value_terminology text),
            node_archetype_node_id(archetype text, rm_version text),
            node_archetype_details(archetype text, rm_version text, template_id text),
            node_code_phrase(terminology text, code text),
            version_uid(vo_id uuid, creating_system_id text, trunk_version integer, branch_number integer, branch_version integer),
            node_date_time(value jsonb),
            node_fragment(data jsonb, rm_type text, archetype text, name text, name_code text, name_terminology text, value_type text, value_num numeric, value_text text, value_code text, value_terminology text, rm_version text, language text, encoding text, origin_compact boolean, time_compact boolean, party_self boolean, uid_derived boolean, version_uid text, template_id text),
            node_search_text(value_type text, value_text text, data jsonb),
            node_magnitude(value_type text, value_num numeric, data jsonb),
            node_code(value_type text, value_code text, data jsonb),
            node_number(value_type text, value_num numeric, data jsonb),
            stamp_posture(a_key text, a_value text)
            TO ferroehr_party_reader;
        GRANT SELECT ON TABLE posture TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA ext GRANT EXECUTE ON FUNCTIONS TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA ext GRANT EXECUTE ON FUNCTIONS TO ferroehr_clinical_reader;
        ALTER DEFAULT PRIVILEGES IN SCHEMA ext GRANT EXECUTE ON FUNCTIONS TO ferroehr_party;
        ALTER DEFAULT PRIVILEGES IN SCHEMA ext GRANT EXECUTE ON FUNCTIONS TO ferroehr_party_reader;
    ELSE
        RAISE NOTICE 'skipping the ext grants (roles absent: see the role block NOTICE)';
    END IF;
END $$;
