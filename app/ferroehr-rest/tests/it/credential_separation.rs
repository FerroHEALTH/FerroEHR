// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The two-DSN posture: the server assembled on one login role per
//! pseudonymisation domain, serving both domains and reaching neither across.
//!
//! Every other suite here runs the schema separation through ONE credential —
//! `FerroEhrService::new` derives the demographic pool from the clinical pool's
//! own connect options and swaps only the `search_path`. That proves the
//! routing, and nothing about the posture the book recommends and the chart
//! configures: `[storage.party] url` pointing at a role that is a member of
//! `ferroehr_party` while `[db] url` authenticates as `ferroehr_clinical`.
//!
//! So these tests take the path the binary takes — two [`DbConfig`] DSNs,
//! [`ferroehr::db::connect_domains`], whose four
//! pools handed to the service — and then assert three things: both domains
//! serve through the assembled router, each of the server's OWN pools is
//! refused the other domain's relations for want of privilege, and a
//! single-DSN deployment still serves both domains, so the split stays a
//! choice rather than a requirement.
//!
//! NOTE: no openEHR spec governs database roles or the pseudonymisation
//! boundary — our own design/extension (GDPR Art. 4(5) and Art. 32(1)(a); the
//! migrations carry the derivation).

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use axum::Router;
use ferroehr::config::secret::SecretUrl;
use ferroehr::db::DbConfig;
use ferroehr::db::domain::{Domain, DomainDsn, StorageConfig};
use ferroehr::service::FerroEhrService;
use http::StatusCode;
use sqlx::PgPool;
use uuid::Uuid;

use crate::common;
use crate::common::BASE;

/// `SQLSTATE` 42501 `insufficient_privilege` — what `PostgreSQL` reports for a
/// refused read, whether the missing grant is on the relation or on its schema
/// (`PostgreSQL` docs § Appendix A "`PostgreSQL` Error Codes", class 42).
const SQLSTATE_INSUFFICIENT_PRIVILEGE: &str = "42501";

/// Relations of the demographic domain no clinical credential may read.
const DEMOGRAPHIC_RELATIONS: &[&str] = &[
    "party.version",
    "party.version_cold",
    "party.node",
    "party.vo_head",
    "party.contribution",
];

/// Relations of the clinical domain no demographic credential may read.
const CLINICAL_RELATIONS: &[&str] = &[
    "clinical.ehr",
    "clinical.version",
    "clinical.version_cold",
    "clinical.node",
    "clinical.vo_head",
];

/// A password for a throwaway login role, fresh per call.
///
/// The value is never a secret: the role lives as long as one test against an
/// ephemeral clone. It is generated rather than written down because a literal
/// here is indistinguishable, to a scanner and to a reader, from a credential
/// that does matter.
fn throwaway_password() -> String {
    format!("pw{}", Uuid::now_v7().simple())
}

/// Rewrite the userinfo of a testkit clone DSN so a pool can connect to the
/// same database as a different login role (scheme/host/port/database
/// preserved).
fn with_role(base_url: &str, user: &str, password: &str) -> String {
    let (scheme, rest) = base_url.split_once("://").expect("dsn scheme");
    let host_and_path = rest.split_once('@').map_or(rest, |(_, tail)| tail);
    format!("{scheme}://{user}:{password}@{host_and_path}")
}

/// A DSN authenticating as a fresh non-superuser login role that is a member
/// of `domain_roles` (a comma-separated `IN ROLE` list) — how a production
/// deployment runs, and never as superuser, which would bypass both RLS and
/// every privilege check these tests are about.
///
/// Roles are cluster-global on the shared testkit server, so the login role is
/// named off the clone's database name and the testkit sweep reaps it.
async fn dsn_as(db: &testkit::TestDb, suffix: &str, domain_roles: &str) -> String {
    let login = format!("{}_{suffix}", db.name());
    let password = throwaway_password();
    sqlx::query(sqlx::AssertSqlSafe(format!(
        "CREATE ROLE {login} LOGIN PASSWORD '{password}' IN ROLE {domain_roles}"
    )))
    .execute(&db.pool())
    .await
    .expect("create the login role");
    with_role(db.url(), &login, &password)
}

/// The two pools and the router the binary would assemble from `settings`.
///
/// This is `ferroehr-server`'s own sequence (`connect_domains` →
/// `FerroEhrService::new` → `with_demographic_pool`), so the credential each
/// domain uses is the one the configuration names rather than one derived from
/// the other.
struct Deployment {
    /// The pool serving the `ehr` schema.
    clinical: PgPool,
    /// The pool serving the `demographic` schema.
    demographic: PgPool,
    /// The assembled ITS-REST router over both.
    router: Router,
}

