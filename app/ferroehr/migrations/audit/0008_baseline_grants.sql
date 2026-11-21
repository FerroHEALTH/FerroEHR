-- SPDX-FileCopyrightText: Cadasto B.V.
-- SPDX-License-Identifier: BUSL-1.1

-- audit: the grants.
--
-- The clinical writer role writes the records and the intakes and reads what it chains, the
-- reader role reads; neither may change a chained record. The chain moves,
-- the verification walks and the retention sweeps that must cross those
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
    audit_event_leave_outbox(),
    register_render_policy(p_policy jsonb),
    lock_search_tokens(),
    hold_search_tokens(),
    release_search_tokens(p_tokens integer[]),
    reap_audit_events_due(p_today date, p_unstated_cutoff timestamp with time zone),
    audit_event_chain_move(p_limit integer, p_fhir_feed boolean, p_syslog boolean),
    audit_chain_link_at(p_sequence bigint),
    audit_chain_verification_link(p_sequence bigint),
    audit_chain_verification_start(p_full boolean),
    audit_chain_verification_step_end(p_after bigint, p_records integer, p_head bigint),
    verify_audit_chain_step(p_after bigint, p_after_digest text, p_upto bigint, p_advance boolean, p_last boolean, p_full boolean),
    verify_audit_chain_window(p_after_seq bigint, p_after_hash bytea, p_upto bigint, p_check_head boolean),
    verify_audit_chain_after(p_after_seq bigint, p_after_hash bytea),
    audit_event_link_reject_deletion(),
    access_log_entry_unindex(),
    access_log_link_append(),
    access_log_shared_unchanged(),
    access_log_checkpoint_names_a_link(),
    access_log_checkpoint_reject_removal(),
    access_log_link_at(p_sequence bigint),
    access_log_sealed_link(p_sequence bigint, p_proof text),
    access_log_chain_move(p_batches bigint[], p_entries bigint[], p_seqs bigint[], p_previous text[], p_digests text[], p_fhir_feed boolean, p_syslog boolean),
    access_log_chain_take(),
    access_log_shared_collect(p_today date, p_limit integer),
    access_log_tombstone_end(p_content text),
    fold_access_log_tombstones(p_now timestamp with time zone, p_limit integer),
    access_log_verification_link(p_sequence bigint),
    access_log_verification_start(p_full boolean),
    access_log_verification_step_end(p_after bigint, p_records integer, p_head bigint),
    verify_access_log_chain_step(p_after bigint, p_after_digest text, p_upto bigint, p_modified bigint[], p_advance boolean, p_last boolean, p_full boolean),
    verify_access_log_chain_window(p_after_seq bigint, p_after_digest text, p_upto bigint, p_check_head boolean),
    verify_access_log_chain_after(p_after_seq bigint, p_after_digest text),
    chain_verification_lag(p_chain text),
    fold_access_log_imported(p_now timestamp with time zone, p_limit integer),
    audit_intake_reject_change(),
    access_log_intake_unknown(p_what text, p_digest text, p_removed boolean),
    access_log_intake_take(p_record_types text[], p_uids text[], p_times text[], p_contents text[], p_shared text[], p_digests text[], p_ehr_ids text[], p_party_ids text[], p_retain_until text[], p_action_terminologies text[], p_action_codes text[], p_result_terminologies text[], p_result_codes text[], p_actions text[], p_outcomes smallint[], p_principals text[], p_patient_ids text[], p_resource_ids text[], p_request_ids text[], p_client_ips text[], p_index_ehr_ids text[], p_search_tokens text[], p_policies bytea[], p_shared_digests text[], p_shared_contents text[], p_token_entries integer[], p_token_parameters text[], p_token_systems text[], p_token_codes text[], p_token_shares boolean[], p_reserved_by uuid, p_token_digests bytea[]),
    audit_intake_release(p_ids uuid[]),
    audit_intake_discard(p_ids uuid[])
    FROM PUBLIC;

