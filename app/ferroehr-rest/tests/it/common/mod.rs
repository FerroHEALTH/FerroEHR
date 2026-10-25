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
use ferroehr::config::server::{AdminConfig, ServerConfig};
use ferroehr::service::FerroEhrService;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authn::Authenticator;
use ferroehr_rest::extensions::access::authz::{AuthzResolvers, ResolveError};
use ferroehr_rest::state::AppState;
use ferrohealth_smart::config::{AuthSection, BasicSection, BasicUser, IssuerKey};
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
pub(crate) async fn router_with(config: AppConfig, service: Arc<FerroEhrService>) -> Router {
    let authenticator = Authenticator::new(&config)
        .await
        .expect("test auth config is valid");
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
    (db, router_with(config, service).await)
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
        auth: AuthSection {
            enabled: false,
            ..AuthSection::default()
        },
        admin: AdminConfig {
            enabled: admin_enabled,
        },
        ..Default::default()
    }
}

/// The scopes a test token or a test Basic user carries when the test is not
/// about SMART: every SMART on openEHR family with every permission, and the
/// product scope that admits the ADMIN and DEMOGRAPHIC APIs. Every route then
/// passes the SMART layer, and the test exercises the layer it is about.
pub(crate) const FULL_SCOPES: &str =
    "user/composition-*.cruds user/template-*.cruds user/aql-*.cruds __ferroehr.platform";

/// The `kid` of the committed CNF test key the fixtures sign with.
const TEST_KID: &str = "cnf-smart-test";

/// The committed CNF test issuer's public key set
/// (`docs/conformance/party/smart/`), written once to a file the resource
/// server reads.
pub(crate) fn jwks_file() -> PathBuf {
    static FILE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    FILE.get_or_init(|| {
        let path =
            std::env::temp_dir().join(format!("ferroehr-rest-it-jwks-{}.json", std::process::id()));
        std::fs::write(
            &path,
            include_str!("../../../../../docs/conformance/party/smart/jwks.json"),
        )
        .expect("write the test key set");
        path
    })
    .clone()
}

/// Bearer authentication trusting `issuer` for `audience`, its key set read
/// from the committed test key's file — no authorization server is reached.
pub(crate) fn issuer_auth(issuer: &str, audience: &str) -> AuthSection {
    AuthSection {
        issuer: IssuerKey::Flat(issuer.to_owned()),
        audience: vec![audience.to_owned()],
        jwks_file: jwks_file().display().to_string(),
        ..AuthSection::default()
    }
}

static METADATA_ISSUER: tokio::sync::OnceCell<String> = tokio::sync::OnceCell::const_new();

/// The URI of an authorization server shared by the whole test binary: it
/// serves RFC 8414 metadata naming a `token_endpoint` and the committed test
/// key set, so a server that serves the SMART discovery document can boot
/// against it. `wiremock` runs each server on a thread of its own, so the one
/// started here outlives the runtime of the test that started it.
pub(crate) async fn metadata_issuer() -> &'static str {
    METADATA_ISSUER
        .get_or_init(|| async {
            use wiremock::matchers::{method, path};
            use wiremock::{Mock, MockServer, ResponseTemplate};
            let server = MockServer::start().await;
            let uri = server.uri();
            Mock::given(method("GET"))
                .and(path("/.well-known/oauth-authorization-server"))
                .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "issuer": uri,
                    "jwks_uri": format!("{uri}/jwks"),
                    "authorization_endpoint": format!("{uri}/authorize"),
                    "token_endpoint": format!("{uri}/token"),
                    "grant_types_supported": ["authorization_code", "client_credentials"],
                    "token_endpoint_auth_methods_supported": ["client_secret_basic"],
                    "response_types_supported": ["code"],
                    "code_challenge_methods_supported": ["S256"],
                })))
                .mount(&server)
                .await;
            Mock::given(method("GET"))
                .and(path("/jwks"))
                .respond_with(ResponseTemplate::new(200).set_body_string(include_str!(
                    "../../../../../docs/conformance/party/smart/jwks.json"
                )))
                .mount(&server)
                .await;
            Box::leak(Box::new(server));
            uri
        })
        .await
}

/// The URI [`metadata_issuer`] started; call it only after that has run.
pub(crate) fn started_metadata_issuer() -> &'static str {
    METADATA_ISSUER
        .get()
        .expect("metadata_issuer runs before a token names it")
}

/// Basic authentication admitting each `(user, Argon2id PHC hash, scopes)`.
pub(crate) fn basic_auth(users: &[(&str, &str, &str)]) -> AuthSection {
    AuthSection {
        basic: Some(BasicSection {
            users: users
                .iter()
                .map(|(user, hash, scopes)| BasicUser {
                    user: (*user).to_owned(),
                    password_hash: Some(secrecy::SecretString::from((*hash).to_owned())),
                    password_hash_file: None,
                    scopes: scopes.split_whitespace().map(str::to_owned).collect(),
                    roles: Vec::new(),
                })
                .collect(),
            cache_ttl_seconds: 60,
        }),
        ..AuthSection::default()
    }
}

/// The Argon2id PHC hash of `password`, under a fixed salt so the fixtures
/// stay hermetic, at the default (OWASP-floor) costs.
pub(crate) fn hash_password(password: &str) -> String {
    use argon2::password_hash::PasswordHasher as _;
    argon2::Argon2::default()
        .hash_password_with_salt(password.as_bytes(), b"1234567890123456")
        .expect("hash")
        .to_string()
}

/// Turns Basic authentication on in `config` for each `(user, roles)`, every
/// user with the password `pw` and [`FULL_SCOPES`]; the roles go to
/// `[authz.rbac.basic_roles]`, where the RBAC gate reads a Basic user's roles.
pub(crate) fn with_basic_users(config: &mut AppConfig, users: &[(&str, &[&str])]) {
    let hash = hash_password("pw");
    let entries: Vec<(&str, &str, &str)> = users
        .iter()
        .map(|(user, _roles)| (*user, hash.as_str(), FULL_SCOPES))
        .collect();
    config.auth = basic_auth(&entries);
    for (user, roles) in users {
        config.rbac.basic_roles.insert(
            (*user).to_owned(),
            roles.iter().map(|role| (*role).to_owned()).collect(),
        );
    }
}

/// An `Authorization` field value carrying `claims` as an RFC 9068 access
/// token (`typ` `at+jwt`) signed with the committed test key.
///
/// The RFC 9068 §2.2 claims a test leaves out are filled in (`iat`, `jti`,
/// `client_id`), and a token whose claims name no `scope` carries
/// [`FULL_SCOPES`], so only a test about the scopes needs to name them.
pub(crate) fn bearer(claims: &Value) -> String {
    let mut claims = claims.clone();
    let now = jiff::Timestamp::now().as_second();
    if let Some(object) = claims.as_object_mut() {
        object.entry("iat").or_insert(serde_json::json!(now));
        object
            .entry("jti")
            .or_insert_with(|| serde_json::json!(uuid::Uuid::now_v7().to_string()));
        object
            .entry("client_id")
            .or_insert(serde_json::json!("ferroehr-test-client"));
        object
            .entry("scope")
            .or_insert(serde_json::json!(FULL_SCOPES));
    }
    let mut header = Header::new(Algorithm::RS256);
    header.typ = Some("at+jwt".to_owned());
    header.kid = Some(TEST_KID.to_owned());
    let key = EncodingKey::from_rsa_pem(include_bytes!(
        "../../../../../docs/conformance/party/smart/cnf-smart-test.key.pem"
    ))
    .expect("the committed test key");
    let token = encode(&header, &claims, &key).expect("encode");
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
