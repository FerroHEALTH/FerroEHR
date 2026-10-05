// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end ABAC PEP over the real axum app: the fine-grained layer through the
//! assembled router (auth → RBAC → dispatch → ABAC pre/post checks → backend)
//! over the **real** `FerroEhrService` (the scripted `Mock` is gone).
//!
//! Drives the router with `tower`'s `oneshot`, a bearer token carrying the
//! `patient_id` claim, an in-memory subject resolver, and a permit-all PDP
//! engine — so the *patient gate* is what denies. It asserts: a composition
//! create for another patient's EHR is a **pre-check 403** (and the ATNA layer
//! records the deny); a create for the caller's own EHR clears the gate; a
//! composition read for another patient's EHR is a **post-check 403**; a read of
//! the caller's own EHR is served; a missing patient claim is a 403; and
//! disabling ABAC restores today's behaviour.
//!
//! The own-patient success paths need real data, so each test seeds its EHR(s)
//! and (for reads) a committed composition through an **auth-off seed app over
//! the same service** (bypassing the gate under test), then drives the gated
//! assertions through the ABAC app.
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

use async_trait::async_trait;
use axum::Router;
use axum::body::Body;
use ferroehr::config::auth::AuthConfig;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig, SyslogConfig, Transport};
use ferroehr::system_log::sender::{AuditSender, start};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::engine::{AuthzError, PolicyEngine};
use ferroehr_rest::extensions::access::authz::request::{Attr, AuthzRequest, Decision};
use ferroehr_rest::extensions::access::authz::{AuthzHandle, AuthzResolvers, ResolveError};
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::{build_full, build_with};
use http::{Request, StatusCode};
use serde_json::{Value, json};
use tokio::net::UdpSocket;
use tower::ServiceExt;

use crate::common;
use crate::common::BASE;

const HMAC_SECRET: &str = "abac-test-secret";
const ISSUER: &str = "https://issuer.example";
/// The audience every fixture token is minted for: `audiences` is mandatory
/// whenever `[auth.oidc]` is present, so a token for another resource server
/// can never authenticate here.
const AUDIENCE: &str = "ferroehr";
// Two EHRs with distinct subjects (the resolver maps them below).
const EHR_OWN: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
const EHR_OTHER: &str = "11111111-2222-3333-4444-555555555555";
const PATIENT_OWN: &str = "PATIENT-1";
const PATIENT_OTHER: &str = "PATIENT-2";

// ── fixtures ───────────────────────────────────────────────────────────────────

/// A minimal *valid* templateless RM COMPOSITION (RM ehr `COMPOSITION`
/// mandatory attributes), so `create_composition` commits without an OPT upload.
fn composition() -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "abac test" },
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
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "conformance tester" }
    })
}

// ── a permit-all PDP engine: the patient gate is what denies here ─────────────

#[derive(Debug)]
struct PermitAll;

#[async_trait]
impl PolicyEngine for PermitAll {
    async fn decide(&self, _req: &AuthzRequest<'_>) -> Result<Decision, AuthzError> {
        Ok(Decision::Permit)
    }
}

// ── config + app assembly ─────────────────────────────────────────────────────

fn rest_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        auth: common::hs256_auth_config(ISSUER, AUDIENCE, HMAC_SECRET),
        // Lifted the way the binary lifts it, from the one setting, so this
        // follows a change to the default claim name instead of pinning a
        // second copy of it.
        audit_organization_claim: AuthzConfig::default()
            .abac
            .organization_claim()
            .map(str::to_owned),
        ..Default::default()
    }
}

/// An auth-off config used only to seed data over the shared service.
fn seed_config() -> AppConfig {
    let mut c = rest_config();
    c.auth = AuthConfig {
        enabled: false,
        ..AuthConfig::default()
    };
    c
}

/// The in-memory subject resolver: EHR → patient subject.
fn resolvers() -> AuthzResolvers {
    AuthzResolvers {
        subject: Arc::new(|ehr_id: String| {
            Box::pin(async move {
                let subject = match ehr_id.as_str() {
                    EHR_OWN => Some(PATIENT_OWN.to_owned()),
                    EHR_OTHER => Some(PATIENT_OTHER.to_owned()),
                    _ => None,
                };
                Ok::<_, ResolveError>(subject)
            })
        }),
        template_of_version: Arc::new(|_vo: String, _v: Option<String>| {
            Box::pin(async move { Ok::<_, ResolveError>(Some("org.openehr::t.v1".to_owned())) })
        }),
    }
}

