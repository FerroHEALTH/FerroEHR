-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- party: the grants.
--
-- The writer role (ferroehr_party) reads and writes every relation, the
-- reader role (ferroehr_party_reader) reads them; the migration bookkeeping
-- is read-only to both. The default privileges extend the same grants to
-- every table a later file creates here. The clinical roles are granted
-- nothing in this schema; they may run the one function of party_door.
--
-- The role grants run only when the runtime roles exist (see the role block
-- of ext); without them the block is skipped with a NOTICE.
--
-- No openEHR spec governs database roles: our own design/extension.

REVOKE ALL ON FUNCTION
    node_shared_member(a_key text, a_num integer, a_tier text, a_vo_id uuid, a_sys_version integer),
    node_shared(a_shared jsonb, a_tier text, a_vo_id uuid, a_sys_version integer),
    binding_date(p_value text, p_upper boolean),
    binding_validity(p_interval jsonb),
    binding_fold(p_value text),
    binding_change_set(items jsonb),
    binding_changed(),
    binding_revision_committed(),
    person_version_changed(),
    facts_behind(held pg_snapshot)
    FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_party') THEN
        GRANT USAGE ON SCHEMA party TO ferroehr_party;
        GRANT USAGE ON SCHEMA party TO ferroehr_party_reader;
        GRANT SELECT ON TABLE _sqlx_migrations TO ferroehr_party;
        GRANT SELECT ON TABLE
            _sqlx_migrations,
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
            item_tag,
            party_relationship_target,
            event_outbox,
            event_outbox_reader,
            blob_ref,
            audit_reservation,
            binding_identifier,
            binding_membership,
            binding_representation,
            binding_lease,
            binding_state,
            binding_group_revision,
            binding_capability,
            binding_provider,
            binding_client,
            binding_grant_revision,
            binding_caller_fact,
            binding_caller_fact_party,
            binding_record_subject,
            binding_patient_identifier,
            binding_key_space,
            binding_care_validity,
            scim_deactivation,
            scim_contact_erasure,
            scim_group,
            pmir_subscription,
            pmir_message,
            pmir_identity,
            pmir_merge,
            pmir_suppression,
            access_revision_of,
            request_revisions
            TO ferroehr_party_reader;
        GRANT SELECT, INSERT, DELETE, UPDATE ON TABLE
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
            item_tag,
            party_relationship_target,
            event_outbox,
            event_outbox_reader,
            blob_ref,
            audit_reservation,
            binding_identifier,
            binding_membership,
            binding_representation,
            binding_lease,
            binding_state,
            binding_group_revision,
            binding_capability,
            binding_provider,
            binding_client,
            binding_grant_revision,
            binding_caller_fact,
            binding_caller_fact_party,
            binding_record_subject,
            binding_patient_identifier,
            binding_key_space,
            binding_care_validity,
            scim_deactivation,
            scim_contact_erasure,
            scim_group,
            pmir_subscription,
            pmir_message,
            pmir_identity,
            pmir_merge,
            pmir_suppression,
            access_revision_of,
            request_revisions
            TO ferroehr_party;
        GRANT SELECT, INSERT, DELETE, MAINTAIN, UPDATE ON TABLE
            version,
            vo_attestation,
            node
            TO ferroehr_party;
        GRANT EXECUTE ON FUNCTION
            binding_date(p_value text, p_upper boolean),
            binding_validity(p_interval jsonb),
            binding_fold(p_value text),
            binding_change_set(items jsonb)
            TO ferroehr_party;
        GRANT EXECUTE ON FUNCTION
            node_shared_member(a_key text, a_num integer, a_tier text, a_vo_id uuid, a_sys_version integer),
            node_shared(a_shared jsonb, a_tier text, a_vo_id uuid, a_sys_version integer),
            facts_behind(held pg_snapshot)
            TO ferroehr_party, ferroehr_party_reader;
        ALTER DEFAULT PRIVILEGES IN SCHEMA party GRANT SELECT,INSERT,DELETE,UPDATE ON TABLES TO ferroehr_party;
        ALTER DEFAULT PRIVILEGES IN SCHEMA party GRANT SELECT ON TABLES TO ferroehr_party_reader;
        -- The party roles read the same revisions through the door, so a statement
        -- of a ride selects them on either pool (#6320).
        GRANT USAGE ON SCHEMA party_door TO ferroehr_party, ferroehr_party_reader;
        GRANT EXECUTE ON FUNCTION party_door.party_revisions(held pg_snapshot)
            TO ferroehr_party, ferroehr_party_reader;
        -- The one door the clinical roles may pass (#6320): USAGE on the door's
        -- schema and EXECUTE on its one function, nothing of the party schema.
        IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
            GRANT USAGE ON SCHEMA party_door TO ferroehr_clinical, ferroehr_clinical_reader;
            GRANT EXECUTE ON FUNCTION party_door.party_revisions(held pg_snapshot)
                TO ferroehr_clinical, ferroehr_clinical_reader;
        END IF;
    ELSE
        RAISE NOTICE 'skipping the party grants (roles absent: see the role block NOTICE of ext)';
    END IF;
END $$;
