// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end patient scope over the real axum app: the `[auth] patient_claim`
//! gate through the assembled router (auth → RBAC → dispatch → patient-scope
//! pre/post checks → backend) over the **real** `FerroEhrService`.
//!
//! Drives the router with `tower`'s `oneshot`, a bearer token carrying the
//! `patient_id` claim and an in-memory subject lookup. It asserts: a
//! composition create for another patient's EHR is a **pre-check 403** (and
//! the ATNA layer records the deny); a create for the caller's own EHR clears
//! the scope; a composition read for another patient's EHR is a **post-check
//! 403**; a read of the caller's own EHR is served; a missing patient claim is
//! a 403; and no configured claim leaves the scope off.
//!
//! The own-patient success paths need real data, so each test seeds its EHR(s)
//! and (for reads) a committed composition through an **auth-off seed app over
//! the same service** (bypassing the gate under test), then drives the gated
//! assertions through the scoped app. No openEHR spec governs the patient
//! scope: our own extension.
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::aql::subject::EhrSubject;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::config::rest::ApiConfig;
use ferroehr::error::ServiceError;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig, SyslogConfig};
use ferroehr::system_log::sender::{AuditSender, start};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::access::patient_scope::{PatientScope, ResolveError, SubjectFn};
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use testkit::syslog_tls::SyslogTlsListener;
use tower::ServiceExt;

use crate::common;
use crate::common::BASE;

const ISSUER: &str = "https://issuer.example";
/// The audience every fixture token is minted for: `audience` is mandatory
/// whenever `[auth]` names an issuer, so a token for another resource server
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
pub(crate) fn composition() -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "patient scope test" },
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

// ── config + app assembly ─────────────────────────────────────────────────────

/// The claim every scoped app reads the patient from.
const PATIENT_CLAIM: &str = "patient_id";

