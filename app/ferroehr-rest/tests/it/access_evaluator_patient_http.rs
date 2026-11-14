// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A patient confined to their own EHR by the access evaluator, end to end
//! over the assembled router (#5586).
//!
//! A SMART App Launch patient-context credential (a `patient/` grant and no
//! `user/` or `system/` one) makes its caller the `subject` caller kind, and
//! the evaluator admits that caller to the one EHR its launch context names
//! (`ehrId`, ITS-REST `master07` §Context Selection; `master08` §Resource
//! Scopes: "Access is restricted to data within that patient's EHR"). Every
//! other EHR refuses it `403`, whether the operation names it in its path, its
//! body or its result, and a professional is held to no patient context. The
//! cases are those of the token-claim patient scope this replaces, with their
//! assertions; the evaluator's own suites (`app/ferroehr/tests/it/
//! access_eval_aql.rs`) hold the same rule with no SMART layer in front. Data
//! is seeded through an auth-off app over the same service. Confining a
//! subject caller to their own EHR is our own design.
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig, SyslogConfig};
use ferroehr::system_log::sender::{AuditSender, start};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use testkit::syslog_tls::SyslogTlsListener;
use tower::ServiceExt;

use crate::common;
use crate::common::BASE;

/// The issuer of the professionals' tokens.
const ISSUER: &str = "https://issuer.example";
/// The issuer of the person's own tokens: a `patients` issuer of its own.
const PATIENTS: &str = "https://patients.example";
/// The audience every fixture token is minted for.
const AUDIENCE: &str = "ferroehr";
/// The subject every fixture token carries.
const SUBJECT: &str = "svc";
// Two EHRs, the caller's own and another patient's.
const EHR_OWN: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
const EHR_OTHER: &str = "11111111-2222-3333-4444-555555555555";

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

/// The professionals' and the patients' issuers trusted, each one's
/// `patient/` grants counted (`[auth.smart] patient_issuers`).
fn rest_config() -> AppConfig {
    let mut config = common::api_config(false);
    config.auth = common::persons_and_patients(ISSUER, PATIENTS, AUDIENCE);
    config.auth.smart.patient_issuers = vec![
        ferrohealth_secret::secret_url::SecretUrl::new(ISSUER),
        ferrohealth_secret::secret_url::SecretUrl::new(PATIENTS),
    ];
    config
}

/// An auth-off config used only to seed data over the shared service.
fn seed_config() -> AppConfig {
    common::api_config(false)
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

/// The authenticated app over `svc`.
async fn gated_app(svc: Arc<FerroEhrService>) -> Router {
    common::build_full(rest_config(), svc, None, Observability::default()).await
}

/// A real service plus the auth-off seed app over it.
async fn service_and_seed_app() -> (testkit::TestDb, Arc<FerroEhrService>, Router) {
    let (pg, svc) = service(None).await;
    let seed = common::build_with(seed_config(), Arc::clone(&svc)).await;
    (pg, svc, seed)
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

/// The person's token: every `patient/` grant on the EHR's data and its
/// queries, in the launch context of `launch`, or of no EHR.
fn patient(launch: Option<&str>) -> String {
    let mut claims = json!({
        "sub": SUBJECT, "iss": PATIENTS, "aud": AUDIENCE,
        "exp": jiff::Timestamp::now().as_second() + 3600,
        "scope": "patient/composition-*.crud patient/aql-*.s",
    });
    if let Some(ehr) = launch {
        claims["ehrId"] = json!(ehr);
    }
    common::bearer(&claims)
}

/// A professional's token, held to no patient context.
fn professional() -> String {
    common::bearer(&json!({
        "sub": SUBJECT, "iss": ISSUER, "aud": AUDIENCE,
        "exp": jiff::Timestamp::now().as_second() + 3600,
    }))
}

/// The fixture composition as the person writes it into their own EHR, with
/// themselves as its composer (EHDS Art. 5).
fn own_composition() -> Value {
    let mut own = composition();
    own["composer"] = json!({ "_type": "PARTY_SELF" });
    own
}

fn request(method: &str, path: &str, token: &str, body: &Value) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(common::full(path))
        .header("authorization", token)
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.9")
        .body(Body::from(body.to_string()))
        .expect("request")
}

async fn status(app: &Router, req: Request<Body>) -> StatusCode {
    common::send_status(app, req).await
}

/// Drive one request as the person in the launch context of `launch`.
async fn probe(app: &Router, method: &str, path: &str, launch: Option<&str>) -> StatusCode {
    status(
        app,
        request(method, path, &patient(launch), &own_composition()),
    )
    .await
}

/// Run `aql` through `POST /query/aql` with `credential`, scoped to the EHR
/// `scope` names, returning the first column of every row.
async fn query_first_column(
    app: &Router,
    aql: &str,
    credential: &str,
    scope: Option<&str>,
) -> Vec<Value> {
    let mut req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/query/aql"))
        .header("authorization", credential)
        .header("content-type", "application/json");
    if let Some(ehr) = scope {
        req = req.header("openehr-ehr-id", ehr);
    }
    let req = req
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

// ── tests ─────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn create_for_other_patient_is_pre_check_forbidden() {
    let (_pg, svc) = service(None).await;
    let app = gated_app(svc).await;
    let s = probe(
        &app,
        "POST",
        &format!("/ehr/{EHR_OTHER}/composition"),
        Some(EHR_OWN),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "the evaluator must deny a create for another patient's EHR"
    );
}

#[tokio::test]
async fn create_for_own_patient_clears_the_gate() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;

    let app = gated_app(svc).await;
    let s = probe(
        &app,
        "POST",
        &format!("/ehr/{EHR_OWN}/composition"),
        Some(EHR_OWN),
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
    seed_ehr(&seed, EHR_OTHER).await;
    let uid = seed_composition(&seed, EHR_OTHER).await;

    let app = gated_app(svc).await;
    let s = probe(
        &app,
        "GET",
        &format!("/ehr/{EHR_OTHER}/composition/{uid}"),
        Some(EHR_OWN),
    )
    .await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "the evaluator must deny a read of another patient's composition"
    );
}

