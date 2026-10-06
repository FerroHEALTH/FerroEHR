// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end ATNA audit over the real axum app.
//!
//! Drives the assembled router (auth + audit + dispatch) with `tower`'s
//! `oneshot`, over the **real** `FerroEhrService` (the scripted `Mock`
//! and its in-memory `AuditSink` are gone). Auditing now runs the real ATNA path: an
//! [`AuditSender`] ships a DICOM `AuditMessage` (rendered to XML, framed as an
//! RFC 5424 syslog record) over UDP to a listener the test binds. We assert on
//! the datagram the listener receives — the action code, outcome indicator,
//! user, object class and object id the HTTP middleware produced.
//!
//! Where the old Mock scripted a canned success (a `501` stub or a fixed uid),
//! the assertion is re-targeted to the **real** behaviour for that scenario:
//! - the ad-hoc query group is implemented, so an AQL execute succeeds (`0`,
//!   was a `501`/serious-failure under the stub);
//! - the demographic person-get on an empty DB is a `404` minor-failure (`4`,
//!   was a `501`/serious-failure under the stub);
//! - the composition read/update/delete object ids are the real committed
//!   version uids (seeded through the service before the audited request).
//!
//! DICOM codes asserted (`system_log::codes`): action `C`/`R`/`U`/`D`/`E`;
//! `EventOutcomeIndicator` `0` success / `4` minor / `8` serious;
//! `csd-code="110100"` (Application Activity / template), `"110110"`
//! (Patient Record / demographic), `"110112"` (Query), and `"110114"`
//! (User Authentication, with `EventTypeCode` `"110122"` Login).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    clippy::missing_assert_message,
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
use ferroehr::config::auth::{AuthConfig, BasicConfig, BasicUser};
use ferroehr::config::server::ServerConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig, SyslogConfig, Transport};
use ferroehr::system_log::sender::{AuditSender, SubjectResolver, start};
use ferroehr_rest::config::AppConfig;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tokio::net::UdpSocket;
use tower::ServiceExt;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
// base64("alice:pw")
const BASIC_ALICE: &str = "Basic YWxpY2U6cHc=";
const CLIENT_IP: &str = "203.0.113.9";
const EHR: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

// ── fixtures ───────────────────────────────────────────────────────────────────

/// A minimal *valid* templateless RM COMPOSITION (commits without an OPT).
fn composition() -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "audit test" },
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

// ── auth config ────────────────────────────────────────────────────────────────

fn hash_pw(pw: &str) -> String {
    Argon2::default()
        .hash_password_with_salt(pw.as_bytes(), b"1234567890123456")
        .expect("hash")
        .to_string()
}

fn auth_on() -> AuthConfig {
    AuthConfig {
        enabled: true,
        basic: Some(BasicConfig {
            users: vec![BasicUser {
                username: "alice".to_owned(),
                password_hash: ferroehr::config::secret::Secret::new(hash_pw("pw")),
                password_hash_file: None,
                roles: vec!["USER".to_owned()],
            }],
        }),
        oidc: None,
        ..AuthConfig::default()
    }
}

fn rest_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        auth: auth_on(),
        ..Default::default()
    }
}

fn auth_off_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            swagger_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        auth: AuthConfig {
            enabled: false,
            ..AuthConfig::default()
        },
        ..Default::default()
    }
}

// ── ATNA capture ───────────────────────────────────────────────────────────────

/// Bind a UDP listener and an [`AuditSender`] (fail-open, login suppressed by
/// default) shipping DICOM records to it.
async fn audit_capture(suppress_login: bool) -> (UdpSocket, AuditSender) {
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
        suppress_login_events: suppress_login,
        fail_mode: FailMode::Open,
        queue_capacity: 64,
        ..AuditConfig::default()
    };
    let (sender, _handle) = start(cfg, None, None).await.expect("audit start");
    (socket, sender)
}

/// A fail-closed sender whose drain is deliberately stalled (a blocking subject
/// resolver + a 1-slot queue), so once the queue fills every further emit is
/// `Rejected` → the middleware returns `503`.
async fn audit_capture_fail_closed() -> AuditSender {
    // A free port for the sender's connected UDP transport. The drain is stalled
    // (see below) so it never actually sends — no listener is required.
    let port = {
        let s = UdpSocket::bind("127.0.0.1:0").await.expect("bind udp");
        s.local_addr().expect("addr").port()
    };
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
        fail_mode: FailMode::Closed,
        resolve_subject: true,
        queue_capacity: 1,
        ..AuditConfig::default()
    };
    // The resolver blocks the drain forever after the first record it dequeues,
    // so the 1-slot queue saturates and stays saturated.
    let resolver: SubjectResolver = Arc::new(|_ehr_id: String| {
        Box::pin(async move {
            // Never resolves → the drain parks here forever, so the 1-slot queue
            // saturates and every further emit is rejected.
            std::future::pending::<()>().await;
            None
        })
    });
    let (sender, _handle) = start(cfg, Some(resolver), None).await.expect("audit start");
    sender
}