fn rest_config() -> AppConfig {
    AppConfig {
        server: common::server(),
        api: ApiConfig {
            openapi_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        auth: common::issuer_auth(ISSUER, AUDIENCE),
        ..Default::default()
    }
}

/// An auth-off config used only to seed data over the shared service.
fn seed_config() -> AppConfig {
    let mut c = rest_config();
    c.auth = common::auth_off();
    c
}

/// A subject the claim id names under one namespace.
fn unique(id: &str) -> EhrSubject {
    EhrSubject::Unique {
        id: id.to_owned(),
        namespace: Some("test.patients".to_owned()),
    }
}

/// The in-memory subject lookup: EHR → patient subject.
fn lookup() -> SubjectFn {
    Arc::new(|ehr_id: String| {
        Box::pin(async move {
            let subject = match ehr_id.as_str() {
                EHR_OWN => Some(unique(PATIENT_OWN)),
                EHR_OTHER => Some(unique(PATIENT_OTHER)),
                _ => None,
            };
            Ok::<_, ResolveError>(subject)
        })
    })
}

/// The authorization handle: RBAC by default, with the patient scope over
/// `subject` read in `namespace` when `scoped`.
fn handle(scoped: bool, namespace: Option<&str>, subject: SubjectFn) -> Option<Arc<AuthzHandle>> {
    let scope = scoped.then(|| PatientScope::new(PATIENT_CLAIM, namespace, subject));
    AuthzHandle::build(&AuthzConfig::default(), &rest_config().api.base_path, scope).map(Arc::new)
}

fn authz(scoped: bool) -> Option<Arc<AuthzHandle>> {
    handle(scoped, None, lookup())
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

/// The app over `svc`, with the patient scope when `scoped`.
async fn scoped_app(svc: Arc<FerroEhrService>, scoped: bool) -> Router {
    common::build_full(rest_config(), svc, authz(scoped), Observability::default()).await
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

async fn audit_capture() -> (SyslogTlsListener, AuditSender) {
    let socket = SyslogTlsListener::bind(0).await.expect("syslog listener");
    let port = socket.port;
    let cfg = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: false,
            retention: None,
            sgb_v_309_controller: false,
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
        },
        syslog: Some(SyslogConfig {
            url: ferrohealth_secret::secret_url::SecretUrl::new(format!("tls://127.0.0.1:{port}")),
            tls: ferrohealth_config::tls::TlsClientSection {
                trust_roots_file: Some(socket.ca_file.clone()),
                ..ferrohealth_config::tls::TlsClientSection::default()
            },
            ..SyslogConfig::default()
        }),
        suppress_login_events: true,
        fail_mode: FailMode::Open,
        queue_capacity: 64,
        ..AuditConfig::default()
    };
    let (sender, _handle) = start(cfg, None, None).await.expect("audit start");
    (socket, sender)
}

/// How long a test waits for the audit record it asserts on. A pass returns
/// the moment the record lands, so the bound only has to cover a loaded or
/// instrumented build; no openEHR spec governs it: our own design.
const RECORD_DEADLINE: Duration = Duration::from_secs(60);

/// The first DICOM audit frame (as UTF-8) that `matches`, waiting at most
/// [`RECORD_DEADLINE`]; fails naming the frames seen when none does. A
/// silence window would read a slow record as an absent one (#3950).
async fn record_matching(socket: &mut SyslogTlsListener, matches: fn(&str) -> bool) -> String {
    let deadline = tokio::time::Instant::now() + RECORD_DEADLINE;
    let mut seen: Vec<String> = Vec::new();
    loop {
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let frame = socket.recv(left).await;
        assert!(
            frame.is_some(),
            "no matching audit record within {RECORD_DEADLINE:?}; records seen: {seen:#?}"
        );
        let xml = String::from_utf8_lossy(&frame.unwrap_or_default()).into_owned();
        if matches(&xml) {
            return xml;
        }
        seen.push(xml);
    }
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
    common::bearer(&claims(patient))
}

fn request(method: &str, path: &str, token: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(common::full(path))
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
    let seed = common::build_with(seed_config(), Arc::clone(&svc)).await;
    (pg, svc, seed)
}

/// Drive one gated request as a caller whose token carries `patient`.
async fn probe(app: &Router, method: &str, path: &str, patient: Option<&str>) -> StatusCode {
    status(app, request(method, path, &bearer(patient))).await
}

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_for_other_patient_is_pre_check_forbidden() {
    // The pre-check denies before dispatch (no data needed).
    let (_pg, svc) = service(None).await;
    let app = scoped_app(svc, true).await;
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

    let app = scoped_app(svc, true).await;
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

    let app = scoped_app(svc, true).await;
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

    let app = scoped_app(svc, true).await;
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
    let app = scoped_app(svc, true).await;
    let s = probe(&app, "POST", &format!("/ehr/{EHR_OWN}/composition"), None).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a token without the configured patient claim is denied"
    );
}

#[tokio::test]
async fn no_patient_claim_leaves_the_scope_off() {
    // No patient claim configured → no scope → a create for any EHR is served (auth+RBAC only).
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OTHER).await;

    let app = scoped_app(svc, false).await;
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
async fn a_patient_scope_deny_is_audited() {
    // A 403 from the patient scope carries the Principal, so the ATNA layer records a
    // failure audit for the denied caller. The emitter ships a DICOM record
    // over syslog/TLS; we assert on the frames: a minor-failure outcome
    // (`EventOutcomeIndicator="4"`) attributed to `svc`.
    let (mut socket, sender) = audit_capture().await;
    let (_pg, svc) = service(Some(sender)).await;
    let app = scoped_app(svc, true).await;

    let s = probe(
        &app,
        "POST",
        &format!("/ehr/{EHR_OTHER}/composition"),
        Some(PATIENT_OWN),
    )
    .await;
    assert_eq!(s, StatusCode::FORBIDDEN);

    let xml = record_matching(&mut socket, |xml| {
        xml.contains(r#"UserID="svc""#) && xml.contains(r#"EventOutcomeIndicator="4""#)
    })
    .await;
    assert!(
        xml.contains(r#"UserID="svc""#),
        "attributed to `svc`: {xml}"
    );
}

// ── the query subject scope ───────────────────────────────────────────────────

/// Create an EHR with a chosen id whose `EHR_STATUS.subject` refers to
/// `patient` under `test.patients` (`PUT /ehr/{id}` with a status body → 201).
async fn seed_ehr_for(seed: &Router, ehr_id: &str, patient: &str) {
    seed_ehr_in(seed, ehr_id, "test.patients", patient).await;
}

/// Create an EHR with a chosen id whose `EHR_STATUS.subject` refers to
/// `patient` under `namespace` (`PUT /ehr/{id}` with a status body → 201).
async fn seed_ehr_in(seed: &Router, ehr_id: &str, namespace: &str, patient: &str) {
    let status = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": namespace,
                "type": "PERSON",
                "id": { "_type": "GENERIC_ID", "value": patient, "scheme": "test" }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    });
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}"))
        .header("content-type", "application/json")
        .body(Body::from(status.to_string()))
        .expect("request");
    let (code, _, body) = common::send(seed, req).await;
    assert_eq!(code, StatusCode::CREATED, "seed EHR {ehr_id}: {body}");
}

