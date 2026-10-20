// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The fixed token sets the CDR serves and the viewer names.
//!
//! Two closed vocabularies cross the REST boundary as plain strings: the
//! deployment gap codes `GET /management/status` and the status document
//! carry, and the API-family slugs the OpenAPI documents are served under
//! (`api-docs/ferroehr-{slug}.openapi.json`). The viewer links no application
//! crate, so it compiles this file through a `#[path]` module, as it does
//! `manufacturer.rs`; the server holds its own definitions to these lists in
//! tests, so a token added on one side without the other fails a build. The
//! file depends on nothing, so both crates and both viewer targets compile it
//! unchanged. No openEHR spec governs either set: our own design/extension.

/// Every deployment gap code the server reports, in declaration order.
pub const DEPLOYMENT_GAPS: [&str; 13] = [
    "shared_credential",
    "shared_cluster",
    "open_subject_namespace",
    "audit_off",
    "audit_fails_open",
    "open_ehr_access_default",
    "migrate_on_runtime_credential",
    "auth_off",
    "rbac_off",
    "plaintext_listener",
    "plaintext_broker",
    "plaintext_audit_feed",
    "plaintext_object_store",
];

/// Every API family served as its own OpenAPI document, as
/// `(url slug, selector name)`, in the server's selector order.
pub const OPENAPI_FAMILIES: [(&str, &str); 12] = [
    ("ehr", "openEHR: EHR"),
    ("query", "openEHR: Query"),
    ("definition", "openEHR: Definition"),
    ("demographic", "openEHR: Demographic"),
    ("admin", "openEHR: Admin"),
    ("management", "FerroEHR: Status & Management"),
    ("terminology", "FerroEHR: Terminology"),
    ("relationships", "FerroEHR: Party Relationships"),
    ("messaging", "FerroEHR: Messaging"),
    ("events", "FerroEHR: Event Subscriptions"),
    ("audit", "FerroEHR: Audit"),
    ("smart", "FerroEHR: SMART Discovery"),
];
