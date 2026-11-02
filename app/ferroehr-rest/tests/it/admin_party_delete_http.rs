// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end tests for the physical party delete extension route
//! (`DELETE /admin/party/{party_id}`), which physically deletes a party along
//! with its relationships and answers `party_id_does_not_exist` for an unknown
//! one.
//!
//! No ITS-REST operation governs the route — our own extension. The tests drive
//! it through the assembled router with the RBAC gate on and the local audit
//! store recording: a non-admin is refused, an admin deletes, and the deletion
//! leaves the access record the EHR physical delete leaves for an EHR.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::config::rest::AdminConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::sender::start;
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;
use crate::common::BASE;
use crate::common::EXT;

const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "ferroehr";

/// A syntactically valid party id that is never created.
const UNKNOWN_PARTY: &str = "11111111-2222-4333-8444-555555555555";

/// Bearer authentication, the admin group mounted, the default RBAC gate.
fn config() -> AppConfig {
    AppConfig {
        server: common::server(),
        auth: common::issuer_auth(ISSUER, AUDIENCE),
        admin: AdminConfig { enabled: true },
        ..Default::default()
    }
}

/// An audited app over a fresh database with the local store on.
async fn app() -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = common::local_audit();
    let (sender, _handle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender)
        .with_audit_store(AuditStore::new(pool.clone()));
    let authz = AuthzHandle::build(
        &AuthzConfig::default(),
        &config().rest.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new);
    let app = ferroehr_rest::build_full(config(), Arc::new(svc), authz, Observability::default())
        .await
        .expect("build app");
    (db, pool, app)
}

/// A bearer for `subject` holding `role`.
fn bearer(subject: &str, role: &str) -> String {
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).expect("timestamp") + 3600;
    let claims: Value = json!({
        "sub": subject,
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": exp,
        "roles": [role],
    });
    common::bearer(&claims)
}

/// Sends `method uri` as `credential`, with an optional JSON body.
async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    credential: &str,
    body: Option<&Value>,
) -> (StatusCode, header::HeaderMap) {
    let request = common::req(method, uri)
        .auth(credential)
        .accept("application/json")
        .json(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .build();
    let (status, headers, _) = common::send(app, request).await;
    (status, headers)
}

/// Creates a PERSON as `credential` and returns its versioned-object id.
async fn create_person(app: &Router, credential: &str) -> String {
    let (status, headers) = call(
        app,
        "POST",
        &format!("{BASE}/demographic/person"),
        credential,
        Some(&crate::demographic_http::person_body()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create person");
    let version_uid = common::etag_uid(&headers);
    version_uid
        .split("::")
        .next()
        .expect("a versioned-object id")
        .to_owned()
}

/// The status of `GET /demographic/person/{party}` as `credential`.
async fn person_status(app: &Router, party: &str, credential: &str) -> StatusCode {
    call(
        app,
        "GET",
        &format!("{BASE}/demographic/person/{party}"),
        credential,
        None,
    )
    .await
    .0
}

/// The `(action, resource_class, resource_id)` of `principal`'s
/// `admin_party_delete` records, once the drain has written one (60 s cap).
async fn delete_records(pool: &PgPool, principal: &str) -> Vec<(String, String, Option<String>)> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let rows: Vec<(String, String, Option<String>)> = sqlx::query_as(
            "SELECT action, resource_class, resource_id FROM audit.audit_event \
             WHERE principal = $1 AND operation = 'admin_party_delete'",
        )
        .bind(principal)
        .fetch_all(pool)
        .await
        .expect("read the trail");
        if !rows.is_empty() || Instant::now() > deadline {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// A principal without the admin role is refused with `403`, and the party
/// survives.
#[tokio::test]
async fn a_non_admin_is_refused_and_the_party_survives() {
    let (_db, _pool, app) = app().await;
    let admin = bearer("party-admin", "ADMIN");
    let party = create_person(&app, &admin).await;

    let (status, _) = call(
        &app,
        "DELETE",
        &format!("{EXT}/admin/party/{party}"),
        &bearer("party-clinician", "USER"),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN);
    assert_eq!(person_status(&app, &party, &admin).await, StatusCode::OK);
}

/// An admin's delete answers `204`, the party is gone, and the access record
/// is a `D` on the demographic resource naming the party, as the EHR physical
/// delete records a `D` on the EHR.
#[tokio::test]
async fn an_admin_deletes_the_party_and_the_access_record_names_it() {
    let (_db, pool, app) = app().await;
    let admin = bearer("party-eraser", "ADMIN");
    let party = create_person(&app, &admin).await;

    let (status, headers) = call(
        &app,
        "DELETE",
        &format!("{EXT}/admin/party/{party}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(headers.get(header::LOCATION).is_none());
    assert_eq!(
        person_status(&app, &party, &admin).await,
        StatusCode::NOT_FOUND
    );

    assert_eq!(
        delete_records(&pool, "party-eraser").await,
        vec![("D".to_owned(), "demographic".to_owned(), Some(party))]
    );
}

/// An unknown party is `404` (`party_id_does_not_exist`), a malformed id
/// `400`.
#[tokio::test]
async fn an_unknown_party_is_404_and_a_malformed_id_400() {
    let (_db, _pool, app) = app().await;
    let admin = bearer("party-prober", "ADMIN");
    let (unknown, _) = call(
        &app,
        "DELETE",
        &format!("{EXT}/admin/party/{UNKNOWN_PARTY}"),
        &admin,
        None,
    )
    .await;
    assert_eq!(unknown, StatusCode::NOT_FOUND);
    let (malformed, _) = call(
        &app,
        "DELETE",
        &format!("{EXT}/admin/party/not-a-uuid"),
        &admin,
        None,
    )
    .await;
    assert_eq!(malformed, StatusCode::BAD_REQUEST);
}