/// Run `aql` through `POST /query/aql` as a caller whose token carries
/// `patient`, returning the first column of every row.
async fn query_first_column(app: &Router, aql: &str, patient: &str) -> Vec<Value> {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/query/aql"))
        .header("authorization", bearer(Some(patient)))
        .header("content-type", "application/json")
        .body(Body::from(json!({ "q": aql }).to_string()))
        .expect("request");
    let (code, _, body) = common::send(app, req).await;
    assert_eq!(code, StatusCode::OK, "query {aql}: {body}");
    let result: Value = serde_json::from_str(&body).expect("RESULT_SET JSON");
    result["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(|row| row[0].clone())
        .collect()
}

/// The patient scope filters the HTTP query result to the caller's EHRs
/// for every query shape, including a query with no versioned-object root,
/// which reads the `ehr` row alone (our own extension; no openEHR spec governs
/// the patient scope).
#[tokio::test]
async fn query_is_filtered_to_the_callers_patient() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr_for(&seed, EHR_OWN, PATIENT_OWN).await;
    seed_ehr_for(&seed, EHR_OTHER, PATIENT_OTHER).await;
    seed_composition(&seed, EHR_OWN).await;
    seed_composition(&seed, EHR_OTHER).await;

    let gated = scoped_app(Arc::clone(&svc), true).await;
    let open = scoped_app(svc, false).await;
    for aql in [
        "SELECT e/ehr_id/value FROM EHR e",
        "SELECT e/ehr_id/value, e/ehr_status/subject FROM EHR e",
        "SELECT e/ehr_id/value FROM EHR e CONTAINS EHR_STATUS s",
        "SELECT e/ehr_id/value FROM EHR e CONTAINS COMPOSITION c",
        "SELECT e/ehr_id/value, c/name/value FROM EHR e CONTAINS COMPOSITION c LIMIT 10",
        "SELECT e/ehr_id/value FROM EHR e CONTAINS (COMPOSITION c1 OR EHR_STATUS s1)",
    ] {
        let control = query_first_column(&open, aql, PATIENT_OWN).await;
        assert!(
            control.contains(&json!(EHR_OTHER)),
            "{aql}: with no patient scope the other EHR is served: {control:?}"
        );
        let served = query_first_column(&gated, aql, PATIENT_OWN).await;
        assert!(!served.is_empty(), "{aql}: the caller's EHR is served");
        assert!(
            served.iter().all(|id| *id == json!(EHR_OWN)),
            "{aql}: only the caller's EHR is served: {served:?}"
        );
    }
    let count = query_first_column(&gated, "SELECT COUNT(*) FROM EHR e", PATIENT_OWN).await;
    assert_eq!(count, vec![json!(1)], "one EHR is in the caller's scope");
}

const EHR_SHARED_A: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const EHR_SHARED_B: &str = "aaaaaaaa-0000-4000-8000-000000000002";
const EHR_ANONYMOUS: &str = "aaaaaaaa-0000-4000-8000-000000000003";
const EHR_UNIQUE: &str = "aaaaaaaa-0000-4000-8000-000000000004";

