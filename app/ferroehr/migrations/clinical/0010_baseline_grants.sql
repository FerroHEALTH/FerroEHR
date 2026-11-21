-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the grants.
--
-- The writer role (ferroehr_clinical) reads and writes every relation, the
-- reader role (ferroehr_clinical_reader) reads them; the migration
-- bookkeeping is read-only to both, so a runtime credential cannot make an
-- unmigrated or altered database pass `[database] migrations = "verify"`. The
-- default privileges extend the same grants to every table a later file
-- creates here. The party roles are granted nothing in this schema.
--
-- The role grants run only when the runtime roles exist (see the role block
-- of ext); without them the block is skipped with a NOTICE.
--
-- No openEHR spec governs database roles: our own design/extension.

REVOKE ALL ON FUNCTION
    node_shared_member(a_key text, a_num integer, a_tier text, a_vo_id uuid, a_sys_version integer),
    node_shared(a_shared jsonb, a_tier text, a_vo_id uuid, a_sys_version integer),
    subject_pseudonym_guard(),
    access_class_notify(),
    access_settings_notify(),
    access_class_unknown(ehr uuid),
    access_stale_copy(),
    definition_changed()
    FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT USAGE ON SCHEMA clinical TO ferroehr_clinical;
        GRANT USAGE ON SCHEMA clinical TO ferroehr_clinical_reader;
        GRANT SELECT ON TABLE _sqlx_migrations TO ferroehr_clinical;
        GRANT SELECT ON TABLE
            _sqlx_migrations,
            ehr,
            template_store,
            template_ref,
            archetype_store,
            adl2_artefact,
            definition_revision,
            stored_query,
            commit_audit,
            contribution,
            version,
            version_hot,
            version_cold,
            vo_head,
            vo_attestation,
            vo_attestation_hot,
            vo_attestation_cold,
            node,
            node_hot,
            node_cold,
            ehr_folder,
            item_tag,
            restriction,
            retention_anchor,
            retention_policy,
            category_map,
            retention_due,
            event_outbox,
            event_outbox_reader,
            blob_ref,
            audit_reservation,
            instance,
            usage_report_instance,
            usage_report_window,
            usage_report_counter,
            dpop_proof,
            policy_register,
            access_policy_version,
            access_delay,
            access_class,
            access_settings,
            access_projection_state,
            access_care,
            access_care_state,
            access_revision_of
            TO ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION
            subject_pseudonym_guard(),
            access_class_unknown(ehr uuid),
            access_stale_copy()
            TO ferroehr_clinical;
        GRANT SELECT, INSERT, DELETE, MAINTAIN, UPDATE ON TABLE
            ehr,
            version,
            vo_attestation,
            node
            TO ferroehr_clinical;
        GRANT SELECT, INSERT, DELETE, UPDATE ON TABLE
            template_store,
            template_ref,
            archetype_store,
            adl2_artefact,
            definition_revision,
            stored_query,
            commit_audit,
            contribution,
            version_hot,
            version_cold,
            vo_head,
            vo_attestation_hot,
            vo_attestation_cold,
            node_hot,
            node_cold,
            ehr_folder,
            item_tag,
            restriction,
            retention_anchor,
            retention_policy,
            category_map,
            retention_due,
            event_outbox,
            event_outbox_reader,
            blob_ref,
            audit_reservation,
            instance,
            usage_report_instance,
            usage_report_window,
            usage_report_counter,
            dpop_proof,
            policy_register,
            access_policy_version,
            access_delay,
            access_class,
            access_settings,
            access_projection_state,
            access_care,
            access_care_state,
            access_revision_of
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION
            access_class_unknown(ehr uuid),
            access_stale_copy()
            TO ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION
            node_shared_member(a_key text, a_num integer, a_tier text, a_vo_id uuid, a_sys_version integer),
            node_shared(a_shared jsonb, a_tier text, a_vo_id uuid, a_sys_version integer)
            TO ferroehr_clinical, ferroehr_clinical_reader;
        ALTER DEFAULT PRIVILEGES IN SCHEMA clinical GRANT SELECT,INSERT,DELETE,UPDATE ON TABLES TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA clinical GRANT SELECT ON TABLES TO ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the clinical grants (roles absent: see the role block NOTICE of ext)';
    END IF;
END $$;