#[tokio::test]
async fn read_of_own_patient_is_served() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;
    let uid = seed_composition(&seed, EHR_OWN).await;

    let app = gated_app(svc).await;
    let s = probe(
        &app,
        "GET",
        &format!("/ehr/{EHR_OWN}/composition/{uid}"),
        Some(EHR_OWN),
    )
    .await;
    assert_eq!(s, StatusCode::OK, "own-patient read is served");
}

/// A patient context that names no EHR reaches none.
#[tokio::test]
async fn a_patient_context_naming_no_ehr_is_forbidden() {
    let (_pg, svc) = service(None).await;
    let app = gated_app(svc).await;
    let s = probe(&app, "POST", &format!("/ehr/{EHR_OWN}/composition"), None).await;
    assert_eq!(
        s,
        StatusCode::FORBIDDEN,
        "a patient context naming no EHR is denied"
    );
}

/// A professional is held to no patient context: a create for any EHR is
/// served.
#[tokio::test]
async fn a_professional_is_held_to_no_patient_context() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OTHER).await;

    let app = gated_app(svc).await;
    let s = status(
        &app,
        request(
            "POST",
            &format!("/ehr/{EHR_OTHER}/composition"),
            &professional(),
            &composition(),
        ),
    )
    .await;
    assert_ne!(s, StatusCode::FORBIDDEN);
    assert_eq!(s, StatusCode::CREATED);
}