/// A scoped app whose subject lookup reads the database through
/// `ferroehr::aql::subject::ehr_subject`, as the binary wires it, plus the
/// auth-off seed app over the same service; `patient_namespace` is the
/// `[auth] patient_namespace` setting.
async fn db_resolved_apps(patient_namespace: Option<&str>) -> (testkit::TestDb, Router, Router) {
    let (pg, pool) = common::migrated_pool().await;
    let svc = Arc::new(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&pool),
    ));
    let seed = common::build_with(seed_config(), Arc::clone(&svc)).await;
    let subject: SubjectFn = Arc::new(move |ehr_id: String| {
        let pool = pool.clone();
        Box::pin(async move {
            let id = ehr_id
                .parse::<ferroehr::ids::EhrId>()
                .map_err(|e| ResolveError::new("ehr id", e))?;
            ferroehr::aql::subject::ehr_subject(&pool, id)
                .await
                .map_err(|e| ResolveError::new("ehr subject lookup", ServiceError::from(e)))
        })
    });
    let handle = handle(true, patient_namespace, subject);
    let app = common::build_full(rest_config(), svc, handle, Observability::default()).await;
    (pg, seed, app)
}

/// The direct-route patient gate applies the rule the query patient scope
/// applies: an id held under two namespaces names neither subject, and a
/// subject-less EHR is in no patient's scope (our own extension; no openEHR
/// spec governs the patient claim).
#[tokio::test]
async fn the_direct_route_gate_refuses_ambiguous_and_subject_less_ehrs() {
    let (_pg, seed, app) = db_resolved_apps(None).await;
    seed_ehr_in(&seed, EHR_SHARED_A, "test.patients", "PATIENT-X").await;
    seed_ehr_in(&seed, EHR_SHARED_B, "other.patients", "PATIENT-X").await;
    seed_ehr(&seed, EHR_ANONYMOUS).await;
    seed_ehr_in(&seed, EHR_UNIQUE, "test.patients", "PATIENT-U").await;

    for ehr in [EHR_SHARED_A, EHR_SHARED_B] {
        assert_eq!(
            probe(
                &app,
                "POST",
                &format!("/ehr/{ehr}/composition"),
                Some("PATIENT-X")
            )
            .await,
            StatusCode::FORBIDDEN,
            "{ehr}: an id held under two namespaces names neither subject"
        );
    }
    assert_eq!(
        probe(
            &app,
            "POST",
            &format!("/ehr/{EHR_ANONYMOUS}/composition"),
            Some("PATIENT-U")
        )
        .await,
        StatusCode::FORBIDDEN,
        "a subject-less EHR is in no patient's scope"
    );
    assert_eq!(
        probe(
            &app,
            "POST",
            &format!("/ehr/{EHR_UNIQUE}/composition"),
            Some("PATIENT-U")
        )
        .await,
        StatusCode::CREATED,
        "an id held under one namespace names its subject"
    );
}

/// Without a configured namespace, a query reads none of the EHRs whose
/// subject id two namespaces hold, as the direct routes refuse the EHRs
/// themselves: the id names neither subject (our own extension; no openEHR
/// spec governs the patient claim).
#[tokio::test]
async fn a_query_over_an_id_held_under_two_namespaces_reads_nothing() {
    let (_pg, seed, app) = db_resolved_apps(None).await;
    seed_ehr_in(&seed, EHR_SHARED_A, "test.patients", "PATIENT-X").await;
    seed_ehr_in(&seed, EHR_SHARED_B, "other.patients", "PATIENT-X").await;
    seed_ehr_in(&seed, EHR_UNIQUE, "test.patients", "PATIENT-U").await;

    let served = query_first_column(&app, "SELECT e/ehr_id/value FROM EHR e", "PATIENT-X").await;
    assert_eq!(
        served,
        Vec::<Value>::new(),
        "the bare id names neither subject"
    );
    let served = query_first_column(&app, "SELECT e/ehr_id/value FROM EHR e", "PATIENT-U").await;
    assert_eq!(served, vec![json!(EHR_UNIQUE)], "a unique id reads its EHR");
}