DO $$
BEGIN
    IF EXISTS (SELECT FROM pg_roles WHERE rolname = 'ferroehr_clinical') THEN
        GRANT USAGE ON SCHEMA audit TO ferroehr_clinical;
        GRANT USAGE ON SCHEMA audit TO ferroehr_clinical_reader;
        GRANT SELECT ON TABLE
            audit_render_policy,
            audit_event_link,
            audit_chain_state,
            audit_chain_gap,
            audit_chain_checkpoint,
            access_log_chain_state,
            access_log_chain_verified,
            access_log_chain_gap
            TO ferroehr_clinical;
        GRANT SELECT ON TABLE
            audit_render_policy,
            audit_search_token,
            audit_search_set,
            audit_event,
            audit_event_link,
            audit_chain_state,
            audit_chain_gap,
            audit_chain_checkpoint,
            audit_chain_seal,
            access_log_shared,
            access_log_entry,
            access_log_link,
            access_log_checkpoint,
            access_log_chain_state,
            access_log_chain_verified,
            access_log_chain_gap,
            access_log_record,
            access_log_imported_link,
            access_log_imported_record,
            access_log_imported_checkpoint,
            access_log_imported_gap,
            audit_event_intake,
            access_log_intake,
            syslog_outbox,
            access_log_search,
            audit_record,
            access_trail
            TO ferroehr_clinical_reader;
        GRANT SELECT, INSERT ON TABLE
            audit_search_token,
            audit_search_set,
            audit_chain_seal,
            audit_event,
            access_log_shared,
            access_log_entry,
            access_log_link,
            access_log_checkpoint,
            access_log_record,
            access_log_imported_link,
            access_log_imported_record,
            access_log_imported_checkpoint,
            access_log_imported_gap,
            audit_event_intake,
            access_log_intake,
            access_log_search,
            audit_record,
            access_trail
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION
            register_render_policy(p_policy jsonb),
            hold_search_tokens(),
            reap_audit_events_due(p_today date, p_unstated_cutoff timestamp with time zone),
            audit_event_chain_move(p_limit integer, p_fhir_feed boolean, p_syslog boolean),
            audit_chain_link_at(p_sequence bigint),
            audit_chain_verification_link(p_sequence bigint),
            audit_chain_verification_start(p_full boolean),
            audit_chain_verification_step_end(p_after bigint, p_records integer, p_head bigint),
            verify_audit_chain_step(p_after bigint, p_after_digest text, p_upto bigint, p_advance boolean, p_last boolean, p_full boolean),
            verify_audit_chain_window(p_after_seq bigint, p_after_hash bytea, p_upto bigint, p_check_head boolean),
            verify_audit_chain_after(p_after_seq bigint, p_after_hash bytea),
            verify_audit_chain(),
            access_log_link_at(p_sequence bigint),
            access_log_sealed_link(p_sequence bigint, p_proof text),
            access_log_chain_move(p_batches bigint[], p_entries bigint[], p_seqs bigint[], p_previous text[], p_digests text[], p_fhir_feed boolean, p_syslog boolean),
            access_log_chain_take(),
            access_log_shared_collect(p_today date, p_limit integer),
            fold_access_log_tombstones(p_now timestamp with time zone, p_limit integer),
            access_log_verification_link(p_sequence bigint),
            access_log_verification_start(p_full boolean),
            access_log_verification_step_end(p_after bigint, p_records integer, p_head bigint),
            verify_access_log_chain_step(p_after bigint, p_after_digest text, p_upto bigint, p_modified bigint[], p_advance boolean, p_last boolean, p_full boolean),
            verify_access_log_chain_window(p_after_seq bigint, p_after_digest text, p_upto bigint, p_check_head boolean),
            verify_access_log_chain_after(p_after_seq bigint, p_after_digest text),
            verify_access_log_chain(),
            chain_verification_lag(p_chain text),
            fold_access_log_imported(p_now timestamp with time zone, p_limit integer),
            access_log_intake_unknown(p_what text, p_digest text, p_removed boolean),
            access_log_intake_take(p_record_types text[], p_uids text[], p_times text[], p_contents text[], p_shared text[], p_digests text[], p_ehr_ids text[], p_party_ids text[], p_retain_until text[], p_action_terminologies text[], p_action_codes text[], p_result_terminologies text[], p_result_codes text[], p_actions text[], p_outcomes smallint[], p_principals text[], p_patient_ids text[], p_resource_ids text[], p_request_ids text[], p_client_ips text[], p_index_ehr_ids text[], p_search_tokens text[], p_policies bytea[], p_shared_digests text[], p_shared_contents text[], p_token_entries integer[], p_token_parameters text[], p_token_systems text[], p_token_codes text[], p_token_shares boolean[], p_reserved_by uuid, p_token_digests bytea[]),
            audit_intake_release(p_ids uuid[]),
            audit_intake_discard(p_ids uuid[])
            TO ferroehr_clinical;
        GRANT EXECUTE ON FUNCTION
            audit_chain_link_at(p_sequence bigint),
            verify_audit_chain_window(p_after_seq bigint, p_after_hash bytea, p_upto bigint, p_check_head boolean),
            verify_audit_chain_after(p_after_seq bigint, p_after_hash bytea),
            verify_audit_chain(),
            access_log_link_at(p_sequence bigint),
            access_log_sealed_link(p_sequence bigint, p_proof text),
            verify_access_log_chain_window(p_after_seq bigint, p_after_digest text, p_upto bigint, p_check_head boolean),
            verify_access_log_chain_after(p_after_seq bigint, p_after_digest text),
            verify_access_log_chain()
            TO ferroehr_clinical_reader;
        GRANT SELECT, DELETE ON TABLE fhir_feed_outbox TO ferroehr_clinical;
        GRANT SELECT, INSERT, DELETE ON TABLE syslog_outbox TO ferroehr_clinical;
        GRANT UPDATE(content) ON TABLE
            access_log_entry,
            access_log_imported_record,
            access_log_record
            TO ferroehr_clinical;
        GRANT UPDATE(shared) ON TABLE access_log_entry TO ferroehr_clinical;
        GRANT UPDATE(published_at) ON TABLE access_log_checkpoint TO ferroehr_clinical;
        GRANT UPDATE(digest) ON TABLE access_log_shared TO ferroehr_clinical;
        GRANT UPDATE(retain_until) ON TABLE access_log_shared TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA audit GRANT SELECT,INSERT ON TABLES TO ferroehr_clinical;
        ALTER DEFAULT PRIVILEGES IN SCHEMA audit GRANT SELECT ON TABLES TO ferroehr_clinical_reader;
    ELSE
        RAISE NOTICE 'skipping the audit grants (roles absent: see the role block NOTICE of ext)';
    END IF;
END $$;
