// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for `ferroehr-rest` — the ITS-REST protocol adapter: the
//! assembled axum router end to end (routing, content negotiation, committal and
//! versioning headers, the status/error table), every API group's wire behaviour
//! (EHR / COMPOSITION / DIRECTORY / CONTRIBUTION, definitions, demographics,
//! query, admin, message), the Simplified Formats surface, authentication
//! (Basic + Bearer) with the RBAC/ABAC PEP and SMART scope enforcement, the ATNA
//! audit middleware, and the flagged extensions (FHIR, terminology, management,
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
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]

mod canonical_json_literals;
mod common;

mod abac_e2e;
mod admin_extension_http;
mod admin_http;
mod admin_integrity_http;
mod audit_e2e;
mod audit_iti81;
mod authz_cedar_engine;
mod authz_remote_pdp;
mod authz_route_matrix;
mod base_path_http;
mod cohort_http;
mod composition_validation_http;
mod connection_bounds;
mod credential_separation;
mod definition_adl2_http;
mod definition_archetype_http;
mod demographic_http;
mod demographic_tags_http;
mod directory_http;
mod ehr_access_gate;
mod error_chain;
mod event_subscription_http;
mod example_http;
mod extensions_openapi;
mod fhir_http;
mod fhir_inbound;
mod fixture_smoke;
mod flat_http;
mod headers;
mod http;
mod item_tag_http;
mod legal_marks_http;
mod management;
mod message_extension_http;
mod overload_http;
mod protocol_tail;
mod query_ehr_id_header;
mod rbac_e2e;
mod served_openapi;
mod server_tls;
mod service_query;
mod smart_http;
mod spec_profile_http;
mod stored_query_definition_http;
mod template_adl14_http;
mod terminology_http;
mod trace_shape;
