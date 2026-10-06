// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end RBAC gate over the real axum app.
//!
//! Drives the assembled router (auth + RBAC + dispatch) with `tower`'s
//! `oneshot`, over the **real** `FerroEhrService` (the scripted `Mock`
//! is gone), and asserts the coarse role gate: an Admin-class operation is 403
//! for a `USER` and clears the gate for an `ADMIN`; a clinical operation needs a
//! role; disabling RBAC restores today's behaviour; a deny is attributed to the
//! caller and audited by the ATNA layer; and scope→role extraction
//! clears the admin gate (a `scope` named `ADMIN` surfaces as role `ADMIN`).
//!
//! These assert only that the gate did not reject (`!= 403`): the concrete
//! post-gate status is real-backend behaviour, not the gate's, so pinning it
//! here would couple an authorization test to unrelated handler outcomes.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::time::Duration;

use argon2::Argon2;
use argon2::password_hash::PasswordHasher;
use axum::Router;
use axum::body::Body;
use ferroehr::config::auth::{AuthConfig, BasicConfig, BasicUser, OidcConfig};
use ferroehr::config::authz::AuthzConfig;
use ferroehr::config::server::AdminConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig, SyslogConfig, Transport};
use ferroehr::system_log::sender::{AuditSender, start};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::{AuthzHandle, AuthzResolvers, ResolveError};

use crate::common;
use http::{Request, StatusCode};
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::{Value, json};
use tokio::net::UdpSocket;
use tower::ServiceExt;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const HMAC_SECRET: &str = "rbac-test-secret";
const ISSUER: &str = "https://issuer.example";
/// The audience every fixture token is minted for: `audiences` is mandatory
/// whenever `[auth.oidc]` is present, so a token for another resource server
/// can never authenticate here.
const AUDIENCE: &str = "ferroehr";
const EHR_ID: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

// ── config + app assembly ─────────────────────────────────────────────────────

fn hash_pw(pw: &str) -> String {
    Argon2::default()
        .hash_password_with_salt(pw.as_bytes(), b"1234567890123456")
        .expect("hash")
        .to_string()
}

fn user(name: &str, roles: &[&str]) -> BasicUser {
    BasicUser {
        username: name.to_owned(),
        password_hash: ferroehr::config::secret::Secret::new(hash_pw("pw")),
        password_hash_file: None,
        roles: roles.iter().map(|r| (*r).to_owned()).collect(),
    }
}

fn rest_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        auth: AuthConfig {
            enabled: true,
            basic: Some(BasicConfig {
                users: vec![
                    user("user", &["USER"]),
                    user("root", &["ADMIN"]),
                    user("noroles", &[]),
                ],
            }),
            oidc: Some(OidcConfig {
                issuer: ISSUER.to_owned(),
                audiences: vec![AUDIENCE.to_owned()],
                algorithms: vec!["HS256".to_owned()],
                hmac_secret: Some(ferroehr::config::secret::Secret::new(
                    HMAC_SECRET.to_owned(),
                )),
                jwks_json: None,
                ..OidcConfig::default()
            }),
            ..AuthConfig::default()
        },
        // The admin group must be reachable so the RBAC gate is what decides
        // access (the admin tests assert 403 for USER vs a cleared gate for
        // ADMIN at the dispatcher, not the config gate's 404).
        admin: AdminConfig { enabled: true },
        ..Default::default()
    }
}

fn authz(enabled: bool) -> Option<Arc<AuthzHandle>> {
    let mut cfg = AuthzConfig::default();
    cfg.rbac.enabled = enabled;
    // RBAC-only: no engine, inert resolvers (nothing here consults ABAC).
    let resolvers = AuthzResolvers {
        subject: Arc::new(|_| Box::pin(async { Ok::<_, ResolveError>(None) })),
        template_of_version: Arc::new(|_, _| Box::pin(async { Ok::<_, ResolveError>(None) })),
    };
    AuthzHandle::build(&cfg, &rest_config().server.base_path, None, resolvers).map(Arc::new)
}

