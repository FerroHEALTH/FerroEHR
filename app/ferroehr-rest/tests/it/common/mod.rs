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
use ferroehr::config::rest::{AdminConfig, RestConfig};
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

/// The configured openEHR base path the ITS-REST routes hang off.
pub(crate) const BASE: &str = "/ferroehr/rest/openehr/v1";

/// The root FerroEHR's own extensions hang off, beside [`BASE`] (#4330).
pub(crate) const EXT: &str = "/ferroehr/rest/extensions/v1";

/// The root of the FHIR and IHE interfaces, beside [`BASE`] (#4330).
pub(crate) const FHIR: &str = "/ferroehr/rest/fhir/r4";

/// The paths below [`EXT`] the tests name relative to their namespace: every
/// route ITS-REST does not define that is no FHIR interface.
const EXTENSION_ROOTS: &[&str] = &[
    "/admin/access-delay",
    "/admin/access-opt-out",
    "/admin/access-restriction",
    "/admin/archive",
    "/admin/config",
    "/admin/dump",
    "/admin/event_subscription",
    "/admin/integrity",
    "/admin/load",
    "/admin/party",
    "/admin/query",
    "/admin/report",
    "/admin/research-objection",
    "/admin/restriction",
    "/admin/retention",
    "/admin/template",
    "/definition/archetype",
    "/definition/artefact",
    "/demographic/party_relationship",
    "/demographic/versioned_party_relationship",
    "/message",
    "/query/cohort",
    "/terminology",
];

/// The full server path of a route a test names relative to its namespace:
/// below [`EXT`] for an extension, below the REST root for `/fhir/r4/...`,
/// below [`BASE`] for everything else. A path already spelled from the REST
/// root (`/ferroehr/...`) is returned as it is.
pub(crate) fn full(relative: &str) -> String {
    if relative.starts_with("/ferroehr/") {
        return relative.to_owned();
    }
    let under = |root: &str| {
        relative
            .strip_prefix(root)
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/') || rest.starts_with('?'))
    };
    if EXTENSION_ROOTS.iter().any(|root| under(root)) {
        format!("{EXT}{relative}")
    } else if let Some(fhir) = relative.strip_prefix("/fhir/r4") {
        format!("{FHIR}{fhir}")
    } else {
        format!("{BASE}{relative}")
    }
}

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
    let authenticator = Authenticator::new(&config, None)
        .await
        .expect("test auth config is valid");
    // A router a test drives is serving, as `build_with` marks it.
    let observability = ferroehr_rest::extensions::management::Observability::default();
    observability.lifecycle.booted();
    let state = AppState::with_parts(config, service, None, observability);
    ferroehr_rest::router::router(state, authenticator)
}

/// The assembled router over a real service with authentication disabled —
/// the baseline most HTTP tests want (auth-specific tests build their own
/// [`AppConfig`] via [`router_with`]).
pub(crate) async fn test_router() -> (testkit::TestDb, Router) {
    let (db, service) = test_service().await;
    (db, unauthenticated_router(service).await)
}

/// The assembled router over `service` with authentication disabled.
pub(crate) async fn unauthenticated_router(service: Arc<FerroEhrService>) -> Router {
    let mut config = AppConfig {
        server: server(),
        ..AppConfig::default()
    };
    config.auth.enabled = false;
    router_with(config, service).await
}

/// The `[server]` section the wire suites serve under: the family defaults with
/// a body ceiling that admits the operational templates they upload, which a
/// deployment raises the same way (`server.body_limit_bytes`); the IPS template
/// alone is larger than the family's 1 MiB default.
pub(crate) fn server() -> ferrohealth_server::section::ServerSection {
    ferrohealth_server::section::ServerSection {
        body_limit_bytes: 64 * 1024 * 1024,
        ..ferrohealth_server::section::ServerSection::default()
    }
}