/// The same subject map, with the template attribute resolved from the REAL
/// service — i.e. from the `version.template_id` the commit routes stamp.
fn service_resolvers(svc: Arc<FerroEhrService>) -> AuthzResolvers {
    AuthzResolvers {
        template_of_version: Arc::new(move |vo: String, version: Option<String>| {
            let svc = Arc::clone(&svc);
            Box::pin(async move {
                let vo_id = vo
                    .parse::<ferroehr::ids::VoId>()
                    .map_err(|e| ResolveError::new("vo id", e))?;
                svc.template_of_version(vo_id, version.as_deref())
                    .await
                    .map_err(|e| ResolveError::new("template of version", e))
            })
        }),
        ..resolvers()
    }
}

fn authz(abac_enabled: bool) -> Option<Arc<AuthzHandle>> {
    let mut cfg = AuthzConfig::default();
    cfg.abac.enabled = abac_enabled;
    let engine: Option<Arc<dyn PolicyEngine>> =
        abac_enabled.then(|| -> Arc<dyn PolicyEngine> { Arc::new(PermitAll) });
    AuthzHandle::build(&cfg, &rest_config().server.base_path, engine, resolvers()).map(Arc::new)
}

/// An ABAC handle with a caller-chosen PDP engine and resolvers.
fn authz_with(
    engine: Arc<dyn PolicyEngine>,
    resolvers: AuthzResolvers,
) -> Option<Arc<AuthzHandle>> {
    let mut cfg = AuthzConfig::default();
    cfg.abac.enabled = true;
    AuthzHandle::build(
        &cfg,
        &rest_config().server.base_path,
        Some(engine),
        resolvers,
    )
    .map(Arc::new)
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

/// The ABAC app over `svc`.
fn abac_app(svc: Arc<FerroEhrService>, abac_enabled: bool) -> Router {
    build_full(
        rest_config(),
        svc,
        authz(abac_enabled),
        Observability::default(),
    )
    .expect("build app")
}

// ── seeding through an auth-off app over the same service ───────────────────────

/// Create an EHR with a chosen id (`PUT /ehr/{id}` → 201).
async fn seed_ehr(seed: &Router, ehr_id: &str) {
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}"))
        .body(Body::empty())
        .expect("request");
    let resp = seed.clone().oneshot(req).await.expect("seed ehr");
    assert_eq!(resp.status(), StatusCode::CREATED, "seed EHR {ehr_id}");
}

/// Commit `body` into `ehr_id` through the direct composition route; return its
/// version uid (from the `ETag`, which carries `W/"<version_uid>"`).
async fn post_composition(seed: &Router, ehr_id: &str, body: String) -> String {
    let (status, headers, response) = common::send(
        seed,
        common::post_json(&format!("{BASE}/ehr/{ehr_id}/composition"), &body),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "seed composition in {ehr_id}: {response}"
    );
    common::etag_uid(&headers)
}

/// Commit the templateless fixture composition into `ehr_id`.
async fn seed_composition(seed: &Router, ehr_id: &str) -> String {
    post_composition(seed, ehr_id, composition().to_string()).await
}

// ── ATNA capture ───────────────────────────────────────────────────────────────

async fn audit_capture() -> (UdpSocket, AuditSender) {
    let socket = UdpSocket::bind("127.0.0.1:0").await.expect("bind udp");
    let port = socket.local_addr().expect("addr").port();
    let cfg = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: false,
            retention_days: 0,
            sgb_v_309_controller: false,
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
    let (sender, _handle) = start(cfg, None, None).await.expect("audit start");
    (socket, sender)
}

/// Collect DICOM audit datagrams until the window elapses.
async fn drain_audit(socket: &UdpSocket) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 65_536];
    while let Ok(Ok((n, _))) =
        tokio::time::timeout(Duration::from_millis(800), socket.recv_from(&mut buf)).await
    {
        out.push(String::from_utf8_lossy(&buf[..n]).into_owned());
    }
    out
}

// ── credentials + requests ────────────────────────────────────────────────────