/// A DIRECTORY operation is held to the patient scope as every other
/// EHR-scoped operation is: there is no switch that leaves it out.
#[tokio::test]
async fn a_directory_of_another_patient_is_refused() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OTHER).await;
    seed_ehr(&seed, EHR_OWN).await;
    let app = scoped_app(svc, true).await;

    let folder = json!({
        "_type": "FOLDER",
        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.generic.v1" },
            "rm_version": "1.1.0"
        },
        "name": { "_type": "DV_TEXT", "value": "root" }
    })
    .to_string();
    let directory = |ehr: &str, patient: &str| {
        Request::builder()
            .method("POST")
            .uri(common::full(&format!("/ehr/{ehr}/directory")))
            .header("authorization", bearer(Some(patient)))
            .header("content-type", "application/json")
            .body(Body::from(folder.clone()))
            .expect("request")
    };
    assert_eq!(
        status(&app, directory(EHR_OTHER, PATIENT_OWN)).await,
        StatusCode::FORBIDDEN,
        "a directory create in another patient's EHR is refused"
    );
    assert_eq!(
        status(&app, directory(EHR_OWN, PATIENT_OWN)).await,
        StatusCode::CREATED,
        "a directory create in the caller's own EHR is served"
    );
}

/// With `[auth] patient_namespace` set, the claim names the (id,
/// namespace) pair: an id held under two namespaces reaches exactly the EHR
/// under the configured one, on a direct route and through a query.
#[tokio::test]
async fn a_configured_patient_namespace_names_the_pair_exactly() {
    let (_pg, seed, app) = db_resolved_apps(Some("test.patients")).await;
    seed_ehr_in(&seed, EHR_SHARED_A, "test.patients", "PATIENT-X").await;
    seed_ehr_in(&seed, EHR_SHARED_B, "other.patients", "PATIENT-X").await;

    assert_eq!(
        probe(
            &app,
            "POST",
            &format!("/ehr/{EHR_SHARED_A}/composition"),
            Some("PATIENT-X")
        )
        .await,
        StatusCode::CREATED,
        "the subject under the configured namespace is the caller's"
    );
    assert_eq!(
        probe(
            &app,
            "POST",
            &format!("/ehr/{EHR_SHARED_B}/composition"),
            Some("PATIENT-X")
        )
        .await,
        StatusCode::FORBIDDEN,
        "the same id under another namespace is another subject"
    );
    let served = query_first_column(&app, "SELECT e/ehr_id/value FROM EHR e", "PATIENT-X").await;
    assert_eq!(served, vec![json!(EHR_SHARED_A)]);
}

// ── an item tag is checked as its target ──────────────────────────────────────

/// One request as a caller whose token carries `patient`, with a one-tag list
/// as its body (the body a tag write takes; a read ignores it).
async fn tag_probe(app: &Router, method: &str, path: &str, patient: &str) -> StatusCode {
    let req = Request::builder()
        .method(method)
        .uri(common::full(path))
        .header("authorization", bearer(Some(patient)))
        .header("content-type", "application/json")
        .body(Body::from(
            json!([{ "key": "reviewed", "value": "true" }]).to_string(),
        ))
        .expect("request");
    status(app, req).await
}

/// The `EHR_STATUS` `VERSIONED_OBJECT` id of `ehr_id`, read through `seed`.
async fn status_vo(seed: &Router, ehr_id: &str) -> String {
    let req = Request::builder()
        .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status"))
        .body(Body::empty())
        .expect("request");
    let (status, headers, body) = common::send(seed, req).await;
    assert_eq!(status, StatusCode::OK, "seed EHR_STATUS read: {body}");
    let uid = common::etag_uid(&headers);
    uid.split("::").next().expect("object id").to_owned()
}

