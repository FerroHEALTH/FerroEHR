// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared test fixture: the REST router over a **real** `FerroEhrService` on a
//! **real `PostgreSQL` 18** from the shared `testkit` harness (one server,
//! one migrated template database, one `CREATE DATABASE … TEMPLATE` clone
//! per call — see `tools/testkit`).
//!
//! These HTTP tests exercise the same concrete service the binary ships, so
//! every scenario is set up through the real API (upload a template, create
//! an EHR, …) and every assertion observes real behaviour. Hold the returned
//! [`testkit::TestDb`] guard for the test's lifetime — dropping it releases
//! the clone.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::path::PathBuf;
use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use ferroehr::config::auth::{AuthConfig, OidcConfig};
use ferroehr::config::server::{AdminConfig, ServerConfig};
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::access::authz::{AuthzResolvers, ResolveError};
use ferroehr_rest::state::AppState;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::Value;
use sqlx::PgPool;
use tower::ServiceExt;

/// The configured API base path every gated route hangs off.
pub(crate) const BASE: &str = "/ferroehr/rest/openehr/v1";

/// A fresh, fully migrated database from the shared harness.
pub(crate) async fn test_db() -> testkit::TestDb {
    testkit::db().await.expect("testkit database")
}

/// A fresh, fully-migrated database and its pool. Hold the guard with the
/// pool.
pub(crate) async fn migrated_pool() -> (testkit::TestDb, PgPool) {
    let db = test_db().await;
    let pool = db.pool();
    (db, pool)
}

/// The real platform service over a fresh database.
pub(crate) async fn test_service() -> (testkit::TestDb, Arc<FerroEhrService>) {
    let (db, pool) = migrated_pool().await;
    (
        db,
        Arc::new(FerroEhrService::new(
            &ferroehr::db::domain::DomainPools::from_shared(&pool),
        )),
    )
}

/// The assembled router over a real service with the given configuration —
/// the same wiring as [`ferroehr_rest::build_with`], split open so tests can
/// hand-tune `AppConfig` (auth modes, admin/extension toggles).
pub(crate) fn router_with(config: AppConfig, service: Arc<FerroEhrService>) -> Router {
    let authenticator = Authenticator::new(config.auth.clone()).expect("test auth config is valid");
    let state = AppState::with_backend(config, service);
    ferroehr_rest::router::router(state, authenticator)
}

/// The assembled router over a real service with authentication disabled —
/// the baseline most HTTP tests want (auth-specific tests build their own
/// [`AppConfig`] via [`router_with`]).
pub(crate) async fn test_router() -> (testkit::TestDb, Router) {
    let mut config = AppConfig::default();
    config.auth.enabled = false;
    let (db, service) = test_service().await;
    (db, router_with(config, service))
}

/// The unauthenticated API configuration the wire suites drive: [`BASE`] as the
/// base path, no Swagger UI, no permissive CORS, and the ADMIN group gated by
/// `admin_enabled`.
pub(crate) fn api_config(admin_enabled: bool) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            bind: "127.0.0.1:0".to_owned(),
            base_path: BASE.to_owned(),
            max_in_flight: 1024,
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            cors_permissive: false,
            ..Default::default()
        },
        auth: AuthConfig {
            enabled: false,
            basic: None,
            oidc: None,
            ..AuthConfig::default()
        },
        admin: AdminConfig {
            enabled: admin_enabled,
        },
        ..Default::default()
    }
}

/// Bearer-only authentication against an HS256 issuer — orders of magnitude
/// cheaper than an Argon2 verification for suites that issue many requests.
pub(crate) fn hs256_auth_config(issuer: &str, audience: &str, secret: &str) -> AuthConfig {
    AuthConfig {
        enabled: true,
        basic: None,
        oidc: Some(OidcConfig {
            issuer: issuer.to_owned(),
            audiences: vec![audience.to_owned()],
            algorithms: vec!["HS256".to_owned()],
            hmac_secret: Some(ferroehr::config::secret::Secret::new(secret.to_owned())),
            jwks_json: None,
            ..OidcConfig::default()
        }),
        ..AuthConfig::default()
    }
}

/// An `Authorization` field value carrying `claims` as an HS256 token signed
/// with `secret`.
pub(crate) fn hs256_bearer(secret: &str, claims: &Value) -> String {
    let token = encode(
        &Header::new(Algorithm::HS256),
        claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("encode");
    format!("Bearer {token}")
}

/// Resolvers that resolve nothing — the shape a suite uses when its subject and
/// template attributes are irrelevant to what it asserts.
pub(crate) fn null_resolvers() -> AuthzResolvers {
    AuthzResolvers {
        subject: Arc::new(|_| Box::pin(async { Ok::<_, ResolveError>(None) })),
        template_of_version: Arc::new(|_, _| Box::pin(async { Ok::<_, ResolveError>(None) })),
    }
}

/// Drive one request through `app` and return its status, headers and body.
pub(crate) async fn send(
    app: &Router,
    req: Request<Body>,
) -> (StatusCode, header::HeaderMap, String) {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

/// [`send`] without the response headers.
pub(crate) async fn send_body(app: &Router, req: Request<Body>) -> (StatusCode, String) {
    let (status, _headers, body) = send(app, req).await;
    (status, body)
}

/// [`send`] keeping only the response status.
pub(crate) async fn send_status(app: &Router, req: Request<Body>) -> StatusCode {
    app.clone().oneshot(req).await.expect("response").status()
}

/// A bodyless request for `uri` (an absolute path, not a [`BASE`]-relative one).
pub(crate) fn request(method: &str, uri: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// A bodyless `GET` of the absolute path `uri`.
pub(crate) fn get(uri: &str) -> Request<Body> {
    request("GET", uri)
}

/// A bodyless `GET` of the absolute path `uri`, carrying `credential` as the
/// `Authorization` field value.
pub(crate) fn get_authorized(uri: &str, credential: &str) -> Request<Body> {
    Request::builder()
        .uri(uri)
        .header(header::AUTHORIZATION, credential)
        .body(Body::empty())
        .expect("request")
}

/// A bodyless `DELETE` of the absolute path `uri`.
pub(crate) fn delete(uri: &str) -> Request<Body> {
    request("DELETE", uri)
}

/// A JSON `POST` of `body` to the absolute path `uri`.
pub(crate) fn post_json(uri: &str, body: &str) -> Request<Body> {
    Request::builder()
        .method("POST")
        .uri(uri)
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_owned()))
        .expect("request")
}

/// The bare uid inside a weak `ETag` (`W/"{uid}"`).
pub(crate) fn etag_uid(headers: &header::HeaderMap) -> String {
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

/// Create an EHR through the released wire; return its server-assigned id
/// (read back from the create `ETag`).
pub(crate) async fn create_ehr(app: &Router) -> String {
    let (status, headers, _body) = send(app, request("POST", &format!("{BASE}/ehr"))).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create");
    etag_uid(&headers)
}

/// The vendored IPS operational template (canonical XML).
pub(crate) fn ips_opt_xml() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt"),
    )
    .expect("ips.v0.opt vendored in openehr-its")
}

/// The vendored IPS composition (canonical JSON) the template above validates.
///
/// The pair is driven end to end through the real `FerroEhrService` (upload →
/// create-EHR → commit) by the platform crate's validation suite, so it commits
/// cleanly over the wire too.
pub(crate) fn ips_canonical_composition() -> Value {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
        ),
    )
    .expect("ips_canonical.json vendored in openehr-its");
    serde_json::from_str(&text).expect("valid canonical composition")
}
