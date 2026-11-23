-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the grants.
--
-- The clinical writer role writes the records and their holds and reads them, the
-- reader role reads; neither may change a stored record. The release and
-- discard of holds, the folds and the retention sweeps that must cross those
-- rules are SECURITY DEFINER functions the writer role may execute. The
-- migration bookkeeping is read-only to the runtime roles. The default
-- privileges extend the table grants to every table a later file creates
-- here.
--
-- The role grants run only when the runtime roles exist (see the role block
-- of ext); without them the block is skipped with a NOTICE.
--
-- No openEHR spec governs database roles: our own design/extension.

REVOKE ALL ON FUNCTION
    delivery_park_undelivered(p_chain text, p_records bigint[], p_reason text),
    register_render_policy(p_policy jsonb),
    lock_search_tokens(),
    hold_search_tokens(),
    release_search_tokens(p_tokens integer[]),
    reap_audit_events_due(p_today date, p_unstated_cutoff timestamp with time zone),
    seal_reject_change(),
    audit_event_reaped_reject_change(),
    fold_audit_event_reaped(p_now timestamp with time zone, p_limit integer),
    access_log_shared_unchanged(),
    access_log_shared_collect(p_today date, p_limit integer),
    access_log_tombstone_end(p_content text),
    access_log_entry_indexed(),
    fold_access_log_tombstones(p_now timestamp with time zone, p_limit integer),
    chain_verification_lag(p_chain text),
    fold_access_log_imported(p_now timestamp with time zone, p_limit integer),
    access_log_intake_unknown(p_what text, p_digest text, p_removed boolean),
    access_log_intake_part_gone(p_digest text),
    access_log_token_generation_holds(p_generation bigint),
    access_log_intake_take(p_record_types text[], p_uids text[], p_times text[], p_contents text[], p_shared text[], p_digests text[], p_ehr_ids text[], p_party_ids text[], p_retain_until text[], p_action_terminologies text[], p_action_codes text[], p_result_terminologies text[], p_result_codes text[], p_actions text[], p_outcomes smallint[], p_principals text[], p_patient_ids text[], p_resource_ids text[], p_request_ids text[], p_client_ips text[], p_index_ehr_ids text[], p_search_tokens text[], p_policies bytea[], p_shared_digests text[], p_shared_contents text[], p_token_entries integer[], p_token_parameters text[], p_token_systems text[], p_token_codes text[], p_token_shares boolean[], p_reserved_by uuid, p_token_digests bytea[], p_token_generation bigint),
    audit_intake_release(p_ids uuid[]),
    audit_intake_discard(p_ids uuid[]),
    access_log_search(p_cursor bigint)
    FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT USAGE ON SCHEMA audit TO ferroehr_clinical;
        GRANT USAGE ON SCHEMA audit TO ferroehr_clinical_reader;
        GRANT SELECT ON TABLE audit_render_policy TO ferroehr_clinical;
        GRANT SELECT ON TABLE
            audit_render_policy,
            audit_search_token,
            audit_search_set,
            audit_event,
            audit_event_hold,
            audit_event_reaped,
            audit_event_seal,
            audit_event_seal_state,
            audit_event_seal_fold,
            audit_event_seal_verified,
            access_log_shared,
            access_log_entry,
            access_log_index,
            access_log_index_state,
            access_log_record,
            access_log_imported_record,
            access_log_imported_bundle,
            access_log_imported_folded,
            delivery_frontier,
            delivery_cursor,
            delivery_parked,
            system_search,
            audit_record,
            access_log_by_key,
            access_trail
            TO ferroehr_clinical_reader;
        GRANT SELECT, INSERT ON TABLE
            audit_search_token,
            audit_search_set,
            audit_event,
            access_log_shared,
            access_log_entry,
            access_log_record,
            access_log_imported_record,
            access_log_imported_bundle,
            access_log_imported_folded,
            system_search,
            audit_record,
            access_log_by_key,
            access_trail
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION
            register_render_policy(p_policy jsonb),
            hold_search_tokens(),
            reap_audit_events_due(p_today date, p_unstated_cutoff timestamp with time zone),
            fold_audit_event_reaped(p_now timestamp with time zone, p_limit integer),
            access_log_shared_collect(p_today date, p_limit integer),
            fold_access_log_tombstones(p_now timestamp with time zone, p_limit integer),
                    chain_verification_lag(p_chain text),
            fold_access_log_imported(p_now timestamp with time zone, p_limit integer),
            access_log_intake_unknown(p_what text, p_digest text, p_removed boolean),
            access_log_intake_part_gone(p_digest text),
            access_log_token_generation_holds(p_generation bigint),
            delivery_park_undelivered(p_chain text, p_records bigint[], p_reason text),
            access_log_intake_take(p_record_types text[], p_uids text[], p_times text[], p_contents text[], p_shared text[], p_digests text[], p_ehr_ids text[], p_party_ids text[], p_retain_until text[], p_action_terminologies text[], p_action_codes text[], p_result_terminologies text[], p_result_codes text[], p_actions text[], p_outcomes smallint[], p_principals text[], p_patient_ids text[], p_resource_ids text[], p_request_ids text[], p_client_ips text[], p_index_ehr_ids text[], p_search_tokens text[], p_policies bytea[], p_shared_digests text[], p_shared_contents text[], p_token_entries integer[], p_token_parameters text[], p_token_systems text[], p_token_codes text[], p_token_shares boolean[], p_reserved_by uuid, p_token_digests bytea[], p_token_generation bigint),
            audit_intake_release(p_ids uuid[]),
            audit_intake_discard(p_ids uuid[])
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION access_log_search(p_cursor bigint)
            TO ferroehr_clinical, ferroehr_clinical_reader;
        GRANT SELECT, UPDATE ON TABLE delivery_frontier TO ferroehr_clinical;
        GRANT SELECT, INSERT, UPDATE, DELETE ON TABLE delivery_cursor, delivery_parked
            TO ferroehr_clinical;
        GRANT UPDATE(content) ON TABLE
            access_log_entry,
            access_log_imported_record,
            access_log_record
            TO ferroehr_clinical;
        GRANT UPDATE(shared) ON TABLE access_log_entry TO ferroehr_clinical;
        GRANT UPDATE(published_at) ON TABLE access_log_seal TO ferroehr_clinical;
        GRANT SELECT, INSERT ON TABLE access_log_hold, audit_event_hold TO ferroehr_clinical;
        GRANT UPDATE(published_at) ON TABLE audit_event_seal TO ferroehr_clinical;
        GRANT SELECT, INSERT ON TABLE audit_event_seal TO ferroehr_clinical;
        GRANT SELECT, UPDATE ON TABLE audit_event_seal_state, audit_event_seal_verified TO ferroehr_clinical;
        GRANT INSERT ON TABLE audit_event_seal_verified TO ferroehr_clinical;
        GRANT SELECT ON TABLE audit_event_seal_fold, audit_event_reaped TO ferroehr_clinical;
        GRANT SELECT ON TABLE access_log_hold TO ferroehr_clinical_reader;
        GRANT SELECT, INSERT ON TABLE access_log_seal TO ferroehr_clinical;
        GRANT SELECT, UPDATE ON TABLE access_log_seal_state, access_log_seal_verified TO ferroehr_clinical;
        GRANT INSERT ON TABLE access_log_seal_verified TO ferroehr_clinical;
        GRANT SELECT ON TABLE access_log_seal_fold TO ferroehr_clinical;
        -- The frontier round copies the search index (access_log::index).
        GRANT SELECT, INSERT ON TABLE access_log_index TO ferroehr_clinical;
        GRANT SELECT, UPDATE ON TABLE access_log_index_state TO ferroehr_clinical;
        GRANT SELECT ON TABLE audit_search_token_generation TO ferroehr_clinical, ferroehr_clinical_reader;
        -- The frontier reads the id sequences' last values (access_log::frontier).
        GRANT SELECT ON SEQUENCE access_log_entry_id_seq, audit_event_seq_seq TO ferroehr_clinical;
        GRANT SELECT ON TABLE
            access_log_seal,
            access_log_seal_state,
            access_log_seal_fold,
            access_log_seal_verified
            TO ferroehr_clinical_reader;
        GRANT UPDATE(digest) ON TABLE access_log_shared TO ferroehr_clinical;
        GRANT UPDATE(retain_until) ON TABLE access_log_shared TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA audit GRANT SELECT,INSERT ON TABLES TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA audit GRANT SELECT ON TABLES TO ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the audit grants (roles absent: see the role block NOTICE of ext)';
    END IF;
END $$;