#[tokio::test]
async fn an_item_tag_of_another_patient_is_refused_as_its_target_is() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OTHER).await;
    let uid = seed_composition(&seed, EHR_OTHER).await;
    let vo = uid.split("::").next().expect("object id").to_owned();
    let status_id = status_vo(&seed, EHR_OTHER).await;
    let app = scoped_app(svc, true).await;

    // Each tag operation beside the target operation whose patient-scope mode it takes.
    let pairs = [
        (
            "composition tag read",
            ("GET", format!("/ehr/{EHR_OTHER}/composition/{uid}")),
            ("GET", format!("/ehr/{EHR_OTHER}/composition/{vo}/tags")),
        ),
        (
            "composition tag put",
            ("PUT", format!("/ehr/{EHR_OTHER}/composition/{vo}")),
            ("PUT", format!("/ehr/{EHR_OTHER}/composition/{vo}/tags")),
        ),
        (
            "composition tag delete",
            ("PUT", format!("/ehr/{EHR_OTHER}/composition/{vo}")),
            (
                "DELETE",
                format!("/ehr/{EHR_OTHER}/composition/{vo}/tags/reviewed"),
            ),
        ),
        (
            "EHR tag read",
            ("GET", format!("/ehr/{EHR_OTHER}")),
            ("GET", format!("/ehr/{EHR_OTHER}/tags")),
        ),
        (
            "EHR_STATUS tag read",
            ("GET", format!("/ehr/{EHR_OTHER}/ehr_status")),
            (
                "GET",
                format!("/ehr/{EHR_OTHER}/ehr_status/{status_id}/tags"),
            ),
        ),
        (
            "EHR_STATUS tag put",
            ("PUT", format!("/ehr/{EHR_OTHER}/ehr_status")),
            (
                "PUT",
                format!("/ehr/{EHR_OTHER}/ehr_status/{status_id}/tags"),
            ),
        ),
        (
            "EHR_STATUS tag delete",
            ("PUT", format!("/ehr/{EHR_OTHER}/ehr_status")),
            (
                "DELETE",
                format!("/ehr/{EHR_OTHER}/ehr_status/{status_id}/tags/reviewed"),
            ),
        ),
    ];
    for (label, (target_method, target), (tag_method, tag)) in pairs {
        assert_eq!(
            tag_probe(&app, target_method, &target, PATIENT_OWN).await,
            StatusCode::FORBIDDEN,
            "the target of the {label} is refused for another patient's EHR"
        );
        assert_eq!(
            tag_probe(&app, tag_method, &tag, PATIENT_OWN).await,
            StatusCode::FORBIDDEN,
            "the {label} is refused as its target is"
        );
    }
}

#[tokio::test]
async fn an_item_tag_of_the_callers_patient_is_served() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;
    let uid = seed_composition(&seed, EHR_OWN).await;
    let vo = uid.split("::").next().expect("object id").to_owned();
    let status_id = status_vo(&seed, EHR_OWN).await;
    let app = scoped_app(svc, true).await;

    let served = [
        ("PUT", format!("/ehr/{EHR_OWN}/composition/{vo}/tags")),
        ("GET", format!("/ehr/{EHR_OWN}/composition/{vo}/tags")),
        (
            "DELETE",
            format!("/ehr/{EHR_OWN}/composition/{vo}/tags/reviewed"),
        ),
        ("GET", format!("/ehr/{EHR_OWN}/tags")),
        ("PUT", format!("/ehr/{EHR_OWN}/ehr_status/{status_id}/tags")),
        ("GET", format!("/ehr/{EHR_OWN}/ehr_status/{status_id}/tags")),
        (
            "DELETE",
            format!("/ehr/{EHR_OWN}/ehr_status/{status_id}/tags/reviewed"),
        ),
    ];
    for (method, path) in served {
        let s = tag_probe(&app, method, &path, PATIENT_OWN).await;
        assert!(
            s.is_success(),
            "a {method} of an own tag is served, got {s}"
        );
    }
}

/// A `POST /message/export` manifest naming `ehr_ids`, for a caller whose
/// token carries `patient`.
fn manifest_export(ehr_ids: &[&str], patient: &str) -> Request<Body> {
    let entities: Vec<Value> = ehr_ids
        .iter()
        .map(|id| json!({ "_type": "EXTRACT_ENTITY_MANIFEST", "extract_id_key": id, "ehr_id": id }))
        .collect();
    let spec = json!({
        "_type": "EXTRACT_SPEC",
        "extract_type": {
            "_type": "DV_CODED_TEXT", "value": "openEHR EHR",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "803"
            }
        },
        "include_multimedia": false,
        "priority": 0,
        "link_depth": 0,
        "manifest": { "_type": "EXTRACT_MANIFEST", "entities": entities }
    });
    Request::builder()
        .method("POST")
        .uri(common::full("/message/export"))
        .header("authorization", bearer(Some(patient)))
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.9")
        .body(Body::from(spec.to_string()))
        .expect("request")
}