/// A bearer token carrying the `USER` role and, optionally, a `patient_id`
/// claim.
///
/// The role travels in the `roles` claim — an RFC 9068 §2.2.3.1 carrier — not in
/// `scope`: an OAuth2 scope grants a client delegated authority (RFC 6749 §3.3)
/// and never becomes a role.
fn claims(patient: Option<&str>) -> Value {
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).unwrap() + 3600;
    let mut claims = json!({
        "sub": "svc",
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": exp,
        "roles": ["USER"],
    });
    if let Some(p) = patient {
        claims["patient_id"] = json!(p);
    }
    claims
}

fn bearer(patient: Option<&str>) -> String {
    common::hs256_bearer(HMAC_SECRET, &claims(patient))
}

/// The same token plus the `organization_id` claim — the claim
/// `[authz.abac] organization_claim` names by default, which is also the one
/// the audit trail records the accessing organisation from.
fn bearer_with_organisation(patient: Option<&str>, organisation: &str) -> String {
    let mut claims = claims(patient);
    claims["organization_id"] = json!(organisation);
    common::hs256_bearer(HMAC_SECRET, &claims)
}

fn request(method: &str, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(format!("{BASE}{path}"))
        .header("authorization", token)
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.9")
        .body(Body::from(composition().to_string()))
        .expect("request")
}

async fn status(app: &Router, req: Request<Body>) -> StatusCode {
    common::send_status(app, req).await
}

/// A real service plus the auth-off seed app over it — the fixture every gated
/// scenario starts from (data is seeded through the seed app, which bypasses
/// the gate under test).
async fn service_and_seed_app() -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let (pg, svc) = service(None).await;
    let seed = build_with(seed_config(), Arc::clone(&svc)).expect("seed app");
    (pg, svc, seed)
}

/// Drive one gated request as a caller whose token carries `patient`.
async fn probe(app: &Router, method: &str, path: &str, patient: Option<&str>) -> StatusCode {
    status(app, request(method, path, &bearer(patient))).await
}

/// The app whose PDP permits only requests carrying the IPS template attribute,
/// resolved from the REAL service.
fn template_scoped_app(svc: Arc<FerroEhrService>) -> Router {
    build_full(
        rest_config(),
        Arc::clone(&svc),
        authz_with(Arc::new(RequireIpsTemplate), service_resolvers(svc)),
        Observability::default(),
    )
    .expect("build app")
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_for_other_patient_is_pre_check_forbidden() {
    // The pre-check denies before dispatch (no data needed).
    let (_pg, svc) = service(None).await;
    let app = abac_app(svc, true);
    let s = probe(
        &app,
        "POST",
        &format!("/ehr/{EHR_OTHER}/composition"),
        Some(PATIENT_OWN),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "the patient gate must deny a create for another patient's EHR"
    );
}

#[tokio::test]
async fn create_for_own_patient_clears_the_gate() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;

    let app = abac_app(svc, true);
    let s = probe(
        &app,
        "POST",
        &format!("/ehr/{EHR_OWN}/composition"),
        Some(PATIENT_OWN),
    )
    .await;
    assert_ne!(
        s,
        StatusCode::FORBIDDEN,
        "own-patient create must clear the gate"
    );
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn read_of_other_patient_is_post_check_forbidden() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    // Seed EHR_OTHER + a composition so the read succeeds and the post-check can
    // evaluate it; the post-check then denies because EHR_OTHER's subject is
    // PATIENT_OTHER while the caller's claim is PATIENT_OWN.
    seed_ehr(&seed, EHR_OTHER).await;
    let uid = seed_composition(&seed, EHR_OTHER).await;

    let app = abac_app(svc, true);
    let s = probe(
        &app,
        "GET",
        &format!("/ehr/{EHR_OTHER}/composition/{uid}"),
        Some(PATIENT_OWN),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "the post-check must deny a read of another patient's composition"
    );
}

#[tokio::test]
async fn read_of_own_patient_is_served() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;
    let uid = seed_composition(&seed, EHR_OWN).await;

    let app = abac_app(svc, true);
    let s = probe(
        &app,
        "GET",
        &format!("/ehr/{EHR_OWN}/composition/{uid}"),
        Some(PATIENT_OWN),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "own-patient read is served");
}