#[tokio::test]
async fn a_patient_deny_is_audited() {
    // A 403 from the evaluator carries the Principal, so the ATNA layer records a
    // failure audit for the denied caller: a minor-failure outcome
    // (`EventOutcomeIndicator="4"`) attributed to `svc`.
    let (mut socket, sender) = audit_capture().await;
    let (_pg, svc) = service(Some(sender)).await;
    let app = gated_app(svc).await;

    let s = probe(
        &app,
        "POST",
        &format!("/ehr/{EHR_OTHER}/composition"),
        Some(EHR_OWN),
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

/// A patient's query reads only the rows of their own EHR for every query
/// shape, including one with no versioned-object root, which reads the `ehr`
/// row alone.
#[tokio::test]
async fn query_is_filtered_to_the_callers_patient() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr_in(&seed, EHR_OWN, "test.patients", "PATIENT-1").await;
    seed_ehr_in(&seed, EHR_OTHER, "test.patients", "PATIENT-2").await;
    seed_composition(&seed, EHR_OWN).await;
    seed_composition(&seed, EHR_OTHER).await;

    let app = gated_app(svc).await;
    let own = patient(Some(EHR_OWN));
    for aql in [
        "SELECT e/ehr_id/value FROM EHR e",
        "SELECT e/ehr_id/value, e/ehr_status/subject FROM EHR e",
        "SELECT e/ehr_id/value FROM EHR e CONTAINS EHR_STATUS s",
        "SELECT e/ehr_id/value FROM EHR e CONTAINS COMPOSITION c",
        "SELECT e/ehr_id/value, c/name/value FROM EHR e CONTAINS COMPOSITION c LIMIT 10",
        "SELECT e/ehr_id/value FROM EHR e CONTAINS (COMPOSITION c1 OR EHR_STATUS s1)",
    ] {
        let control = query_first_column(&app, aql, &professional(), None).await;
        assert!(
            control.contains(&json!(EHR_OTHER)),
            "{aql}: with no patient context the other EHR is served: {control:?}"
        );
        let served = query_first_column(&app, aql, &own, Some(EHR_OWN)).await;
        assert!(!served.is_empty(), "{aql}: the caller's EHR is served");
        assert!(
            served.iter().all(|id| *id == json!(EHR_OWN)),
            "{aql}: only the caller's EHR is served: {served:?}"
        );
    }
    let count = query_first_column(&app, "SELECT COUNT(*) FROM EHR e", &own, Some(EHR_OWN)).await;
    assert_eq!(count, vec![json!(1)], "one EHR is in the caller's scope");
}

const EHR_SHARED_A: &str = "aaaaaaaa-0000-4000-8000-000000000001";
const EHR_SHARED_B: &str = "aaaaaaaa-0000-4000-8000-000000000002";
const EHR_ANONYMOUS: &str = "aaaaaaaa-0000-4000-8000-000000000003";
const EHR_UNIQUE: &str = "aaaaaaaa-0000-4000-8000-000000000004";

/// A patient reaches only the EHR of their launch context: neither EHR whose
/// subject shares an id across two namespaces, nor a subject-less EHR.
#[tokio::test]
async fn the_direct_route_gate_refuses_ambiguous_and_subject_less_ehrs() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr_in(&seed, EHR_SHARED_A, "test.patients", "PATIENT-X").await;
    seed_ehr_in(&seed, EHR_SHARED_B, "other.patients", "PATIENT-X").await;
    seed_ehr(&seed, EHR_ANONYMOUS).await;
    seed_ehr_in(&seed, EHR_UNIQUE, "test.patients", "PATIENT-U").await;
    let app = gated_app(svc).await;

    for ehr in [EHR_SHARED_A, EHR_SHARED_B] {
        assert_eq!(
            probe(
                &app,
                "POST",
                &format!("/ehr/{ehr}/composition"),
                Some(EHR_UNIQUE)
            )
            .await,
            StatusCode::FORBIDDEN,
            "{ehr}: another patient's EHR"
        );
    }
    assert_eq!(
        probe(
            &app,
            "POST",
            &format!("/ehr/{EHR_ANONYMOUS}/composition"),
            Some(EHR_UNIQUE)
        )
        .await,
        StatusCode::FORBIDDEN,
        "a subject-less EHR is in no patient's context"
    );
    assert_eq!(
        probe(
            &app,
            "POST",
            &format!("/ehr/{EHR_UNIQUE}/composition"),
            Some(EHR_UNIQUE)
        )
        .await,
        StatusCode::CREATED,
        "the EHR of the launch context is the caller's"
    );
}

/// A DIRECTORY operation is held to the same rule as every other EHR-scoped
/// operation: there is no switch that leaves it out.
#[tokio::test]
async fn a_directory_of_another_patient_is_refused() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OTHER).await;
    seed_ehr(&seed, EHR_OWN).await;
    let app = gated_app(svc).await;

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
    let directory = |ehr: &str, launch: &str| {
        Request::builder()
            .method("POST")
            .uri(common::full(&format!("/ehr/{ehr}/directory")))
            .header("authorization", patient(Some(launch)))
            .header("content-type", "application/json")
            .body(Body::from(folder.clone()))
            .expect("request")
    };
    assert_eq!(
        status(&app, directory(EHR_OTHER, EHR_OWN)).await,
        StatusCode::FORBIDDEN,
        "a directory create in another patient's EHR is refused"
    );
    assert_eq!(
        status(&app, directory(EHR_OWN, EHR_OWN)).await,
        StatusCode::CREATED,
        "a directory create in the caller's own EHR is served"
    );
}

/// Two EHRs whose subjects share an id under two namespaces are two
/// patients: the person in the context of one reaches exactly that EHR, on a
/// direct route and through a query.
#[tokio::test]
async fn a_shared_subject_id_reaches_only_the_launch_ehr() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr_in(&seed, EHR_SHARED_A, "test.patients", "PATIENT-X").await;
    seed_ehr_in(&seed, EHR_SHARED_B, "other.patients", "PATIENT-X").await;
    let app = gated_app(svc).await;

    assert_eq!(
        probe(
            &app,
            "POST",
            &format!("/ehr/{EHR_SHARED_A}/composition"),
            Some(EHR_SHARED_A)
        )
        .await,
        StatusCode::CREATED,
        "the EHR of the launch context is the caller's"
    );
    assert_eq!(
        probe(
            &app,
            "POST",
            &format!("/ehr/{EHR_SHARED_B}/composition"),
            Some(EHR_SHARED_A)
        )
        .await,
        StatusCode::FORBIDDEN,
        "the same id under another namespace is another subject"
    );
    let served = query_first_column(
        &app,
        "SELECT e/ehr_id/value FROM EHR e",
        &patient(Some(EHR_SHARED_A)),
        Some(EHR_SHARED_A),
    )
    .await;
    assert_eq!(served, vec![json!(EHR_SHARED_A)]);
}