/// Both EHR Extract export routes pass the patient gate a direct EHR read
/// passes: an export of another patient's EHR is refused with `403`, through
/// its path or through a manifest that also names the caller's own EHR.
/// Before, neither route reached the patient scope.
#[tokio::test]
async fn an_export_of_another_patients_ehr_is_forbidden() {
    let (_pg, svc) = service(None).await;
    let app = scoped_app(svc, true).await;

    assert_eq!(
        probe(
            &app,
            "GET",
            &format!("/message/export/{EHR_OTHER}"),
            Some(PATIENT_OWN)
        )
        .await,
        StatusCode::FORBIDDEN,
        "the path export of another patient's EHR is refused"
    );
    assert_eq!(
        status(&app, manifest_export(&[EHR_OWN, EHR_OTHER], PATIENT_OWN)).await,
        StatusCode::FORBIDDEN,
        "a manifest naming another patient's EHR is refused whole"
    );
}

/// An export of the caller's own EHR clears the patient gate on both routes.
#[tokio::test]
async fn an_export_of_the_own_patients_ehr_is_served() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;
    let app = scoped_app(svc, true).await;

    assert_eq!(
        probe(
            &app,
            "GET",
            &format!("/message/export/{EHR_OWN}"),
            Some(PATIENT_OWN)
        )
        .await,
        StatusCode::OK,
        "the path export of the own EHR is served"
    );
    let (code, extracts) = common::send_json(&app, manifest_export(&[EHR_OWN], PATIENT_OWN)).await;
    assert_eq!(code, StatusCode::OK, "{extracts}");
    assert_eq!(extracts.as_array().map(Vec::len), Some(1));
}

/// A `POST` of the CNF messaging corpus extract `name` to `path`, for a caller
/// whose token carries `patient`.
fn import(path: &str, name: &str, patient: Option<&str>) -> Request<Body> {
    let file = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/messaging")
        .join(name);
    let extract = std::fs::read_to_string(&file).expect("read the corpus extract");
    Request::builder()
        .method("POST")
        .uri(common::full(path))
        .header("authorization", bearer(patient))
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.9")
        .body(Body::from(extract))
        .expect("request")
}

/// Both EHR Extract import routes pass the patient-scope pre-check a direct write
/// passes: an import into another patient's EHR is refused with `403`, and so
/// is either import for a token without the configured patient claim. Before,
/// neither route reached the patient scope.
#[tokio::test]
async fn an_import_outside_the_patient_scope_is_forbidden() {
    let (_pg, svc) = service(None).await;
    let app = scoped_app(svc, true).await;

    assert_eq!(
        status(
            &app,
            import(
                &format!("/message/import/{EHR_OTHER}"),
                "ehr_extract.folder.json",
                Some(PATIENT_OWN)
            )
        )
        .await,
        StatusCode::FORBIDDEN,
        "an import into another patient's EHR is refused"
    );
    for path in [
        "/message/import".to_owned(),
        format!("/message/import/{EHR_OWN}"),
    ] {
        assert_eq!(
            status(&app, import(&path, "ehr_extract.v1.json", None)).await,
            StatusCode::FORBIDDEN,
            "{path} needs the patient claim"
        );
    }
}

/// An import into the caller's own EHR clears the gate and lands.
#[tokio::test]
async fn an_import_into_the_own_patients_ehr_lands() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;
    let app = scoped_app(svc, true).await;

    let (code, body) = common::send_body(
        &app,
        import(
            &format!("/message/import/{EHR_OWN}"),
            "ehr_extract.folder.json",
            Some(PATIENT_OWN),
        ),
    )
    .await;
    assert_eq!(code, StatusCode::NO_CONTENT, "{body}");
}