#[tokio::test]
async fn missing_patient_claim_is_forbidden() {
    let (_pg, svc) = service(None).await;
    let app = abac_app(svc, true);
    let s = probe(&app, "POST", &format!("/ehr/{EHR_OWN}/composition"), None).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a token without the configured patient claim is denied"
    );
}

#[tokio::test]
async fn abac_disabled_restores_behaviour() {
    // ABAC off → no gate → a create for any EHR is served (auth+RBAC only).
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OTHER).await;

    let app = abac_app(svc, false);
    let s = probe(
        &app,
        "POST",
        &format!("/ehr/{EHR_OTHER}/composition"),
        Some(PATIENT_OWN),
    )
    .await;
    assert_ne!(s, StatusCode::FORBIDDEN);
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn abac_deny_is_audited() {
    // A 403 from the ABAC gate carries the Principal, so the ATNA layer records a
    // failure audit for the denied caller. The emitter ships a DICOM record
    // over syslog/UDP; we assert on the datagrams: a minor-failure outcome
    // (`EventOutcomeIndicator="4"`) attributed to `svc`.
    let (socket, sender) = audit_capture().await;
    let (_pg, svc) = service(Some(sender)).await;
    let app = abac_app(svc, true);

    let s = probe(
        &app,
        "POST",
        &format!("/ehr/{EHR_OTHER}/composition"),
        Some(PATIENT_OWN),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let records = drain_audit(&socket).await;
    assert!(
        records
            .iter()
            .any(|xml| xml.contains(r#"EventOutcomeIndicator="4""#)
                && xml.contains(r#"UserID="svc""#)),
        "expected a minor-failure audit for `svc`, got {records:?}"
    );
}

/// The accessing organisation reaches the stored access record, and only from
/// the claim the deployment configured.
///
/// EHDS Annex II 3.2(a) asks which healthcare provider an access happened on
/// behalf of (<https://eur-lex.europa.eu/eli/reg/2025/327/oj>). The middleware
/// resolves it from `[authz.abac] organization_claim` — the same claim the
/// policy layer decides on — so a caller whose token carries none leaves the
/// column NULL rather than being attributed to anyone.
#[tokio::test]
async fn the_accessing_organisation_reaches_the_stored_record() {
    let (_pg, pool) = common::migrated_pool().await;
    let cfg = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            sgb_v_309_controller: false,
        },
        suppress_login_events: true,
        fail_mode: FailMode::Open,
        queue_capacity: 64,
        ..AuditConfig::default()
    };
    let (sender, _handle) = start(cfg, None, Some(pool.clone()))
        .await
        .expect("audit start");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender),
    );
    let seed = build_with(seed_config(), Arc::clone(&svc)).expect("seed app");
    seed_ehr(&seed, EHR_OWN).await;
    let app = abac_app(Arc::clone(&svc), true);

    for token in [
        bearer_with_organisation(Some(PATIENT_OWN), "zh-noordwest"),
        bearer(Some(PATIENT_OWN)),
    ] {
        let req = Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{EHR_OWN}"))
            .header("authorization", token)
            .body(Body::empty())
            .expect("request");
        assert_eq!(status(&app, req).await, StatusCode::OK);
    }

    // The drain writes off the request path, so the assertion waits for the
    // two records rather than assuming they have landed.
    let mut recorded: Vec<Option<String>> = Vec::new();
    for _ in 0..60 {
        recorded = sqlx::query_scalar::<_, Option<String>>(
            "SELECT organisation FROM audit.audit_event WHERE principal = 'svc' \
             ORDER BY recorded_at, stored_at",
        )
        .fetch_all(&pool)
        .await
        .expect("audit rows");
        if recorded.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(
        recorded,
        vec![Some("zh-noordwest".to_owned()), None],
        "the organisation is recorded from the configured claim, and absent \
         when the token carries none"
    );
}

/// The same claim, with the ABAC gate switched off.
///
/// The claim name is an ABAC setting, but the organisation is a fact about the
/// caller rather than a decision about them: a deployment that runs no policy
/// layer still has to answer EHDS Annex II 3.2(a). Reading the claim off the
/// live gate made the whole column go blank whenever ABAC was off, silently,
/// which is what this pins.
#[tokio::test]
async fn the_accessing_organisation_is_recorded_with_the_abac_gate_off() {
    let (_pg, pool) = common::migrated_pool().await;
    let cfg = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            sgb_v_309_controller: false,
        },
        suppress_login_events: true,
        fail_mode: FailMode::Open,
        queue_capacity: 64,
        ..AuditConfig::default()
    };
    let (sender, _handle) = start(cfg, None, Some(pool.clone()))
        .await
        .expect("audit start");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender),
    );
    let seed = build_with(seed_config(), Arc::clone(&svc)).expect("seed app");
    seed_ehr(&seed, EHR_OWN).await;
    let app = abac_app(Arc::clone(&svc), false);

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{EHR_OWN}"))
        .header(
            "authorization",
            bearer_with_organisation(Some(PATIENT_OWN), "zh-noordwest"),
        )
        .body(Body::empty())
        .expect("request");
    assert_eq!(status(&app, req).await, StatusCode::OK);

    let mut recorded: Vec<Option<String>> = Vec::new();
    for _ in 0..60 {
        recorded = sqlx::query_scalar::<_, Option<String>>(
            "SELECT organisation FROM audit.audit_event WHERE principal = 'svc' \
             ORDER BY recorded_at, stored_at",
        )
        .fetch_all(&pool)
        .await
        .expect("audit rows");
        if !recorded.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    assert_eq!(
        recorded,
        vec![Some("zh-noordwest".to_owned())],
        "the organisation must be recorded whether or not the ABAC gate is on"
    );
}