/// Build the two pools and the router `settings` and `storage` describe.
async fn deployment(settings: &DbConfig, storage: &StorageConfig) -> Deployment {
    let pools = ferroehr::db::connect_domains(settings, storage)
        .await
        .expect("the domain pools connect");
    let clinical = pools.clinical.clone();
    let demographic = pools.party.clone();
    let service = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&clinical))
            .with_demographic_pool(demographic.clone()),
    );
    let router = common::router_with(common::api_config(false), service);
    Deployment {
        clinical,
        demographic,
        router,
    }
}

/// Create an EHR through the wire and read it back.
async fn the_clinical_domain_serves(router: &Router) {
    let ehr_id = common::create_ehr(router).await;
    let (status, body) =
        common::send_body(router, common::get(&format!("{BASE}/ehr/{ehr_id}"))).await;
    assert_eq!(status, StatusCode::OK, "read the EHR back: {body}");
}

/// Commit a PERSON through the wire and read it back.
async fn the_demographic_domain_serves(router: &Router) {
    let (status, headers, body) = common::send(
        router,
        common::post_json(
            &format!("{BASE}/demographic/person"),
            &crate::demographic_http::person_body().to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit a PERSON: {body}");
    let ovid = common::etag_uid(&headers);
    let (status, body) = common::send_body(
        router,
        common::get(&format!("{BASE}/demographic/person/{ovid}")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "read the PERSON back: {body}");
}

/// Assert that `pool` — a pool the assembled server is holding, not a fresh
/// connection opened for the occasion — is refused every relation in
/// `relations` for want of privilege.
async fn refused_every_relation(pool: &PgPool, whose: &str, relations: &[&str]) {
    for relation in relations {
        let probe = format!("SELECT 1 FROM {relation} LIMIT 1");
        let error = sqlx::query(sqlx::AssertSqlSafe(probe))
            .execute(pool)
            .await
            .expect_err(&format!(
                "the {whose} pool must not be able to read {relation}"
            ));
        let code = error
            .as_database_error()
            .and_then(sqlx::error::DatabaseError::code)
            .map(std::borrow::Cow::into_owned);
        assert_eq!(
            code.as_deref(),
            Some(SQLSTATE_INSUFFICIENT_PRIVILEGE),
            "the {whose} pool reading {relation} must be refused for want of \
             privilege, not fail some other way: {error}"
        );
    }
}

/// Two DSNs, two login roles: both domains serve, and neither of the server's
/// own pools can reach the other domain.
///
/// The refusal half is measured through the pools the router is dispatching
/// on — `PgPool` is a handle, so the clones held here and the ones inside the
/// service are one pool — because the claim is about the RUNNING server's
/// credentials, not about what a fresh connection would be allowed.
#[tokio::test]
async fn two_credentials_serve_both_domains_and_reach_neither_across() {
    let db = common::test_db().await;
    let settings = DbConfig {
        url: SecretUrl::new(dsn_as(&db, "credclin", "ferroehr_clinical").await),
        ..DbConfig::default()
    };
    let storage = StorageConfig {
        party: DomainDsn {
            url: Some(SecretUrl::new(
                dsn_as(&db, "creddemo", "ferroehr_party").await,
            )),
            url_file: None,
        },
        ..StorageConfig::default()
    };
    assert!(
        storage.is_separated(Domain::Party),
        "the fixture must actually configure two DSNs, else this passes vacuously"
    );
    assert_ne!(
        storage.dsn(Domain::Party, &settings),
        settings.url.expose(),
        "and they must be different credentials"
    );

    let deployment = deployment(&settings, &storage).await;

    the_clinical_domain_serves(&deployment.router).await;
    the_demographic_domain_serves(&deployment.router).await;

    refused_every_relation(&deployment.clinical, "clinical", DEMOGRAPHIC_RELATIONS).await;
    refused_every_relation(&deployment.demographic, "demographic", CLINICAL_RELATIONS).await;
}

/// One DSN: the fallback posture still serves both domains, so the credential
/// split stays a deployment choice rather than a requirement.
///
/// The credential here is a member of both domain roles, which is what a stock
/// deployment (and the compose stack) runs, so the schema separation is the
/// only boundary in force — exactly the half the two-credential test above
/// adds to.
#[tokio::test]
async fn one_credential_still_serves_both_domains() {
    let db = common::test_db().await;
    let settings =
        DbConfig::new(dsn_as(&db, "credboth", "ferroehr_clinical, ferroehr_party").await);
    let storage = StorageConfig::default();
    assert!(
        !storage.is_separated(Domain::Party),
        "the fallback posture leaves `[storage.party] url` unset"
    );
    assert_eq!(
        storage.dsn(Domain::Party, &settings),
        settings.url.expose(),
        "and the party pool falls back to the shared DSN"
    );

    let deployment = deployment(&settings, &storage).await;

    the_clinical_domain_serves(&deployment.router).await;
    the_demographic_domain_serves(&deployment.router).await;
}
