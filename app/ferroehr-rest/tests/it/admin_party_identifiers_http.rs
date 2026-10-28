// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end tests for the audited read of a party's protected national
//! identifiers (`GET /admin/party/identifiers`, #3776).
//!
//! No openEHR spec governs the route — our own extension. The tests drive it
//! through the assembled router with identifier protection on, the RBAC gate on
//! and the local audit store recording: an admin without
//! `authz.rbac.identifier_reader_role` is refused, a reader sees the value each
//! version committed, and every read leaves an access record naming the
//! reader, the party and the scheme, never the value.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
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
use ferroehr::service::demographic::identifier::config::IdentifierProtectionConfig;
use ferroehr::service::demographic::identifier::engine::IdentifierProtection;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::sender::start;
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;
use crate::common::BASE;
use crate::common::EXT;

const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "ferroehr";
const READER_ROLE: &str = "IDENTIFIER_READER";

/// A synthetic BSN that passes the eleven-test.
const FIRST_BSN: &str = "111222333"; // privacy-allow: synthetic
/// A second synthetic BSN that passes the eleven-test.
const SECOND_BSN: &str = "123456782"; // privacy-allow: synthetic

/// The identifier-protection root key of the test deployment.
const TEST_ROOT_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

/// Bearer authentication, the admin group mounted, the identifier reader role
/// configured.
fn config() -> AppConfig {
    let mut config = AppConfig {
        server: common::server(),
        auth: common::issuer_auth(ISSUER, AUDIENCE),
        admin: AdminConfig { enabled: true },
        ..Default::default()
    };
    config.rbac.identifier_reader_role = Some(READER_ROLE.to_owned());
    config
}

/// An audited app over a fresh database protecting `nl-bsn`, with the local
/// store on.
async fn app() -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
            full_verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    let (sender, _handle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let key = ferrohealth_secret::secret::Secret::new(TEST_ROOT_KEY);
    let engine = IdentifierProtection::from_config(
        &IdentifierProtectionConfig {
            enabled: true,
            schemes: vec!["nl-bsn".to_owned()],
            key: Some(ferrohealth_secret::secret::Secret::new(TEST_ROOT_KEY)),
            key_file: None,
        },
        Some(&key),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    )
    .expect("the engine builds")
    .expect("protection is enabled");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_identifier_protection(Arc::new(engine))
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

/// A bearer for `subject` holding `roles`.
fn bearer(subject: &str, roles: &[&str]) -> String {
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).expect("timestamp") + 3600;
    let claims: Value = json!({
        "sub": subject,
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": exp,
        "roles": roles,
    });
    common::bearer(&claims)
}

/// A PERSON carrying `bsn` as an `nl-bsn` identifier.
fn person_with(bsn: &str) -> Value {
    let mut person = crate::demographic_http::person_body();
    person["identities"][0]["details"]["items"]
        .as_array_mut()
        .expect("the identity items")
        .push(json!({
            "_type": "ELEMENT",
            "archetype_node_id": "at0004",
            "name": { "_type": "DV_TEXT", "value": "identifier" },
            "value": { "_type": "DV_IDENTIFIER", "type": "nl-bsn",
                       "id": bsn, "issuer": "RvIG", "assigner": "RvIG" }
        }));
    person
}

/// Sends `method uri` as `credential`, with optional extra headers and body.
async fn call(
    app: &Router,
    method: &str,
    uri: &str,
    credential: &str,
    if_match: Option<&str>,
    body: Option<&Value>,
) -> (StatusCode, header::HeaderMap, String) {
    let mut request = Request::builder()
        .method(method)
        .uri(uri)
        .header(header::AUTHORIZATION, credential)
        .header(header::ACCEPT, "application/json")
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(version) = if_match {
        request = request.header(header::IF_MATCH, format!("\"{version}\""));
    }
    let request = request
        .body(body.map_or_else(Body::empty, |b| Body::from(b.to_string())))
        .expect("request");
    common::send(app, request).await
}

/// Creates a PERSON with `FIRST_BSN`, then a second version with `SECOND_BSN`,
/// and returns the two version uids.
async fn two_versions(app: &Router, credential: &str) -> (String, String) {
    let (status, headers, _) = call(
        app,
        "POST",
        &format!("{BASE}/demographic/person"),
        credential,
        None,
        Some(&person_with(FIRST_BSN)),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "create the person");
    let first = common::etag_uid(&headers);
    let party = first.split("::").next().expect("a versioned-object id");
    let (status, headers, body) = call(
        app,
        "PUT",
        &format!("{BASE}/demographic/person/{party}"),
        credential,
        Some(&first),
        Some(&person_with(SECOND_BSN)),
    )
    .await;
    assert!(status.is_success(), "update the person: {status} {body}");
    (first, common::etag_uid(&headers))
}