// ── the template attribute binds on direct-route compositions ────────────────

/// The `template_id` the vendored IPS operational template declares.
const IPS_TEMPLATE_ID: &str = "International Patient Summary";

/// A PDP that permits ONLY when the request carries the IPS template attribute
/// — a template-scoped rule. With `version.template_id` unstamped the
/// attribute resolves to `None` and the rule silently stops binding, which is
/// exactly the defect this pins.
#[derive(Debug)]
struct RequireIpsTemplate;

#[async_trait]
impl PolicyEngine for RequireIpsTemplate {
    async fn decide(&self, req: &AuthzRequest<'_>) -> Result<Decision, AuthzError> {
        let bound = match &req.template {
            Some(Attr::One(t)) => t == IPS_TEMPLATE_ID,
            Some(Attr::Set(set)) => set.iter().any(|t| t == IPS_TEMPLATE_ID),
            None => false,
        };
        Ok(if bound {
            Decision::Permit
        } else {
            Decision::Deny
        })
    }
}

/// Upload the IPS OPT and commit one of its compositions through the DIRECT
/// composition route; return the committed version uid.
async fn seed_templated_composition(seed: &Router, ehr_id: &str) -> String {
    let upload = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header("content-type", "application/xml")
        .body(Body::from(common::ips_opt_xml()))
        .expect("request");
    let (status, body) = common::send_body(seed, upload).await;
    assert_eq!(status, StatusCode::CREATED, "IPS OPT upload: {body}");

    post_composition(
        seed,
        ehr_id,
        common::ips_canonical_composition().to_string(),
    )
    .await
}

#[tokio::test]
async fn template_scoped_rule_binds_on_a_direct_route_composition() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;
    let uid = seed_templated_composition(&seed, EHR_OWN).await;

    let app = template_scoped_app(svc);

    let s = probe(
        &app,
        "GET",
        &format!("/ehr/{EHR_OWN}/composition/{uid}"),
        Some(PATIENT_OWN),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::OK,
        "the template-scoped rule must bind: the direct-route commit stamps \
         version.template_id, so the post-check resolves the attribute"
    );
}

#[tokio::test]
async fn template_scoped_rule_does_not_bind_a_templateless_composition() {
    // The refusing twin: a composition carrying no `archetype_details.template_id`
    // genuinely has no template attribute, so the same rule denies — the
    // resolver answering `None` must mean "no template", never "unstamped".
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;
    let uid = seed_composition(&seed, EHR_OWN).await;

    let app = template_scoped_app(svc);

    let s = probe(
        &app,
        "GET",
        &format!("/ehr/{EHR_OWN}/composition/{uid}"),
        Some(PATIENT_OWN),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a templateless composition carries no template attribute, so the \
         template-scoped rule must deny"
    );
}