/// The unauthenticated API configuration the wire suites drive: [`BASE`] as the
/// base path, no Swagger UI, no permissive CORS, and the ADMIN group gated by
/// `admin_enabled`.
pub(crate) fn api_config(admin_enabled: bool) -> AppConfig {
    AppConfig {
        server: server(),
        rest: RestConfig {
            base_path: BASE.to_owned(),
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
/// user with the password `pw` and [`FULL_SCOPES`]; the roles go to the
/// user's `[[auth.basic.users]] roles`.
pub(crate) fn with_basic_users(config: &mut AppConfig, users: &[(&str, &[&str])]) {
    let hash = hash_password("pw");
    let entries: Vec<(&str, &str, &str)> = users
        .iter()
        .map(|(user, _roles)| (*user, hash.as_str(), FULL_SCOPES))
        .collect();
    config.auth = basic_auth(&entries);
    if let Some(basic) = config.auth.basic.as_mut() {
        for entry in &mut basic.users {
            if let Some((_, roles)) = users.iter().find(|(user, _)| *user == entry.user) {
                entry.roles = roles.iter().map(|role| (*role).to_owned()).collect();
            }
        }
    }
}

/// A Basic `Authorization` field value for `user`, with the password
/// [`with_basic_users`] gives every user.
pub(crate) fn basic_credential(user: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{user}:pw").as_bytes())
    )
}

/// An audited app over the local Audit Record Repository, with `user` an
/// ADMIN, every EHR resolving to the synthetic `subject`, and one EHR that
/// `user` created; returns the EHR id with the store's pool.
pub(crate) async fn audited_app(
    subject: &'static str,
    user: &str,
) -> (testkit::TestDb, PgPool, Router, String) {
    use ferroehr::system_log::config::{AuditConfig, StoreConfig};
    use ferroehr::system_log::sender::{
        AuditHandle, ResolvedSubject, SubjectLookup, SubjectResolver, start,
    };
    let (db, pool) = migrated_pool().await;
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
    let resolver: SubjectResolver = Arc::new(move |_ehr_id| {
        Box::pin(async move {
            SubjectLookup::Found(ResolvedSubject {
                id: subject.to_owned(),
                namespace: Some("urn:ferroehr:test-subjects".to_owned()),
            })
        })
    });
    let (sender, _handle): (_, AuditHandle) = start(audit, Some(resolver), Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(ferroehr::system_log::store::AuditStore::new(pool.clone())),
    );
    let mut config = api_config(true);
    with_basic_users(&mut config, &[(user, &["ADMIN"])]);
    let app = ferroehr_rest::build_full(
        config,
        svc,
        None,
        ferroehr_rest::extensions::management::Observability::default(),
    )
    .await
    .expect("build app");
    let req = Request::post(format!("{BASE}/ehr"))
        .header(header::AUTHORIZATION, basic_credential(user))
        .header("Prefer", "return=representation")
        .body(Body::empty())
        .expect("the EHR create request");
    let (status, body) = send_body(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: Value = serde_json::from_str(&body).expect("EHR json");
    let ehr = value
        .pointer("/ehr_id/value")
        .and_then(Value::as_str)
        .expect("ehr_id")
        .to_owned();
    (db, pool, app, ehr)
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
        content_of_version: Arc::new(|_, _| Box::pin(async { Ok::<_, ResolveError>(None) })),
        content: Arc::default(),
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

/// A request under construction: [`req`] starts it, and the methods below set
/// its body and the headers the suites send most.
pub(crate) struct Req {
    builder: http::request::Builder,
    body: Body,
}

/// A `method` request for the absolute path `uri`, with an empty body until
/// [`Req::json`] or [`Req::body`] sets one.
pub(crate) fn req<U>(method: &str, uri: U) -> Req
where
    http::Uri: TryFrom<U>,
    <http::Uri as TryFrom<U>>::Error: Into<http::Error>,
{
    Req {
        builder: Request::builder().method(method).uri(uri),
        body: Body::empty(),
    }
}

impl Req {
    /// Adds the header `key: value`.
    pub(crate) fn header<K, V>(mut self, key: K, value: V) -> Self
    where
        header::HeaderName: TryFrom<K>,
        <header::HeaderName as TryFrom<K>>::Error: Into<http::Error>,
        header::HeaderValue: TryFrom<V>,
        <header::HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.builder = self.builder.header(key, value);
        self
    }

    /// Sets `body` as the request body.
    pub(crate) fn body(mut self, body: impl Into<Body>) -> Self {
        self.body = body.into();
        self
    }

    /// Sets `body` as an `application/json` request body.
    pub(crate) fn json(self, body: impl Into<Body>) -> Self {
        self.header(header::CONTENT_TYPE, "application/json")
            .body(body)
    }

    /// Adds `Accept: value`.
    pub(crate) fn accept<V>(self, value: V) -> Self
    where
        header::HeaderValue: TryFrom<V>,
        <header::HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.header(header::ACCEPT, value)
    }

    /// Adds `Prefer: return=representation`.
    pub(crate) fn prefer_representation(self) -> Self {
        self.header("Prefer", "return=representation")
    }

    /// Adds `If-Match: value`.
    pub(crate) fn if_match<V>(self, value: V) -> Self
    where
        header::HeaderValue: TryFrom<V>,
        <header::HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.header(header::IF_MATCH, value)
    }

    /// Adds `Authorization: credential`.
    pub(crate) fn auth<V>(self, credential: V) -> Self
    where
        header::HeaderValue: TryFrom<V>,
        <header::HeaderValue as TryFrom<V>>::Error: Into<http::Error>,
    {
        self.header(header::AUTHORIZATION, credential)
    }

    /// The finished request.
    pub(crate) fn build(self) -> Request<Body> {
        self.builder.body(self.body).expect("request")
    }
}

/// [`send`] with the response body read as JSON.
pub(crate) async fn send_json(app: &Router, req: Request<Body>) -> (StatusCode, Value) {
    let (status, _headers, body) = send(app, req).await;
    (
        status,
        serde_json::from_str(&body).expect("a JSON response body"),
    )
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

/// The value of the response header `name`, when present and visible ASCII.
pub(crate) fn header<K: header::AsHeaderName>(
    headers: &header::HeaderMap,
    name: K,
) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

/// The bare uid inside a weak `ETag` (`W/"{uid}"`).
pub(crate) fn etag_uid(headers: &header::HeaderMap) -> String {
    header(headers, header::ETAG)
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

/// The Demo Vitals operational template (canonical XML) vendored in `openehr-sdt`.
pub(crate) fn demo_vitals_opt_xml() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/openehr-sdt/tests/fixtures/better/Demo Vitals.opt"),
    )
    .expect("Demo Vitals.opt vendored in openehr-sdt")
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

/// The process's one telemetry provider for the tests, with FerroEHR's
/// instruments registered on its meter, built once as the binary builds it.
pub(crate) fn telemetry() -> &'static Arc<ferrohealth_telemetry::telemetry::Telemetry> {
    static TELEMETRY: std::sync::OnceLock<Arc<ferrohealth_telemetry::telemetry::Telemetry>> =
        std::sync::OnceLock::new();
    TELEMETRY.get_or_init(|| {
        let telemetry = ferrohealth_telemetry::telemetry::Telemetry::new(
            &ferrohealth_telemetry::section::TelemetrySection::default(),
            ferrohealth_telemetry::telemetry::Service::new(ferroehr::telemetry::SERVICE, "test"),
        )
        .expect("build the telemetry provider");
        ferroehr::telemetry::metrics::init(&telemetry.meter(ferroehr::telemetry::metrics::SCOPE));
        Arc::new(telemetry)
    })
}

/// A live console filter for `/management/loggers`, its subscriber kept for
/// the life of the test process.
pub(crate) fn log_filter() -> ferroehr_rest::extensions::management::LogFilter {
    use tracing_subscriber::layer::SubscriberExt as _;

    type Live = Box<dyn tracing::Subscriber + Send + Sync>;
    static FILTER: std::sync::OnceLock<(Live, ferroehr_rest::extensions::management::LogFilter)> =
        std::sync::OnceLock::new();
    let (_subscriber, filter) = FILTER.get_or_init(|| {
        let section = ferrohealth_telemetry::section::TelemetrySection::default();
        let layers = telemetry()
            .layers::<tracing_subscriber::Registry, _>(
                &section,
                &ferrohealth_telemetry::telemetry::Console::new(false, None),
                std::io::sink,
            )
            .expect("console layers");
        let subscriber: Live = Box::new(tracing_subscriber::registry().with(layers.layers));
        (
            subscriber,
            ferroehr_rest::extensions::management::LogFilter {
                handle: layers.filter,
                boot: section.filter,
            },
        )
    });
    filter.clone()
}

/// The FHIR `AuditEvent` the audit store serves for the record at chain
/// position `chain_seq`, rendered from its columns.
pub(crate) async fn rendered_audit_event(pool: &PgPool, chain_seq: i64) -> Value {
    ferroehr::system_log::store::AuditStore::new(pool.clone())
        .rendered(chain_seq)
        .await
        .expect("render the stored audit record")
        .expect("the audit record is stored")
}

/// The columns migration 0017 added of every stored audit record, then every
/// record's served FHIR document, as text, for the tests that assert a value
/// never reaches the trail.
pub(crate) async fn audit_texts(pool: &PgPool) -> Vec<String> {
    use ferroehr::system_log::store::{AuditSearchFilter, AuditStore};
    let mut texts: Vec<String> = sqlx::query_scalar(
        "SELECT concat_ws(' ', issuer, client_id, ehr_id, subject_namespace, \
         category_content::text) FROM audit.audit_event ORDER BY chain_seq",
    )
    .fetch_all(pool)
    .await
    .expect("read the stored audit rows");
    let (_, documents) = AuditStore::new(pool.clone())
        .search(&AuditSearchFilter {
            count: 1000,
            ..AuditSearchFilter::default()
        })
        .await
        .expect("render the stored audit records");
    texts.extend(documents.iter().map(ToString::to_string));
    texts
}