/// The identifier the expanded version carries.
fn expanded_id(body: &str) -> String {
    let body: Value = serde_json::from_str(body).expect("a JSON body");
    let items = body["version"]["data"]["identities"][0]["details"]["items"]
        .as_array()
        .expect("the identity items");
    items
        .iter()
        .find_map(|item| item["value"]["id"].as_str())
        .expect("the identifier")
        .to_owned()
}

/// The `resource_id`s of `principal`'s demographic reads naming a national
/// identifier, once the drain has written `want` of them (60 s cap).
async fn identifier_records(pool: &PgPool, principal: &str, want: usize) -> Vec<String> {
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT resource_id FROM audit.audit_event \
             WHERE principal = $1 AND resource_id LIKE 'party:%#national-identifier%' \
             ORDER BY recorded_at",
        )
        .bind(principal)
        .fetch_all(pool)
        .await
        .expect("read the trail");
        if rows.len() >= want || Instant::now() > deadline {
            return rows;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// An admin without the identifier reader role is refused with `403`, and no
/// identifier is expanded or recorded as read.
#[tokio::test]
async fn an_admin_without_the_reader_role_is_refused() {
    let (_db, pool, app) = app().await;
    let admin = bearer("identifier-admin", &["ADMIN"]);
    let (first, _) = two_versions(&app, &admin).await;

    let (status, _, body) = call(
        &app,
        "GET",
        &format!("{EXT}/admin/party/identifiers?version_uid={first}"),
        &admin,
        None,
        None,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    assert!(!body.contains(FIRST_BSN), "the refusal leaks no value");

    let (status, _, _) = call(
        &app,
        "GET",
        &format!("{EXT}/admin/party/identifiers?version_uid={first}"),
        &bearer("identifier-clerk", &[READER_ROLE]),
        None,
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "the reader role without the admin class is refused too"
    );
    assert!(
        identifier_records(&pool, "identifier-admin", 1)
            .await
            .is_empty(),
        "a refused read expands nothing"
    );
}

/// A reader sees the value each version committed, the historical one
/// included, and each read is an access record naming the reader, the party
/// and the scheme, never the value.
#[tokio::test]
async fn a_reader_sees_each_versions_value_and_each_read_is_recorded() {
    let (_db, pool, app) = app().await;
    let reader = bearer("identifier-reader", &["ADMIN", READER_ROLE]);
    let (first, second) = two_versions(&app, &reader).await;
    let party = first.split("::").next().expect("a versioned-object id");

    for (version, bsn) in [(&first, FIRST_BSN), (&second, SECOND_BSN)] {
        let (status, _, body) = call(
            &app,
            "GET",
            &format!("{EXT}/admin/party/identifiers?version_uid={version}"),
            &reader,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(expanded_id(&body), bsn, "version {version}");
        let parsed: Value = serde_json::from_str(&body).expect("a JSON body");
        assert_eq!(parsed["expanded_schemes"], json!(["nl-bsn"]));
    }

    let records = identifier_records(&pool, "identifier-reader", 2).await;
    assert_eq!(
        records,
        vec![format!("party:{party}#national-identifier:nl-bsn"); 2],
        "one record per read, naming the party and the scheme"
    );
    for event in common::audit_texts(&pool).await {
        assert!(!event.contains(FIRST_BSN), "a value reached the trail");
        assert!(!event.contains(SECOND_BSN), "a value reached the trail");
    }
}

/// A missing or malformed `version_uid` is `400`, and a version no party
/// holds `404`.
#[tokio::test]
async fn a_bad_version_uid_is_400_and_an_unknown_one_404() {
    let (_db, _pool, app) = app().await;
    let reader = bearer("identifier-prober", &["ADMIN", READER_ROLE]);
    for (query, want) in [
        ("", StatusCode::BAD_REQUEST),
        ("?version_uid=not-a-version", StatusCode::BAD_REQUEST),
        (
            "?version_uid=11111111-2222-4333-8444-555555555555::ferroehr.local::1",
            StatusCode::NOT_FOUND,
        ),
    ] {
        let (status, _, body) = call(
            &app,
            "GET",
            &format!("{EXT}/admin/party/identifiers{query}"),
            &reader,
            None,
            None,
        )
        .await;
        assert_eq!(status, want, "{query}: {body}");
    }
}
