// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for `ferroehr` — the platform library: the service
//! layer (EHR / `EHR_STATUS` / COMPOSITION / DIRECTORY / CONTRIBUTION,
//! definitions, demographics, query, validity, admin, terminology, message,
//! subject proxy), change control + version signing, the AQL engine, the
//! storage/node codec, validation, telemetry, the IHE ATNA system log, and the
//! quarantined extensions (the outbox feed, multimedia) and the FHIR audit and
//! terminology clients.
//!
//! Most tests take a fully-migrated `PostgreSQL` 18 database from the shared
//! harness (`testkit::db()`); the blob suite (`multimedia_s3`) additionally
//! starts a real testcontainer and is serialized by the nextest `containers`
//! group.
//!
//! One binary per crate, split into topic modules
//! (`.claude/rules/testing.md` §One integration-test binary per crate).

#![expect(
    clippy::disallowed_types,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    clippy::panic_in_result_fn,
    clippy::print_stdout,
    clippy::string_slice,
    clippy::too_many_lines,
    clippy::unwrap_used,
    let_underscore_drop,
    reason = "this binary is test code: a panicking assertion, an `expect` on a fixture \
              and direct fixture indexing are how a test fails (the Rust Book ch11), and \
              its fixtures and wire assertions are raw JSON (.claude/rules/testing.md \
              §Test-fixture construction); clippy.toml's `allow-*-in-tests` reaches only \
              `#[test]` functions, so the binary relaxes these once, here (owner \
              approval 2026-11-02, #4767)"
)]

mod access_classes;
mod access_delay_sinks;
mod access_eval_aql;
mod access_eval_examples;
mod access_eval_steps;
mod access_event_fail_closed;
mod access_log_aql;
mod access_log_checkpoint;
mod access_log_retention;
mod access_log_store;
mod access_origins;
mod access_policy;
mod adl14_knowledge_archetypes;
mod adl2_content_commit;
mod adl2_fixture;
mod adl2_quantity_property;
mod adl2_resolution;
mod adl2_vetdf;
mod adl2_vetdf_ferroterm;
mod adl2_write_race;
mod admin_fixture;
mod aql_depth_bound;
mod aql_planner;
mod aql_template_like;
mod audit_chain;
mod audit_ehds_mapping;
mod audit_feed;
mod audit_store;
mod audit_text;
mod caller_facts;
mod canonical_json_literals;
mod codec_corpus;
mod cohort_bench;
mod cohort_query;
mod cohort_subjects;
mod commit_lock_order;
mod committer_binding;
mod committer_check;
mod db_lease;
mod db_outage;
mod decommission;
mod definition_load;
mod directory_item_refs;
mod ehr_index_ihe;
mod ext_functions;
mod external_callers;
mod federation_agreements;
mod federation_node;
mod fixtures;
mod import_concurrency;
mod item_tag_fixture;
mod latest_head_fan_out;
mod legal_marks;
mod licence_stamp;
mod manufacturer;
mod multimedia_s3;
mod node_compact;
mod node_header;
mod node_readers;
mod one_schema_per_statement;
mod opt_resource_meta;
mod outbox_feed;
mod party_binding_adopt;
mod party_binding_check;
mod party_binding_config;
mod party_binding_source;
mod party_mapping;
mod party_mapping_templates;
mod party_reverse_paths;
mod persistence;
mod policy_elements;
mod policy_fixtures;
mod policy_grants;
mod policy_parties;
mod policy_parties_aql;
mod privacy_leak;
mod privacy_policy;
mod privacy_rules;
mod privacy_verbatim_replay;
mod pseudonymisation_boundary;
mod relocated_domain_preparation;
mod report;
mod schema_preparation_credential;
mod scim_users;
mod secondary_use_batch;
mod service_admin;
mod service_aql;
mod service_aql_attestation;
mod service_aql_demographic;
mod service_aql_temporal_binding;
mod service_aql_terminology;
mod service_aql_version_predicate;
mod service_branching;
mod service_contribution;
mod service_definition;
mod service_demographic;
mod service_demographic_refs;
mod service_dump_load;
mod service_ehr;
mod service_ehr_access;
mod service_ehr_package;
mod service_events;
mod service_extract;
mod service_import;
mod service_message_audit;
mod service_party_template;
mod service_relationship_index;
mod service_signing;
mod service_spec_profile;
mod service_system_id;
mod service_template;
mod service_validation;
mod shared_content;
mod signing_pgp;
mod sql_injection;
mod storage_parity;
mod storage_plans;
mod subject_pseudonym_guard;
mod system_callers;
mod system_grants;
mod system_log_tls_roundtrip;
mod system_migration;
mod telemetry;
mod telemetry_metrics;
mod telemetry_samplers;
mod terminology_fhir;
mod terminology_oauth;
mod terminology_wire;
mod typed_body;
mod usage_report;
mod validation_opt;
mod validation_opt14_writer;
