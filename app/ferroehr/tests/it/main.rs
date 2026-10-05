// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for `ferroehr` — the platform library: the SM service
//! layer (EHR / `EHR_STATUS` / COMPOSITION / DIRECTORY / CONTRIBUTION,
//! definitions, demographics, query, validity, admin, terminology, message,
//! subject proxy), change control + version signing, the AQL engine, the
//! storage/node codec, validation, telemetry, the IHE ATNA system log, and the
//! quarantined extensions (eventing, FHIR, multimedia, tenancy).
//!
//! Most tests take a fully-migrated `PostgreSQL` 18 database from the shared
//! harness (`testkit::db()`); the broker/blob suites (`events_amqp`,
//! `fhir_outbound_amqp`, `multimedia_s3`) additionally start real
//! testcontainers and are serialized by the nextest `containers` group.
//!
//! One binary per crate, split into topic modules
//! (`.claude/rules/testing.md` §One integration-test binary per crate).

#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

mod access_event_fail_closed;
mod access_origins;
mod adl14_knowledge_archetypes;
mod adl2_fixture;
mod adl2_vetdf;
mod adl2_vetdf_ferroterm;
mod admin_fixture;
mod aql_planner;
mod audit_chain;
mod audit_ehds_mapping;
mod audit_feed;
mod audit_store;
mod canonical_json_literals;
mod codec_corpus;
mod cohort_bench;
mod cohort_query;
mod directory_item_refs;
mod events_amqp;
mod ext_functions;
mod fhir_ingest_translate;
mod fhir_outbound_amqp;
mod fhir_priority_categories;
mod fixtures;
mod item_tag_fixture;
mod legal_marks;
mod licence_stamp;
mod multimedia_s3;
mod one_schema_per_statement;
mod opt_resource_meta;
mod outbox_prune;
mod persistence;
mod privacy_leak;
mod privacy_policy;
mod privacy_rules;
mod privacy_verbatim_replay;
mod pseudonymisation_boundary;
mod schema_preparation_credential;
mod service_admin;
mod service_aql;
mod service_aql_terminology;
mod service_branching;
mod service_contribution;
mod service_definition;
mod service_demographic;
mod service_dump_load;
mod service_ehr;
mod service_ehr_access;
mod service_ehr_index_conflicts;
mod service_ehr_package;
mod service_events;
mod service_extract;
mod service_import;
mod service_message_audit;
mod service_signing;
mod service_sm3;
mod service_spec_profile;
mod service_system_id;
mod service_tdd;
mod service_template;
mod service_validation;
mod signing_pgp;
mod sql_injection;
mod storage_parity;
mod storage_plans;
mod subject_pseudonym_guard;
mod system_log_tls_roundtrip;
mod telemetry;
mod telemetry_metrics;
mod telemetry_samplers;
mod terminology_fhir;
mod terminology_mtls;
mod terminology_multi_provider;
mod typed_body;
mod usage_report;
mod validation_opt;