/// A real service over a fresh DB, optionally wired with an ATNA audit sender.
async fn service(audit: Option<AuditSender>) -> (testkit::TestDb, Arc<FerroEhrService>) {
    let (pg, pool) = common::migrated_pool().await;
    let mut svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    if let Some(sender) = audit {
        svc = svc.with_audit(sender);
    }
    (pg, Arc::new(svc))
}

async fn app(rbac_enabled: bool, audit: Option<AuditSender>) -> (testkit::TestDb, Router) {
    let (pg, svc) = service(audit).await;
    let app = ferroehr_rest::build_full(
        rest_config(),
        svc,
        authz(rbac_enabled),
        ferroehr_rest::extensions::management::Observability::default(),
    )
    .expect("build app");
    (pg, app)
}

// ── ATNA capture: a UDP listener + a sender pointed at it ──────────────────────

/// Bind a UDP listener and build an [`AuditSender`] shipping DICOM records to it
/// (fail-open, login events suppressed). The returned socket receives the RFC
/// 5424 datagrams; each carries the DICOM `AuditMessage` XML in its `MSG` field.
async fn audit_capture() -> (UdpSocket, AuditSender) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind udp");
    let port = socket.local_addr().expect("addr").port();
    let cfg = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: false,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        syslog: SyslogConfig {
            enabled: true,
            transport: Transport::Udp,
            host: "127.0.0.1".to_owned(),
            port,
            ..SyslogConfig::default()
        },
        suppress_login_events: true,
        fail_mode: FailMode::Open,
        queue_capacity: 64,
        ..AuditConfig::default()
    };
    // The drain task is detached (we drop the handle); it runs while the sender
    // — held by the service — is alive, which spans the whole test.
    let (sender, _handle) = start(cfg, None, None).await.expect("audit start");
    (socket, sender)
}

/// Receive one DICOM audit datagram (as UTF-8) within a short window.
async fn recv_audit(socket: &UdpSocket) -> Option<String> {
    let mut buf = vec![0u8; 65_536];
    match tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut buf)).await {
        Ok(Ok((n, _))) => Some(String::from_utf8_lossy(&buf[..n]).into_owned()),
        _ => None,
    }
}

// ── credentials + requests ────────────────────────────────────────────────────

/// A `Basic user:pw` header for a configured user.
fn basic(name: &str) -> String {
    format!("Basic {}", base64_encode(format!("{name}:pw").as_bytes()))
}

/// A `Bearer` HMAC token carrying the given `scope` claim.
/// A bearer token carrying `scope` and no role claim.
fn bearer(scope: &str) -> String {
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).unwrap() + 3600;
    let claims: Value =
        json!({ "sub": "svc", "iss": ISSUER, "aud": AUDIENCE, "exp": exp, "scope": scope });
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(HMAC_SECRET.as_bytes()),
    )
    .expect("encode");
    format!("Bearer {token}")
}

/// A bearer token carrying `roles` — the RFC 9068 §2.2.3.1 carrier — and no
/// scope, which is how a role actually reaches the RBAC gate.
fn bearer_with_roles(roles: &[&str]) -> String {
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).unwrap() + 3600;
    let claims: Value = json!({
        "sub": "svc",
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": exp,
        "roles": roles,
    });
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(HMAC_SECRET.as_bytes()),
    )
    .expect("encode");
    format!("Bearer {token}")
}

fn req(method: &str, path: &str, authorization: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(format!("{BASE}{path}"))
        .header("authorization", authorization)
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.9")
        .body(Body::from("{}"))
        .expect("request")
}

