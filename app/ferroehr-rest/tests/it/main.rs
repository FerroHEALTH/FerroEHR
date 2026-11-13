// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for `ferroehr-rest` — the ITS-REST protocol adapter: the
//! assembled axum router end to end (routing, content negotiation, committal and
//! versioning headers, the status/error table), every API group's wire behaviour
//! (EHR / COMPOSITION / DIRECTORY / CONTRIBUTION, definitions, demographics,
//! query, admin, message), the Simplified Formats surface, authentication
//! (Basic + Bearer) with the RBAC/ABAC PEP and SMART scope enforcement, the ATNA
//! audit middleware, and the flagged extensions (the ITI-81 audit retrieval,
//! terminology, management,
//! tenancy, event subscriptions, TLS).
//!
//! Most tests drive the real `FerroEhrService` over a fully-migrated `PostgreSQL`
//! 18 database taken from the shared harness (`testkit::db()`), assembled by the
//! shared [`common`] fixture module.
//!
//! One binary per crate, split into topic modules
//! (`.claude/rules/testing.md` §One integration-test binary per crate).

#![expect(
    clippy::disallowed_types,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::missing_assert_message,
    clippy::panic,
    clippy::string_slice,
    clippy::unwrap_used,
    reason = "this binary is test code: a panicking assertion, an `expect` on a fixture \
              and direct fixture indexing are how a test fails (the Rust Book ch11), and \
              its fixtures and wire assertions are raw JSON (.claude/rules/testing.md \
              §Test-fixture construction); clippy.toml's `allow-*-in-tests` reaches only \
              `#[test]` functions, so the binary relaxes these once, here (owner \
              approval 2026-11-02, #4767)"
)]

mod canonical_json_literals;
mod common;

mod abac_e2e;
mod access_delay_http;
mod access_delay_trail;
mod access_opt_out_http;
mod access_restriction_http;
mod admin_extension_http;
mod admin_http;
mod admin_integrity_http;
mod admin_party_delete_http;
mod admin_party_identifiers_http;
mod audit_categories;
mod audit_data_permit;
mod audit_e2e;
mod audit_ehr_subject;
mod audit_emergency_access;
mod audit_iti81;
mod audit_route_coverage;
mod authz_cedar_engine;
mod authz_remote_pdp;
mod authz_route_matrix;
mod base_path_http;
mod basic_binding_http;
mod binding_callers_http;
mod binding_commits_http;
mod binding_issuers_http;
mod binding_purposes_http;
mod binding_refusals_http;
mod caller_kinds_http;
mod cohort_http;
mod composition_validation_http;
mod connection_bounds;
mod connection_per_request;
mod contact_point_http;
mod credential_separation;
mod definition_adl2_delete_http;
mod definition_adl2_http;
mod definition_archetype_http;
mod definition_conversion_http;
mod demographic_ehr_join_http;
mod demographic_http;
mod demographic_refusals_http;
mod demographic_tags_http;
mod demographic_template_http;
mod directory_http;
mod dpop_replay;
mod ehds_person_inserted_http;
mod ehr_access_gate;
mod ehr_access_repair_http;
mod error_chain;
mod example_http;
mod extensions_openapi;
mod federation_http;
mod fixture_smoke;
mod flat_http;
mod headers;
mod http;
mod item_tag_http;
mod legal_marks_http;
mod local_ehr_index_posture;
mod management;
mod message_extension_http;
mod outbox_http;
mod overload_http;
mod party_binding_http;
mod party_reverse_relationships_http;
mod patient_data_assurance;
mod policy_grants_http;
mod policy_registers_http;
mod protocol_tail;
mod query_ehr_id_header;
mod query_refusals_http;
mod rbac_e2e;
mod representation_http;
mod sandbox_seed_http;
mod scim_endpoint;
mod scim_protocol;
mod scim_users_http;
mod served_openapi;
mod served_openapi_snapshot;
mod server_tls;
mod service_query;
mod smart_http;
mod smart_metadata_file;
mod spec_profile_http;
mod stored_query_definition_http;
mod subject_query_privacy;
mod system_grants;
mod template_adl14_http;
mod terminology_http;
mod trace_shape;
mod version_id_argument_http;
mod xml_document_roots_http;
