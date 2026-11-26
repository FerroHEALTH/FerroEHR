-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- clinical: the grants.
--
-- The writer role (ferroehr_clinical) reads and writes every relation, the
-- reader role (ferroehr_clinical_reader) reads them; the migration
-- bookkeeping is read-only to both, so a runtime credential cannot make an
-- unmigrated or altered database pass `[database] migrations = "verify"`. The
-- default privileges extend the same grants to every table a later file
-- creates here. The party roles are granted nothing in this schema; they may
-- run the one function of clinical_door.
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
    directory_slot_held(locked boolean, ehr uuid, highest integer, slot uuid, head integer),
    definition_changed(),
    policy_changed()
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
            commit_audit_facts,
            commit_audit_shared,
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
            policy_revision,
            access_delay,
            access_class,
            access_settings,
            access_projection_state,
            access_care,
            access_care_state,
            access_revision_of,
            request_revisions
            TO ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION
            subject_pseudonym_guard(),
            access_class_unknown(ehr uuid),
            access_stale_copy(),
            directory_slot_held(locked boolean, ehr uuid, highest integer, slot uuid, head integer)
            TO ferroehr_clinical;
        GRANT SELECT, INSERT, DELETE, MAINTAIN, UPDATE ON TABLE
            ehr,
            version,
            vo_attestation,
            node,
            access_care
            TO ferroehr_clinical;
        GRANT SELECT, INSERT, DELETE, UPDATE ON TABLE
            template_store,
            template_ref,
            archetype_store,
            adl2_artefact,
            definition_revision,
            stored_query,
            commit_audit,
            commit_audit_facts,
            commit_audit_shared,
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
            policy_revision,
            access_delay,
            access_class,
            access_settings,
            access_projection_state,
            access_care_state,
            access_revision_of,
            request_revisions
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION
            access_class_unknown(ehr uuid),
            access_stale_copy()
            TO ferroehr_clinical_reader;
        GRANT EXECUTE ON FUNCTION
            node_shared_member(a_key text, a_num integer, a_tier text, a_vo_id uuid, a_sys_version integer),
            node_shared(a_shared jsonb, a_tier text, a_vo_id uuid, a_sys_version integer)
            TO ferroehr_clinical, ferroehr_clinical_reader;
        -- The one door the party roles may pass (#6320): USAGE on the door's
        -- schema and EXECUTE on its one function, nothing of the clinical schema.
        IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
            GRANT USAGE ON SCHEMA clinical_door TO ferroehr_party, ferroehr_party_reader;
            GRANT EXECUTE ON FUNCTION clinical_door.policy_revisions()
                TO ferroehr_party, ferroehr_party_reader;
        END IF;
        ALTER DEFAULT PRIVILEGES IN SCHEMA clinical GRANT SELECT,INSERT,DELETE,UPDATE ON TABLES TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA clinical GRANT SELECT ON TABLES TO ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the clinical grants (roles absent: see the role block NOTICE of ext)';
    END IF;
END $$;