async fn status(app: &Router, request: Request<Body>) -> StatusCode {
    app.clone()
        .oneshot(request)
        .await
        .expect("oneshot")
        .status()
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn admin_op_forbidden_for_user_role() {
    let (_pg, app) = app(true, None).await;
    let s = status(
        &app,
        req("DELETE", &format!("/admin/ehr/{EHR_ID}"), &basic("user")),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN, "USER must not reach an admin op");
}

#[tokio::test]
async fn admin_op_passes_gate_for_admin_role() {
    let (_pg, app) = app(true, None).await;
    // ADMIN clears the RBAC gate; the concrete post-gate status is real-backend
    // behaviour — the point is the gate did NOT reject it with 403.
    let s = status(
        &app,
        req("DELETE", &format!("/admin/ehr/{EHR_ID}"), &basic("root")),
    )
    .await;
    assert_ne!(s, StatusCode::FORBIDDEN, "ADMIN must clear the gate");
}

#[tokio::test]
async fn clinical_op_allowed_for_user_role() {
    let (_pg, app) = app(true, None).await;
    let s = status(&app, req("GET", &format!("/ehr/{EHR_ID}"), &basic("user"))).await;
    assert_ne!(s, StatusCode::FORBIDDEN, "USER may reach a clinical op");
}

#[tokio::test]
async fn zero_role_principal_denied_clinical() {
    let (_pg, app) = app(true, None).await;
    let s = status(
        &app,
        req("GET", &format!("/ehr/{EHR_ID}"), &basic("noroles")),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a roleless principal is denied when RBAC is enabled"
    );
}

#[tokio::test]
async fn rbac_disabled_restores_admin_access() {
    // With RBAC disabled the handle is None → the gate is skipped; a USER reaches
    // the admin op exactly as before this feature (auth-only behaviour).
    let (_pg, app) = app(false, None).await;
    let s = status(
        &app,
        req("DELETE", &format!("/admin/ehr/{EHR_ID}"), &basic("user")),
    )
    .await;
    assert_ne!(s, StatusCode::FORBIDDEN);
}

/// A ROLE clears the admin gate; a SCOPE never does.
///
/// An OAuth2 scope grants a client delegated authority (RFC 6749 §3.3) and
/// asserts nothing about the subject's roles. Mining `scope` for roles made the
/// gate satisfiable by any caller who could name it — and made the
/// at-least-one-role check vacuous for every OIDC token, since `openid` alone
/// passed it. Roles come from the RFC 9068 §2.2.3.1 carriers instead.
#[tokio::test]
async fn a_scope_never_clears_the_admin_gate_but_a_role_does() {
    let (_pg, app) = app(true, None).await;

    // A scope naming the admin role is still only a scope.
    let by_scope = status(
        &app,
        req("DELETE", &format!("/admin/ehr/{EHR_ID}"), &bearer("ADMIN")),
    )
    .await;
    assert_eq!(
        by_scope,
        StatusCode::FORBIDDEN,
        "`scope: ADMIN` must not grant the ADMIN role",
    );

    // And a token whose only "roles" were scopes has no roles at all, so it
    // fails even the clinical at-least-one-role check.
    let clinical_by_scope = status(
        &app,
        req(
            "DELETE",
            &format!("/admin/ehr/{EHR_ID}"),
            &bearer("openid profile"),
        ),
    )
    .await;
    assert_eq!(clinical_by_scope, StatusCode::FORBIDDEN);

    // The role carrier does clear it.
    let by_role = status(
        &app,
        req(
            "DELETE",
            &format!("/admin/ehr/{EHR_ID}"),
            &bearer_with_roles(&["ADMIN"]),
        ),
    )
    .await;
    assert_ne!(
        by_role,
        StatusCode::FORBIDDEN,
        "the `roles` claim must clear the admin gate",
    );
}

/// An OAuth2 principal's committer identifier names the TOKEN ISSUER as its
/// `DV_IDENTIFIER.issuer`, not this server: the subject was minted by the
/// identity provider, and `DV_IDENTIFIER.issuer` is the "authority which issues
/// the kind of id used in the id field of this object" (RM `data_types`
/// `UML/classes/org.openehr.rm.data_types.dv_identifier.adoc` §Attributes).
/// The concrete string is spec-silent — our own design/extension; what this
/// pins is that a federated subject is not attributed to the local product.
#[tokio::test]
async fn oauth2_committer_identifier_names_the_token_issuer() {
    let (_pg, app) = app(true, None).await;

    // Create an EHR as a Bearer principal, no committal headers.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("{BASE}/ehr"))
                .header("authorization", bearer_with_roles(&["USER"]))
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let ehr_id = resp
        .headers()
        .get("ETag")
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_matches(['W', '/', '"'])
        .to_owned();

    // The EHR_STATUS's stored commit audit carries the issuer.
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("GET")
                .uri(format!(
                    "{BASE}/ehr/{ehr_id}/versioned_ehr_status/revision_history"
                ))
                .header("authorization", bearer_with_roles(&["USER"]))
                .header("accept", "application/json")
                .body(Body::empty())
                .expect("request"),
        )
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .expect("body");
    let history: Value = serde_json::from_slice(&bytes).expect("json");
    let identifier = &history["items"][0]["audits"][0]["committer"]["identifiers"][0];
    assert_eq!(
        identifier["type"], "oauth2",
        "identifier records the mechanism: {history}"
    );
    assert_eq!(
        identifier["issuer"], ISSUER,
        "a federated subject's issuer is the token issuer, not the product name"
    );
}