async fn recv_one(socket: &UdpSocket) -> Option<String> {
    let mut buf = vec![0u8; 65_536];
    match tokio::time::timeout(Duration::from_secs(3), socket.recv_from(&mut buf)).await {
        Ok(Ok((n, _))) => Some(String::from_utf8_lossy(&buf[..n]).into_owned()),
        _ => None,
    }
}

/// Drain every DICOM datagram that arrives within the window.
async fn drain(socket: &UdpSocket) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = vec![0u8; 65_536];
    while let Ok(Ok((n, _))) =
        tokio::time::timeout(Duration::from_millis(700), socket.recv_from(&mut buf)).await
    {
        out.push(String::from_utf8_lossy(&buf[..n]).into_owned());
    }
    out
}

// ── service assembly ───────────────────────────────────────────────────────────

/// An audited app (auth on) over a fresh DB.
async fn audit_app(sender: AuditSender) -> (testkit::TestDb, Router) {
    let (pg, pool) = common::migrated_pool().await;
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender),
    );
    (
        pg,
        ferroehr_rest::build_with(rest_config(), svc).expect("build app"),
    )
}

/// An audited app over a DB pre-seeded with an EHR (`EHR`) and one committed
/// composition (returns the composition version uid). Seeding goes through a
/// **separate, unaudited** service on the same pool, so it emits no datagrams.
async fn audit_app_with_composition(sender: AuditSender) -> (testkit::TestDb, Router, String) {
    let (pg, pool) = common::migrated_pool().await;

    // Seed via an unaudited service (no audit datagrams).
    let seed_svc = Arc::new(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&pool),
    ));
    let seed_app = ferroehr_rest::build_with(auth_off_config(), seed_svc).expect("seed app");
    let put = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{EHR}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        seed_app.clone().oneshot(put).await.unwrap().status(),
        StatusCode::CREATED
    );
    let post = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header("content-type", "application/json")
        .body(Body::from(composition().to_string()))
        .unwrap();
    let resp = seed_app.oneshot(post).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED, "seed composition");
    let uid = resp
        .headers()
        .get(http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned();

    // The audited app over the same pool.
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender),
    );
    let app = ferroehr_rest::build_with(rest_config(), svc).expect("build app");
    (pg, app, uid)
}

fn req(method: &str, path: &str, auth: bool) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(format!("{BASE}{path}"))
        .header("x-forwarded-for", CLIENT_IP)
        .header("content-type", "application/json");
    if auth {
        b = b.header("authorization", BASIC_ALICE);
    }
    b.body(Body::empty()).expect("request")
}

// ── tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ehr_create_emits_create_record() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    let resp = app.oneshot(req("POST", "/ehr", true)).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let xml = recv_one(&socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="C""#), "create: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
    assert!(xml.contains(r#"UserID="alice""#), "user alice: {xml}");
    assert!(
        xml.contains(&format!(r#"NetworkAccessPointID="{CLIENT_IP}""#)),
        "client ip: {xml}"
    );
    assert!(
        xml.contains(r#"originalText="Patient Record""#),
        "EHR object: {xml}"
    );
}

#[tokio::test]
async fn composition_get_emits_read_record() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app, uid) = audit_app_with_composition(sender).await;
    let vo = uid.split("::").next().unwrap().to_owned();
    let resp = app
        .oneshot(req("GET", &format!("/ehr/{EHR}/composition/{vo}"), true))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let xml = recv_one(&socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="R""#), "read: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
    // The object id carries the real committed version uid (was Mock-canned).
    assert!(
        xml.contains(&format!(r#"ParticipantObjectID="{uid}""#)),
        "object id = committed uid {uid}: {xml}"
    );
}

#[tokio::test]
async fn composition_update_emits_update_record() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app, uid) = audit_app_with_composition(sender).await;
    let vo = uid.split("::").next().unwrap().to_owned();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{EHR}/composition/{vo}"))
        .header("x-forwarded-for", CLIENT_IP)
        .header("content-type", "application/json")
        .header("authorization", BASIC_ALICE)
        .header("if-match", &uid)
        .body(Body::from(composition().to_string()))
        .expect("request");
    let resp = app.oneshot(request).await.expect("resp");
    // No Prefer header ⇒ return=minimal ⇒ 204 (ITS-REST overview
    // Requests_and_responses.md §Prefer: "If no `Prefer` header is provided,
    // the default behavior is assumed to be `return=minimal`"; minimal —
    // "If no response body is returned, the service SHOULD use `204 No
    // Content`").
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let xml = recv_one(&socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="U""#), "update: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
}

#[tokio::test]
async fn composition_delete_emits_delete_record() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app, uid) = audit_app_with_composition(sender).await;
    let resp = app
        .oneshot(req(
            "DELETE",
            &format!("/ehr/{EHR}/composition/{uid}"),
            true,
        ))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let xml = recv_one(&socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="D""#), "delete: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
}

#[tokio::test]
async fn composition_tags_update_emits_update_record() {
    // A tag mutation IS an audited state change. The openEHR specs are silent
    // on any audit obligation for tags — RM ehr `master04-ehr_package.adoc`
    // §Tags puts them outside change control ("they do not cause re-versioning
    // of the content"), so no CONTRIBUTION and no AUDIT_DETAILS is
    // spec-correct. Emitting an IHE ATNA record instead is OUR OWN DESIGN (no
    // openEHR spec governs this): a tag carries clinical meaning and a mutation
    // with no trail would be a medico-legal blind spot.
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app, uid) = audit_app_with_composition(sender).await;
    let vo = uid.split("::").next().unwrap().to_owned();
    let request = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{EHR}/composition/{vo}/tags"))
        .header("x-forwarded-for", CLIENT_IP)
        .header("content-type", "application/json")
        .header("authorization", BASIC_ALICE)
        .body(Body::from(
            json!([{ "key": "reviewed", "value": "true" }]).to_string(),
        ))
        .expect("request");
    let resp = app.oneshot(request).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let xml = recv_one(&socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="U""#), "update: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
    assert!(xml.contains(r#"UserID="alice""#), "user alice: {xml}");
}

#[tokio::test]
async fn composition_tags_delete_emits_delete_record() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app, uid) = audit_app_with_composition(sender).await;
    let vo = uid.split("::").next().unwrap().to_owned();
    // Seed one tag through the audited app, then drain that record.
    let put = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{EHR}/composition/{vo}/tags"))
        .header("content-type", "application/json")
        .header("authorization", BASIC_ALICE)
        .body(Body::from(json!([{ "key": "reviewed" }]).to_string()))
        .expect("request");
    assert_eq!(
        app.clone().oneshot(put).await.expect("resp").status(),
        StatusCode::NO_CONTENT
    );
    let _seeded = recv_one(&socket).await.expect("the seeding audit record");

    let resp = app
        .oneshot(req(
            "DELETE",
            &format!("/ehr/{EHR}/composition/{vo}/tags/reviewed"),
            true,
        ))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let xml = recv_one(&socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="D""#), "delete: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
}

#[tokio::test]
async fn aql_execute_emits_execute_record() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    // A well-formed ad-hoc query reaches the (implemented) QueryService and
    // succeeds on an empty DB → outcome success (was a stub 501/serious-failure).
    let resp = app
        .oneshot(req(
            "GET",
            "/query/aql?q=SELECT%20c%20FROM%20COMPOSITION%20c",
            true,
        ))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let xml = recv_one(&socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="E""#), "execute: {xml}");
    // Query execution uses the dedicated DICOM EventID 110112 "Query"
    // (DICOM PS3.15 §A.5.1).
    assert!(
        xml.contains(r#"csd-code="110112""#) && xml.contains(r#"originalText="Query""#),
        "query event id: {xml}"
    );
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
}

#[tokio::test]
async fn unauthenticated_request_emits_401_record() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    let resp = app.oneshot(req("POST", "/ehr", false)).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // An authentication event: minor failure, no principal — the DICOM renderer
    // substitutes the configured value_if_missing ("UNKNOWN") for the empty user.
    let xml = recv_one(&socket).await.expect("audit record");
    // A rejected access attempt is a failed user-authentication event: the
    // dedicated DICOM EventID 110114 "User Authentication" with EventTypeCode
    // 110122 "Login" (DICOM PS3.15 §A.5.1).
    assert!(
        xml.contains(r#"csd-code="110114""#),
        "user authentication: {xml}"
    );
    assert!(xml.contains(r#"csd-code="110122""#), "login type: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="4""#),
        "minor failure: {xml}"
    );
    assert!(xml.contains(r#"UserID="UNKNOWN""#), "no principal: {xml}");
}

#[tokio::test]
async fn template_get_emits_template_record() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    // The template is not uploaded → 404, but the op is still audited as the
    // Template class (R); the template id is derived from the path.
    let resp = app
        .oneshot(req(
            "GET",
            "/definition/template/adl1.4/vital_signs.v1",
            true,
        ))
        .await
        .expect("resp");
    let _status = resp.status();
    let xml = recv_one(&socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="R""#), "read: {xml}");
    assert!(
        xml.contains(r#"originalText="template""#),
        "template object: {xml}"
    );
    assert!(
        xml.contains(r#"ParticipantObjectID="vital_signs.v1""#),
        "template id: {xml}"
    );
}

#[tokio::test]
async fn demographic_get_emits_demographic_record() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    // Demographic is implemented; the person does not exist on an empty DB → 404
    // → minor-failure (was a stub 501/serious-failure); R, party uid from path.
    let resp = app
        .oneshot(req("GET", "/demographic/person/p-42", true))
        .await
        .expect("resp");
    let _status = resp.status();
    let xml = recv_one(&socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="R""#), "read: {xml}");
    assert!(
        xml.contains(r#"originalText="demographic""#),
        "demographic object: {xml}"
    );
    assert!(
        xml.contains(r#"ParticipantObjectID="p-42""#),
        "party id: {xml}"
    );
}

#[tokio::test]
async fn login_event_is_suppressed_by_default() {
    // suppress_login=true: an audited operation emits the operation record and no
    // login/application-activity record.
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    let _response = app
        .oneshot(req("GET", "/definition/template/adl1.4", true))
        .await
        .expect("resp");
    let records = drain(&socket).await;
    assert!(
        records
            .iter()
            .any(|x| x.contains(r#"originalText="template""#)),
        "the op record is emitted: {records:?}"
    );
    assert!(
        !records
            .iter()
            .any(|x| x.contains(r#"originalText="User Authentication""#)),
        "suppressed login must emit no user-authentication record: {records:?}"
    );
}

#[tokio::test]
async fn login_event_emitted_when_not_suppressed() {
    // Not suppressed: both the operation record and a login (User
    // Authentication) record are emitted for the fresh Basic authentication.
    let (socket, sender) = audit_capture(false).await;
    let (_pg, app) = audit_app(sender).await;
    let _response = app
        .oneshot(req("GET", "/definition/template/adl1.4", true))
        .await
        .expect("resp");
    let records = drain(&socket).await;
    assert!(
        records
            .iter()
            .any(|x| x.contains(r#"originalText="template""#)),
        "op record present: {records:?}"
    );
    assert!(
        records
            .iter()
            .any(|x| x.contains(r#"originalText="User Authentication""#)
                && x.contains(r#"UserID="alice""#)
                && x.contains(r#"EventOutcomeIndicator="0""#)),
        "a success login record for alice is present: {records:?}"
    );
}

#[tokio::test]
async fn fail_open_serves_request_when_channel_full() {
    // Fail-open: even if the audit emit is dropped, the request still succeeds.
    let (_socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    let resp = app.oneshot(req("POST", "/ehr", true)).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn fail_closed_returns_503_when_channel_full() {
    // Fail-closed: once the (stalled, 1-slot) audit queue saturates, an auditable
    // operation whose record is rejected makes the middleware return 503.
    let sender = audit_capture_fail_closed().await;
    let (_pg, app) = audit_app(sender).await;
    let status = drive_until_503(&app).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn fail_closed_503_carries_openehr_error_body_and_retry_after() {
    // the fail-closed 503 must emit the standard openEHR `{ error, message }`
    // JSON body + a `Retry-After` header, not a plain-text body.
    let sender = audit_capture_fail_closed().await;
    let (_pg, app) = audit_app(sender).await;
    // Saturate, then capture the shed 503 response.
    let resp = loop {
        let r = app
            .clone()
            .oneshot(req("GET", &format!("/ehr/{EHR}/composition/{EHR}"), true))
            .await
            .expect("resp");
        if r.status() == StatusCode::SERVICE_UNAVAILABLE {
            break r;
        }
    };
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(resp.headers().get(http::header::RETRY_AFTER).unwrap(), "1");
    assert_eq!(
        resp.headers().get(http::header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("json error body");
    assert_eq!(body["error"], "Service Unavailable");
    assert!(body.get("message").and_then(Value::as_str).is_some());
}

/// Drive an auditable, ehr-scoped request repeatedly until the fail-closed
/// middleware sheds it with 503 (the stalled queue saturates within a few emits).
async fn drive_until_503(app: &Router) -> StatusCode {
    for _ in 0..50 {
        let status = app
            .clone()
            .oneshot(req("GET", &format!("/ehr/{EHR}/composition/{EHR}"), true))
            .await
            .expect("resp")
            .status();
        if status == StatusCode::SERVICE_UNAVAILABLE {
            return status;
        }
    }
    panic!("fail-closed queue never saturated to 503");
}

/// A query that serves a record emits an access record naming the EHR it
/// served, beside the statement's own execute record (#3156).
///
/// The statement record answers "who ran what". NEN 7513 and EHDS Art. 9 ask a
/// different question — whose record was read — and one execute record over a
/// population query cannot answer it. The served EHR is read off the rows that
/// were actually returned, so the trail names a disclosure that happened.
#[tokio::test]
async fn aql_execute_emits_one_access_record_per_served_ehr() {
    let (socket, sender) = audit_capture(true).await;
    let (_pg, app, _uid) = audit_app_with_composition(sender).await;

    let resp = app
        .oneshot(req(
            "GET",
            "/query/aql?q=SELECT%20c/uid/value%20FROM%20EHR%20e%20CONTAINS%20COMPOSITION%20c",
            true,
        ))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    let records = drain(&socket).await;
    assert!(
        records
            .iter()
            .any(|xml| xml.contains(r#"csd-code="110112""#)
                && xml.contains(r#"EventActionCode="E""#)),
        "the statement's own Query execute record is missing: {records:#?}"
    );
    // The access record: a Patient-Record read (DICOM EventID 110110) carrying
    // the query as its EventTypeCode, which is what distinguishes it from a
    // direct retrieval. Its participant is the EHR's subject, absent on this
    // fixture; the EHR the record names is on the stored row, which
    // `audit_store::access_fields_round_trip_on_both_write_paths` pins.
    let served: Vec<&String> = records
        .iter()
        .filter(|xml| {
            xml.contains(r#"csd-code="110110""#)
                && xml.contains(r#"EventActionCode="R""#)
                && xml.contains("query_execute_adhoc_query")
        })
        .collect();
    assert_eq!(
        served.len(),
        1,
        "one access record per served EHR, and this query served one: {records:#?}"
    );
}

/// The access record of a composition read carries the origins of the data
/// it served (EHDS Annex II 3.2(e), #3212): the `FEEDER_AUDIT` originating
/// systems of the version body, with the true distinct count beside them, on
/// the stored row and as named entities of the FHIR rendering.
#[tokio::test]
async fn composition_get_records_the_origins_of_the_served_data() {
    use ferroehr::system_log::sender::AuditHandle;

    let (_pg, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender),
    );
    let app = ferroehr_rest::build_with(auth_off_config(), svc).expect("build app");

    let put = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{EHR}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(
        app.clone().oneshot(put).await.unwrap().status(),
        StatusCode::CREATED
    );
    let mut imported = composition();
    imported["feeder_audit"] = json!({
        "_type": "FEEDER_AUDIT",
        "originating_system_audit": { "_type": "FEEDER_AUDIT_DETAILS", "system_id": "lab.example" }
    });
    let post = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header("content-type", "application/json")
        .body(Body::from(imported.to_string()))
        .unwrap();
    let resp = app.clone().oneshot(post).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let uid = resp
        .headers()
        .get(http::header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned();
    let vo = uid.split("::").next().unwrap().to_owned();
    let get = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{EHR}/composition/{vo}"))
        .body(Body::empty())
        .unwrap();
    assert_eq!(app.oneshot(get).await.unwrap().status(), StatusCode::OK);

    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let row = loop {
        let found: Option<(Option<Value>, Option<i64>, Value)> = sqlx::query_as(
            "SELECT origins, origin_count, fhir FROM audit.audit_event \
             WHERE operation = 'composition_get' ORDER BY recorded_at DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("read the trail");
        if let Some(row) = found {
            break row;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no composition_get record landed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        row.0,
        Some(json!(["lab.example"])),
        "the served origin is recorded"
    );
    assert_eq!(row.1, Some(1));
    let entities = row.2["entity"].as_array().expect("entities");
    assert!(
        entities
            .iter()
            .any(|e| e["what"]["identifier"]["value"] == "lab.example"),
        "the FHIR rendering names the origin: {}",
        row.2
    );
    let create: Option<Value> = sqlx::query_scalar(
        "SELECT origins FROM audit.audit_event WHERE operation = 'composition_create'",
    )
    .fetch_one(&pool)
    .await
    .expect("the create record");
    assert_eq!(
        create, None,
        "a commit serves no version body, so it records no origin"
    );
}