// ── an item tag is checked as its target ──────────────────────────────────────

/// One request as the person in the launch context of `launch`, with a
/// one-tag list as its body (the body a tag write takes; a read ignores it).
async fn tag_probe(app: &Router, method: &str, path: &str, launch: &str) -> StatusCode {
    let req = Request::builder()
        .method(method)
        .uri(common::full(path))
        .header("authorization", patient(Some(launch)))
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
    let app = gated_app(svc).await;

    // Each tag operation beside the target operation it is refused as.
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
            tag_probe(&app, target_method, &target, EHR_OWN).await,
            StatusCode::FORBIDDEN,
            "the target of the {label} is refused for another patient's EHR"
        );
        assert_eq!(
            tag_probe(&app, tag_method, &tag, EHR_OWN).await,
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
    let app = gated_app(svc).await;

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
        let s = tag_probe(&app, method, &path, EHR_OWN).await;
        assert!(
            s.is_success(),
            "a {method} of an own tag is served, got {s}"
        );
    }
}

/// A `POST /message/export` manifest naming `ehr_ids`, for the person in the
/// launch context of `launch`.
fn manifest_export(ehr_ids: &[&str], launch: &str) -> Request<Body> {
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
        .header("authorization", patient(Some(launch)))
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.9")
        .body(Body::from(spec.to_string()))
        .expect("request")
}

/// Both EHR Extract export routes are held as a direct EHR read is: an
/// export of another patient's EHR is refused with `403`, through its path or
/// through a manifest that also names the caller's own EHR.
#[tokio::test]
async fn an_export_of_another_patients_ehr_is_forbidden() {
    let (_pg, svc) = service(None).await;
    let app = gated_app(svc).await;

    assert_eq!(
        probe(
            &app,
            "GET",
            &format!("/message/export/{EHR_OTHER}"),
            Some(EHR_OWN)
        )
        .await,
        StatusCode::FORBIDDEN,
        "the path export of another patient's EHR is refused"
    );
    assert_eq!(
        status(&app, manifest_export(&[EHR_OWN, EHR_OTHER], EHR_OWN)).await,
        StatusCode::FORBIDDEN,
        "a manifest naming another patient's EHR is refused whole"
    );
}

/// A path export of the caller's own EHR is served. A manifest export names
/// no EHR in its path, so the SMART layer refuses it to a `patient/` grant
/// before the evaluator runs (`master07` §Context Selection); the
/// evaluator's admission of a manifest naming only the caller's own EHR is
/// `access_eval_aql::a_patient_is_refused_every_ehr_but_their_own`.
#[tokio::test]
async fn an_export_of_the_own_patients_ehr_is_served() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;
    let app = gated_app(svc).await;

    assert_eq!(
        probe(
            &app,
            "GET",
            &format!("/message/export/{EHR_OWN}"),
            Some(EHR_OWN)
        )
        .await,
        StatusCode::OK,
        "the path export of the own EHR is served"
    );
}

/// A `POST` of the CNF messaging corpus extract `name` to `path`, for the
/// person in the launch context of `launch`, or of no EHR.
fn import(path: &str, name: &str, launch: Option<&str>) -> Request<Body> {
    let file = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/fixtures/messaging")
        .join(name);
    let extract = std::fs::read_to_string(&file).expect("read the corpus extract");
    Request::builder()
        .method("POST")
        .uri(common::full(path))
        .header("authorization", patient(launch))
        .header("content-type", "application/json")
        .header("x-forwarded-for", "203.0.113.9")
        .body(Body::from(extract))
        .expect("request")
}

/// Both EHR Extract import routes are held as a direct write is: an import
/// into another patient's EHR is refused with `403`, and so is either import
/// in a patient context that names no EHR.
#[tokio::test]
async fn an_import_outside_the_patient_context_is_forbidden() {
    let (_pg, svc) = service(None).await;
    let app = gated_app(svc).await;

    assert_eq!(
        status(
            &app,
            import(
                &format!("/message/import/{EHR_OTHER}"),
                "ehr_extract.folder.json",
                Some(EHR_OWN)
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
            "{path} needs a patient context naming the EHR"
        );
    }
}

/// An import into the caller's own EHR lands.
#[tokio::test]
async fn an_import_into_the_own_patients_ehr_lands() {
    let (_pg, svc, seed) = service_and_seed_app().await;
    seed_ehr(&seed, EHR_OWN).await;
    let app = gated_app(svc).await;

    let (code, body) = common::send_body(
        &app,
        import(
            &format!("/message/import/{EHR_OWN}"),
            "ehr_extract.folder.json",
            Some(EHR_OWN),
        ),
    )
    .await;
    assert_eq!(code, StatusCode::NO_CONTENT, "{body}");
}