#[tokio::test]
async fn rbac_deny_is_audited() {
    // A 403 from the RBAC gate carries the Principal, so the ATNA audit layer
    // records a failure audit for the denied caller. The emitter ships a
    // DICOM record over syslog/UDP; we assert on the datagram: a
    // User-Authentication event (`csd-code="110114"`, DICOM PS3.15 §A.5.1 —
    // a rejected access attempt), a minor-failure outcome
    // (`EventOutcomeIndicator="4"`), attributed to `user`.
    let (socket, sender) = audit_capture().await;
    let (_pg, app) = app(true, Some(sender)).await;

    let s = status(
        &app,
        req("DELETE", &format!("/admin/ehr/{EHR_ID}"), &basic("user")),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let xml = recv_audit(&socket)
        .await
        .expect("an audit datagram is emitted for the denied caller");
    assert!(
        xml.contains(r#"csd-code="110114""#),
        "User-Authentication event: {xml}"
    );
    assert!(
        xml.contains(r#"EventOutcomeIndicator="4""#),
        "minor-failure outcome: {xml}"
    );
    assert!(
        xml.contains(r#"UserID="user""#),
        "attributed to `user`: {xml}"
    );
}

// ── GET /admin/config — the redacted effective configuration ───────────────────

/// A GET request with no `Authorization` header (the unauthenticated probe).
fn get_unauth(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(format!("{BASE}{path}"))
        .body(Body::empty())
        .expect("request")
}

/// Build the app with a populated redacted config snapshot in the observability
/// bundle, so `GET /admin/config` returns a real body. The snapshot is produced
/// by the production redaction method (`FerroEhrConfig::to_redacted_json`).
async fn app_with_config_snapshot(snapshot: Value) -> (testkit::TestDb, Router) {
    let (pg, svc) = service(None).await;
    let obs = ferroehr_rest::extensions::management::Observability {
        env_snapshot: Arc::new(snapshot),
        ..Default::default()
    };
    let app = ferroehr_rest::build_full(rest_config(), svc, authz(true), obs).expect("build app");
    (pg, app)
}

#[tokio::test]
async fn admin_config_unauthenticated_is_401() {
    let (_pg, app) = app(true, None).await;
    let s = status(&app, get_unauth("/admin/config")).await;
    assert_eq!(
        s,
        StatusCode::UNAUTHORIZED,
        "an unauthenticated caller must not read /admin/config"
    );
}

#[tokio::test]
async fn admin_config_forbidden_for_user_role() {
    let (_pg, app) = app(true, None).await;
    let s = status(&app, req("GET", "/admin/config", &basic("user"))).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a USER must not reach the admin config view"
    );
}

#[tokio::test]
async fn admin_config_admin_gets_redacted_snapshot() {
    // The production redaction method builds the snapshot from a config whose
    // DB DSN carries a credential; the admin caller receives the tree with the
    // credential masked (structural `SecretUrl` redaction), never the secret.
    let mut cfg = ferroehr::config::FerroEhrConfig::default();
    cfg.db.url = ferroehr::config::secret::SecretUrl::new(
        "postgres://dbuser:TOP_SECRET_PW@db.internal:5432/ferroehr",
    );
    let snapshot = cfg.to_redacted_json().expect("redacted json");

    let (_pg, app) = app_with_config_snapshot(snapshot).await;
    let resp = app
        .clone()
        .oneshot(req("GET", "/admin/config", &basic("root")))
        .await
        .expect("oneshot");
    assert_eq!(resp.status(), StatusCode::OK, "ADMIN clears the gate");
    let bytes = http_body_util::BodyExt::collect(resp.into_body())
        .await
        .expect("body")
        .to_bytes();
    let body = String::from_utf8_lossy(&bytes);
    assert!(
        !body.contains("TOP_SECRET_PW"),
        "the DB credential must not leak: {body}"
    );
    assert!(
        body.contains("postgres://***@db.internal:5432/ferroehr"),
        "the DSN must be present with credentials masked: {body}"
    );
}

/// Userinfo written into every URL key of the configuration reaches neither
/// `GET {base}/admin/config` nor `GET /management/env`: both serve the snapshot
/// the binary builds with `FerroEhrConfig::to_redacted_json`, and each URL keeps
/// its host and path with the userinfo masked.
#[tokio::test]
async fn no_url_userinfo_reaches_admin_config_or_management_env() {
    use ferroehr::config::management::{AccessLevel, EndpointLevels, ManagementConfig};
    use ferroehr::config::secret::SecretUrl;
    use ferroehr::service::terminology::config::{FhirProviderConfig, TerminologyOauth2Config};

    let leaky = |n: u32| SecretUrl::new(format!("https://u{n}:URL_PW_SENTINEL_{n}@h{n}/p"));
    let mut cfg = ferroehr::config::FerroEhrConfig::default();
    cfg.db.url = leaky(1);
    cfg.terminology.external.providers.insert(
        "ts".to_owned(),
        FhirProviderConfig {
            url: leaky(2),
            ..FhirProviderConfig::default()
        },
    );
    cfg.terminology.external.oauth2_clients.insert(
        "client".to_owned(),
        TerminologyOauth2Config {
            token_url: leaky(3),
            ..TerminologyOauth2Config::default()
        },
    );
    cfg.usage_report.endpoint = leaky(4);
    cfg.authz.abac.remote.server = Some(leaky(5));
    cfg.telemetry.otlp_endpoint = Some(leaky(6));
    cfg.multimedia.endpoint = Some(leaky(7));
    let snapshot = cfg.to_redacted_json().expect("redacted json");

    let (_pg, svc) = service(None).await;
    let obs = ferroehr_rest::extensions::management::Observability {
        management: ManagementConfig {
            enabled: true,
            endpoints: EndpointLevels {
                env: AccessLevel::AdminOnly,
                ..EndpointLevels::default()
            },
            ..ManagementConfig::default()
        },
        env_snapshot: Arc::new(snapshot),
        ..Default::default()
    };
    let app = ferroehr_rest::build_full(rest_config(), svc, authz(true), obs).expect("build app");
    for uri in [format!("{BASE}/admin/config"), "/management/env".to_owned()] {
        let request = Request::builder()
            .method("GET")
            .uri(&uri)
            .header("authorization", basic("root"))
            .body(Body::empty())
            .expect("request");
        let resp = app.clone().oneshot(request).await.expect("oneshot");
        assert_eq!(resp.status(), StatusCode::OK, "{uri}");
        let bytes = http_body_util::BodyExt::collect(resp.into_body())
            .await
            .expect("body")
            .to_bytes();
        let body = String::from_utf8_lossy(&bytes);
        assert!(
            !body.contains("URL_PW_SENTINEL"),
            "URL userinfo leaked from {uri}: {body}"
        );
        for n in 1..=7 {
            assert!(
                body.contains(&format!("https://***@h{n}/p")),
                "URL {n} lost its host and path in {uri}: {body}"
            );
        }
    }
}

// ── base64 helper ─────────────────────────────────────────────────────────────

fn base64_encode(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
