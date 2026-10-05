// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared SM commit-fixture family the topic modules import.
//!
//! Every suite that drives a write through the service layer needs the same
//! pieces: a `PARTY_IDENTIFIED` committer, an `openehr` change-type
//! `DV_CODED_TEXT`, a minimal valid RM COMPOSITION, and the SM
//! `UPDATE_VERSION` envelope carrying them (SM
//! `UML/classes/update_version.adoc`). They live here once, so the per-suite
//! copies cannot drift apart.
//!
//! A suite whose fixture identity is load-bearing (an assertion reads the
//! composer or committer name) takes the parameterized builder rather than a
//! private copy.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              fixture helpers; a failing fixture must panic at the fixture (the \
              Rust Book ch11)"
)]

use serde_json::{Value, json};

use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
use openehr_rm::prelude::PartyProxy;

/// The identity the shared fixtures write as — both the COMPOSITION `composer`
/// and the commit `committer`.
const TESTER: &str = "conformance tester";

/// Returns a `PARTY_IDENTIFIED` committer named `name`, as canonical JSON.
pub(crate) fn committer(name: &str) -> Value {
    json!({ "_type": "PARTY_IDENTIFIED", "name": name })
}

/// Returns a `PARTY_IDENTIFIED` committer named `name`, as the typed RM value.
pub(crate) fn committer_proxy(name: &str) -> PartyProxy {
    openehr_its::json::from_canonical_value(&committer(name)).expect("committer")
}

/// Returns the wire `DV_CODED_TEXT` naming `openehr` terminology `code` with
/// rubric `value`.
///
/// The `audit_change_type` group codes this carries are listed in RM common
/// `master06-change_control_package.adoc` §Contributions.
pub(crate) fn change_type(code: &str, value: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT", "value": value,
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": code
        }
    })
}

/// Returns a minimal *valid* RM COMPOSITION named `name`, composed by
/// `composer`.
///
/// `language`, `territory`, `category` and `composer` are all `1..1` (RM ehr,
/// COMPOSITION class), so typed RM validation rejects a fixture without them.
/// No template is referenced, so the fixture needs no `template_store` row.
pub(crate) fn composition_by(name: &str, composer: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": {
                "_type": "ARCHETYPE_ID",
                "value": "openEHR-EHR-COMPOSITION.encounter.v1"
            },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": name },
        "language": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "en"
        },
        "territory": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
            "code_string": "NL"
        },
        "category": {
            "_type": "DV_CODED_TEXT",
            "value": "event",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "433"
            }
        },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": composer }
    })
}

/// Returns a minimal *valid* RM COMPOSITION named `name`.
pub(crate) fn composition(name: &str) -> Value {
    composition_by(name, TESTER)
}

/// Returns the SM `UPDATE_VERSION` commit envelope for a bare-RM write.
///
/// `change_code` is the `openehr` `audit_change_type` code the version's audit
/// records (`249` creation, `251` modification, `523` deleted); `preceding`
/// names the version this one supersedes (RM common
/// `master06-change_control_package.adoc` §Contributions).
pub(crate) fn uv<T: serde::de::DeserializeOwned>(
    data: &Value,
    change_code: &str,
    preceding: Option<&str>,
) -> UpdateVersion<T> {
    UpdateVersion {
        preceding_version_uid: preceding.map(|p| p.parse().expect("OBJECT_VERSION_ID")),
        lifecycle_state: lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the fixture commit body decodes as its RM type"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded(change_code),
            description: None,
            committer: committer_proxy(TESTER),
            additional_properties: std::collections::BTreeMap::new(),
        }),
        signature: None,
        additional_properties: std::collections::BTreeMap::new(),
    }
}

/// The committed version's `uid/value` string off a served body.
pub(crate) fn uid(v: &Value) -> &str {
    v["uid"]["value"].as_str().expect("uid.value")
}

/// The bare versioned-object UUID of an `OBJECT_VERSION_ID` — everything
/// before the first separator (`object_version_id = object_id, '::',
/// creating_system_id, '::', version_tree_id`; BASE
/// `base_types/master05-identification_package.adoc` §Syntaxes).
pub(crate) fn vo_of(ovid: &str) -> &str {
    ovid.split("::").next().expect("vo uuid")
}

/// A minimal valid root FOLDER (RM ehr master04 §Folders).
pub(crate) fn folder(name: &str) -> Value {
    json!({
        "_type": "FOLDER",
        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
        "name": { "_type": "DV_TEXT", "value": name }
    })
}

// ── separated-credential fixtures ────────────────────────────────────────────

/// A password for a throwaway login role, fresh per call.
///
/// The value is never a secret: the role lives as long as one test against an
/// ephemeral clone. It is generated rather than written down because a literal
/// here is indistinguishable, to a scanner and to a reader, from a credential
/// that does matter, and the repository's own rule is that a finding is fixed
/// rather than suppressed.
pub(crate) fn throwaway_password() -> String {
    format!("pw{}", uuid::Uuid::now_v7().simple())
}

/// Rewrite the userinfo of a testkit clone DSN so a test can connect to the
/// same database as a different login role (scheme/host/port/database
/// preserved).
pub(crate) fn with_role(base_url: &str, user: &str, password: &str) -> String {
    let (scheme, rest) = base_url.split_once("://").expect("dsn scheme");
    let host_and_path = rest.split_once('@').map_or(rest, |(_, tail)| tail);
    format!("{scheme}://{user}:{password}@{host_and_path}")
}

/// A login DSN for a throwaway role holding `domain_role` and nothing else.
///
/// Roles are cluster-global on the shared testkit server, so the login role is
/// named off the clone's database name and the testkit sweep reaps it. The DSN
/// form is what lets a test build a real `DbConfig` on a separated credential.
pub(crate) async fn dsn_as(db: &testkit::TestDb, suffix: &str, domain_role: &str) -> String {
    let login = format!("{}_{suffix}", db.name());
    let password = throwaway_password();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD '{password}' IN ROLE {domain_role}"
    )))
    .execute(&db.pool())
    .await
    .expect("create the login role");
    with_role(db.url(), &login, &password)
}

/// Every link of an error's cause chain, outermost first, joined by ` <- `.
///
/// A curated service message says nothing about the driver fault behind it;
/// the chain does. A test that fails on a database step panics with this so
/// the cause is in the report rather than only on a trace nobody captured
/// (#3250).
pub(crate) fn source_chain(error: &dyn std::error::Error) -> String {
    let mut out = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        out.push_str(" <- ");
        out.push_str(&cause.to_string());
        next = cause.source();
    }
    out
}

/// The four domain pools of a co-located test database, and the layout that
/// describes it.
///
/// Every DB-backed test runs one testkit database with all four schemas in it,
/// which is the shared-DSN posture the boot gate reads as one credential.
pub(crate) fn shared_pools(pool: &sqlx::PgPool) -> ferroehr::db::domain::DomainPools {
    ferroehr::db::domain::DomainPools::from_shared(pool)
}

/// The domain layout of that same co-located database: four domains, one DSN.
pub(crate) fn shared_layout() -> ferroehr::db::domain::DomainLayout {
    ferroehr::db::domain::StorageConfig::default().layout(&ferroehr::db::DbConfig::default())
}
