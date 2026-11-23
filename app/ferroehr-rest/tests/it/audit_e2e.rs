// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end ATNA audit over the real axum app.
//!
//! Drives the assembled router (auth, audit, dispatch) with `tower`'s `oneshot`
//! over the real `FerroEhrService`: an [`AuditSender`] ships a DICOM
//! `AuditMessage` (XML in an RFC 5424 syslog record) over TLS to a listener the
//! test binds, and the tests assert on the frame it receives.
//!
//! DICOM codes asserted (`system_log::codes`): action `C`/`R`/`U`/`D`/`E`;
//! `EventOutcomeIndicator` `0` success / `4` minor / `8` serious;
//! `csd-code="110100"` (Application Activity / template), `"110110"`
//! (Patient Record / demographic), `"110112"` (Query), and `"110114"`
//! (User Authentication, with `EventTypeCode` `"110122"` Login).
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use axum::extract::ConnectInfo;
use ferroehr::config::rest::ApiConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, StoreConfig, SyslogConfig};
use ferroehr::system_log::sender::{AuditSender, start};
use ferroehr_rest::config::AppConfig;
use ferrohealth_server::client_address::ForwardedHeader;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use testkit::syslog_tls::SyslogTlsListener;
use tower::ServiceExt;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
// base64("alice:pw")
const BASIC_ALICE: &str = "Basic YWxpY2U6cHc=";
const CLIENT_IP: &str = "203.0.113.9";
/// The reverse proxy every request arrives through, the one `[server]
/// trusted_proxies` entry.
const PROXY: &str = "10.0.0.1";
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

fn rest_config() -> AppConfig {
    let mut config = AppConfig {
        server: ferrohealth_server::section::ServerSection {
            trusted_proxies: vec![PROXY.parse().expect("a proxy address")],
            forwarded_header: ForwardedHeader::XForwardedFor,
            ..common::server()
        },
        api: ApiConfig {
            openapi_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        ..Default::default()
    };
    common::basic_users(&mut config, USERS);
    config
}

/// The Basic user: a clinician.
const USERS: &[(&str, &[&str])] = &[("alice", &["USER"])];

fn auth_off_config() -> AppConfig {
    AppConfig {
        server: common::server(),
        api: ApiConfig {
            openapi_ui: ferroehr::config::management::AccessLevel::Off,
            ..Default::default()
        },
        auth: common::auth_off(),
        ..Default::default()
    }
}

// ── ATNA capture ───────────────────────────────────────────────────────────────

/// Bind a TLS syslog listener and an [`AuditSender`] (login suppressed by
/// default) over its own audit store, shipping DICOM records to the listener.
async fn audit_capture(suppress_login: bool) -> (testkit::TestDb, SyslogTlsListener, AuditSender) {
    let db = testkit::db().await.expect("the audit store");
    let socket = SyslogTlsListener::bind(0).await.expect("syslog listener");
    let port = socket.port;
    let cfg = AuditConfig {
        enabled: true,
        store: StoreConfig {
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            max_verify_lag: ferrohealth_config::units::Duration::from_secs(0),
            ..StoreConfig::default()
        },
        syslog: Some(SyslogConfig {
            url: ferrohealth_secret::secret_url::SecretUrl::new(format!("tls://127.0.0.1:{port}")),
            tls: ferrohealth_config::tls::TlsClientSection {
                trust_roots_file: Some(socket.ca_file.clone()),
                ..ferrohealth_config::tls::TlsClientSection::default()
            },
            ..SyslogConfig::default()
        }),
        suppress_login_events: suppress_login,
        ..AuditConfig::default()
    };
    let (sender, _handle) = start(cfg, None, db.pool()).await.expect("audit start");
    (db, socket, sender)
}

/// A sender whose store cannot be reached, so no record it is given is
/// written and the middleware answers `503`.
async fn audit_capture_unwritable() -> AuditSender {
    let unreachable = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(200))
        .connect_lazy("postgres://nobody:nothing@127.0.0.1:9/nowhere")
        .expect("a lazy pool builds without connecting");
    let cfg = AuditConfig {
        enabled: true,
        store: StoreConfig {
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            max_verify_lag: ferrohealth_config::units::Duration::from_secs(0),
            ..StoreConfig::default()
        },
        suppress_login_events: true,
        ..AuditConfig::default()
    };
    let (sender, _handle) = start(cfg, None, unreachable).await.expect("audit start");
    sender
}

/// The first frame to arrive within [`RECORD_DEADLINE`].
async fn recv_one(socket: &mut SyslogTlsListener) -> Option<String> {
    let frame = socket.recv(RECORD_DEADLINE).await?;
    Some(String::from_utf8_lossy(&frame).into_owned())
}

/// How long [`collect_until`] waits for the records a test names before it
/// fails. A pass returns the moment they land, so the bound only has to cover
/// the instrumented coverage build; no openEHR spec governs it: our own design.
const RECORD_DEADLINE: Duration = Duration::from_secs(60);

/// A record a test waits for: its name in the failure message, and the test a
/// frame must pass to count as that record.
struct Expected {
    name: &'static str,
    matches: fn(&str) -> bool,
}

/// Every frame that arrives until each of `expected` has been seen.
async fn collect_until(socket: &mut SyslogTlsListener, expected: &[Expected]) -> Vec<String> {
    let deadline = tokio::time::Instant::now() + RECORD_DEADLINE;
    let mut out: Vec<String> = Vec::new();
    loop {
        let missing: Vec<&str> = expected
            .iter()
            .filter(|want| !out.iter().any(|xml| (want.matches)(xml)))
            .map(|want| want.name)
            .collect();
        if missing.is_empty() {
            return out;
        }
        let left = deadline.saturating_duration_since(tokio::time::Instant::now());
        let Some(frame) = socket.recv(left).await else {
            panic!("no {missing:?} record within {RECORD_DEADLINE:?}; records seen: {out:#?}");
        };
        out.push(String::from_utf8_lossy(&frame).into_owned());
    }
}

/// The party id of the [`send_sentinel`] request's record.
const SENTINEL_PARTY: &str = "audit-sentinel";

/// The record of the [`send_sentinel`] request.
const SENTINEL: Expected = Expected {
    name: "sentinel demographic read",
    matches: |xml| xml.contains(r#"ParticipantObjectID="audit-sentinel""#),
};

/// Sends one more audited request after the one under test, so a test that
/// asserts an absence can wait for this request's record instead of a silence.
///
/// The sentinel's record is a record of the system chain, which the chainer
/// moves in intake order, and the syslog route ships its outbox in order over
/// one connection, so every system-chain record the earlier request wrote
/// arrives before [`SENTINEL`]. A record of the access log leaves in its own
/// step of a round, read at another moment, and can arrive after it (#5932):
/// a test that waits for those sends [`send_access_sentinel`] too.
async fn send_sentinel(app: &Router) {
    let _response = app
        .clone()
        .oneshot(req(
            "GET",
            &format!("/demographic/person/{SENTINEL_PARTY}"),
            true,
        ))
        .await
        .expect("sentinel response");
}

/// The record of the [`send_access_sentinel`] request.
const ACCESS_SENTINEL: Expected = Expected {
    name: "access-log sentinel EHR read",
    matches: |xml| xml.contains(r#"originalText="ehr_get_by_id""#),
};

/// Sends a read of the seeded [`EHR`] after the request under test. Its record
/// is an entry of the access log, which the chainer moves in intake order, so
/// every access-log record the earlier request wrote arrives before
/// [`ACCESS_SENTINEL`].
async fn send_access_sentinel(app: &Router) {
    let response = app
        .clone()
        .oneshot(req("GET", &format!("/ehr/{EHR}"), true))
        .await
        .expect("access sentinel response");
    assert_eq!(response.status(), StatusCode::OK, "the EHR read is served");
}

/// The template list's operation record.
const TEMPLATE_OP: Expected = Expected {
    name: "template operation",
    matches: |xml| xml.contains(r#"originalText="template""#),
};

// ── service assembly ───────────────────────────────────────────────────────────

/// An audited app (auth on) over a fresh DB.
async fn audit_app(sender: AuditSender) -> (testkit::TestDb, Router) {
    let (pg, pool) = common::migrated_pool().await;
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender),
    );
    common::grant_basic_users(&svc, &pool, USERS).await;
    (pg, common::build_with(rest_config(), svc).await)
}

/// An audited app over a DB pre-seeded with an EHR (`EHR`) and one committed
/// composition (returns the composition version uid). Seeding goes through a
/// **separate, unaudited** service on the same pool, so it emits no frames.
async fn audit_app_with_composition(sender: AuditSender) -> (testkit::TestDb, Router, String) {
    let (pg, pool) = common::migrated_pool().await;

    // Seed via an unaudited service (no audit frames).
    let seed_svc = Arc::new(FerroEhrService::new(
        &ferroehr::db::domain::DomainPools::from_shared(&pool),
    ));
    let seed_app = common::build_with(auth_off_config(), seed_svc).await;
    let put = common::req("PUT", format!("{BASE}/ehr/{EHR}")).build();
    assert_eq!(
        seed_app.clone().oneshot(put).await.unwrap().status(),
        StatusCode::CREATED
    );
    let post = common::req("POST", format!("{BASE}/ehr/{EHR}/composition"))
        .json(composition().to_string())
        .build();
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
    common::grant_basic_users(&svc, &pool, USERS).await;
    let app = common::build_with(rest_config(), svc).await;
    (pg, app, uid)
}

fn req(method: &str, path: &str, auth: bool) -> Request<Body> {
    from_peer(PROXY, method, path, auth)
}

/// A request over a connection from `peer` that names [`CLIENT_IP`] in
/// `X-Forwarded-For`.
fn from_peer(peer: &str, method: &str, path: &str, auth: bool) -> Request<Body> {
    let mut b = Request::builder()
        .method(method)
        .uri(common::full(path))
        .header("x-forwarded-for", CLIENT_IP)
        .header("content-type", "application/json")
        .extension(ConnectInfo(SocketAddr::new(
            peer.parse().expect("a peer address"),
            443,
        )));
    if auth {
        b = b.header("authorization", BASIC_ALICE);
    }
    b.body(Body::empty()).expect("request")
}

// ── tests ────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn ehr_create_emits_create_record() {
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    let resp = app.oneshot(req("POST", "/ehr", true)).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let xml = recv_one(&mut socket).await.expect("audit record");
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

/// A forwarded hop is believed only from a `[server] trusted_proxies` peer
/// (RFC 7239 §8.1): a peer that is no trusted proxy is the client itself,
/// whatever `X-Forwarded-For` it sends.
#[tokio::test]
async fn an_untrusted_peer_is_recorded_as_itself() {
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    let resp = app
        .oneshot(from_peer("198.51.100.7", "POST", "/ehr", true))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let xml = recv_one(&mut socket).await.expect("audit record");
    assert!(
        xml.contains(r#"NetworkAccessPointID="198.51.100.7""#),
        "the peer: {xml}"
    );
    assert!(!xml.contains(CLIENT_IP), "no forwarded hop: {xml}");
}

#[tokio::test]
async fn composition_get_emits_read_record() {
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app, uid) = audit_app_with_composition(sender).await;
    let vo = uid.split("::").next().unwrap().to_owned();
    let resp = app
        .oneshot(req("GET", &format!("/ehr/{EHR}/composition/{vo}"), true))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);
    let xml = recv_one(&mut socket).await.expect("audit record");
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
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app, uid) = audit_app_with_composition(sender).await;
    let vo = uid.split("::").next().unwrap().to_owned();
    let request = common::req("PUT", format!("{BASE}/ehr/{EHR}/composition/{vo}"))
        .header("x-forwarded-for", CLIENT_IP)
        .auth(BASIC_ALICE)
        .if_match(&uid)
        .json(composition().to_string())
        .build();
    let resp = app.oneshot(request).await.expect("resp");
    // No Prefer header ⇒ return=minimal ⇒ 204 (ITS-REST overview
    // Requests_and_responses.md §Prefer: "If no `Prefer` header is provided,
    // the default behavior is assumed to be `return=minimal`"; minimal —
    // "If no response body is returned, the service SHOULD use `204 No
    // Content`").
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let xml = recv_one(&mut socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="U""#), "update: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
}

#[tokio::test]
async fn composition_delete_emits_delete_record() {
    let (_audit, mut socket, sender) = audit_capture(true).await;
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
    let xml = recv_one(&mut socket).await.expect("audit record");
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
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app, uid) = audit_app_with_composition(sender).await;
    let vo = uid.split("::").next().unwrap().to_owned();
    let request = common::req("PUT", format!("{BASE}/ehr/{EHR}/composition/{vo}/tags"))
        .header("x-forwarded-for", CLIENT_IP)
        .auth(BASIC_ALICE)
        .json(json!([{ "key": "reviewed", "value": "true" }]).to_string())
        .build();
    let resp = app.oneshot(request).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let xml = recv_one(&mut socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="U""#), "update: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
    assert!(xml.contains(r#"UserID="alice""#), "user alice: {xml}");
}

#[tokio::test]
async fn composition_tags_delete_emits_delete_record() {
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app, uid) = audit_app_with_composition(sender).await;
    let vo = uid.split("::").next().unwrap().to_owned();
    // Seed one tag through the audited app, then drain that record.
    let put = common::req("PUT", format!("{BASE}/ehr/{EHR}/composition/{vo}/tags"))
        .auth(BASIC_ALICE)
        .json(json!([{ "key": "reviewed" }]).to_string())
        .build();
    assert_eq!(
        app.clone().oneshot(put).await.expect("resp").status(),
        StatusCode::NO_CONTENT
    );
    let _seeded = recv_one(&mut socket)
        .await
        .expect("the seeding audit record");

    let resp = app
        .oneshot(req(
            "DELETE",
            &format!("/ehr/{EHR}/composition/{vo}/tags/reviewed"),
            true,
        ))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    let xml = recv_one(&mut socket).await.expect("audit record");
    assert!(xml.contains(r#"EventActionCode="D""#), "delete: {xml}");
    assert!(
        xml.contains(r#"EventOutcomeIndicator="0""#),
        "success: {xml}"
    );
}

#[tokio::test]
async fn aql_execute_emits_execute_record() {
    let (_audit, mut socket, sender) = audit_capture(true).await;
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
    let xml = recv_one(&mut socket).await.expect("audit record");
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
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    let resp = app.oneshot(req("POST", "/ehr", false)).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // An authentication event: minor failure, no principal — the DICOM renderer
    // substitutes the configured value_if_missing ("UNKNOWN") for the empty user.
    let xml = recv_one(&mut socket).await.expect("audit record");
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
    let (_audit, mut socket, sender) = audit_capture(true).await;
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
    let xml = recv_one(&mut socket).await.expect("audit record");
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
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    // Demographic is implemented; the person does not exist on an empty DB → 404
    // → minor-failure (was a stub 501/serious-failure); R, party uid from path.
    let resp = app
        .oneshot(req("GET", "/demographic/person/p-42", true))
        .await
        .expect("resp");
    let _status = resp.status();
    let xml = recv_one(&mut socket).await.expect("audit record");
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
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    let _response = app
        .clone()
        .oneshot(req("GET", "/definition/template/adl1.4", true))
        .await
        .expect("resp");
    send_sentinel(&app).await;
    let records = collect_until(&mut socket, &[TEMPLATE_OP, SENTINEL]).await;
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
    let (_audit, mut socket, sender) = audit_capture(false).await;
    let (_pg, app) = audit_app(sender).await;
    let _response = app
        .oneshot(req("GET", "/definition/template/adl1.4", true))
        .await
        .expect("resp");
    let login = Expected {
        name: "user authentication",
        matches: |xml| xml.contains(r#"originalText="User Authentication""#),
    };
    let records = collect_until(&mut socket, &[TEMPLATE_OP, login]).await;
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

/// A response leaves only once its records are durable: right after the
/// answer, the record is in the store, with no wait for a background task
/// (the draft's semantics page, `docs/specifications/access-log`, §Recording rule 3).
#[tokio::test]
async fn a_response_leaves_after_its_record_is_stored() {
    let (audit, _socket, sender) = audit_capture(true).await;
    let (_pg, app) = audit_app(sender).await;
    let resp = app.oneshot(req("POST", "/ehr", true)).await.expect("resp");
    assert_eq!(resp.status(), StatusCode::CREATED);
    let stored: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM audit.access_log_entry) \
              + (SELECT count(*) FROM audit.audit_event)",
    )
    .fetch_one(&audit.pool())
    .await
    .expect("count the stored records");
    assert!(
        stored >= 1,
        "the EHR create's record is stored before its 201"
    );
}

/// A request whose records cannot be written is refused with `503`, never
/// served unrecorded.
#[tokio::test]
async fn a_request_whose_record_cannot_be_written_is_refused() {
    let sender = audit_capture_unwritable().await;
    let (_pg, app) = audit_app(sender).await;
    let status = drive_until_503(&app).await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn the_refusal_carries_an_openehr_error_body_and_retry_after() {
    // the 503 must emit problem details JSON body + a `Retry-After` header,
    // not a plain-text body.
    let sender = audit_capture_unwritable().await;
    let (_pg, app) = audit_app(sender).await;
    let resp = app
        .clone()
        .oneshot(req("GET", &format!("/ehr/{EHR}/composition/{EHR}"), true))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert_eq!(resp.headers().get(http::header::RETRY_AFTER).unwrap(), "1");
    assert_eq!(
        resp.headers().get(http::header::CONTENT_TYPE).unwrap(),
        "application/problem+json"
    );
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let body: Value = serde_json::from_slice(&bytes).expect("problem details");
    assert_eq!(body["status"], 503);
    assert!(body.get("detail").and_then(Value::as_str).is_some());
}

/// Drive an auditable, ehr-scoped request until the middleware refuses it
/// with 503 because its record cannot be written; the first one is.
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
    panic!("a request whose record cannot be written was served");
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
    let (_audit, mut socket, sender) = audit_capture(true).await;
    let (_pg, app, _uid) = audit_app_with_composition(sender).await;

    let resp = app
        .clone()
        .oneshot(req(
            "GET",
            "/query/aql?q=SELECT%20c/uid/value%20FROM%20EHR%20e%20CONTAINS%20COMPOSITION%20c",
            true,
        ))
        .await
        .expect("resp");
    assert_eq!(resp.status(), StatusCode::OK);

    // Each chain delivers in intake order, so once both sentinels' records
    // land, every record the query wrote, on either chain, has landed before
    // them and the count below sees them all.
    send_access_sentinel(&app).await;
    send_sentinel(&app).await;
    let records = collect_until(&mut socket, &[ACCESS_SENTINEL, SENTINEL]).await;
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

/// The origins, the origin count and the FHIR rendering of the latest
/// `composition_get` record, waiting up to five seconds for it to land.
async fn latest_composition_get_record(pool: &sqlx::PgPool) -> (Option<Value>, Option<i64>, Value) {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let found: Option<(Option<Value>, Option<i64>, String, i64)> = sqlx::query_as(
            "SELECT origins, origin_count, chain, chain_seq FROM audit.access_trail \
             WHERE operation = 'composition_get' ORDER BY recorded_at DESC LIMIT 1",
        )
        .fetch_optional(pool)
        .await
        .expect("read the trail");
        if let Some((origins, origin_count, chain, chain_seq)) = found {
            return (
                origins,
                origin_count,
                common::rendered_audit_event(pool, &chain, chain_seq).await,
            );
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no composition_get record landed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The access record of a composition read carries the origins of the data
/// it served (EHDS Annex II 3.2(e), #3212): the `FEEDER_AUDIT` originating
/// systems of the version body, with the true distinct count beside them, on
/// the stored row and as named entities of the FHIR rendering.
#[tokio::test]
async fn composition_get_records_the_origins_of_the_served_data() {
    use ferroehr::system_log::sender::AuditHandle;

    let (_pg, pool) = common::migrated_pool().await;
    let audit = common::local_audit();
    let (sender, _handle): (_, AuditHandle) = start(audit, None, pool.clone())
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender),
    );
    let app = common::build_with(auth_off_config(), svc).await;

    let put = common::req("PUT", format!("{BASE}/ehr/{EHR}")).build();
    assert_eq!(
        app.clone().oneshot(put).await.unwrap().status(),
        StatusCode::CREATED
    );
    let mut imported = composition();
    imported["feeder_audit"] = json!({
        "_type": "FEEDER_AUDIT",
        "originating_system_audit": { "_type": "FEEDER_AUDIT_DETAILS", "system_id": "lab.example" }
    });
    let post = common::req("POST", format!("{BASE}/ehr/{EHR}/composition"))
        .json(imported.to_string())
        .build();
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
    let get = common::req("GET", format!("{BASE}/ehr/{EHR}/composition/{vo}")).build();
    assert_eq!(app.oneshot(get).await.unwrap().status(), StatusCode::OK);

    let row = latest_composition_get_record(&pool).await;
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
            .filter(|e| e["name"] == "origin")
            .flat_map(|e| e["detail"].as_array().into_iter().flatten())
            .any(|d| d["type"] == "system-id" && d["valueString"] == "lab.example"),
        "the FHIR rendering names the origin: {}",
        row.2
    );
    // NOTE: our own design (#6242), a commit's record is held until the commit is confirmed.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let create: Option<Value> = loop {
        let found: Option<Option<Value>> = sqlx::query_scalar(
            "SELECT origins FROM audit.access_trail WHERE operation = 'composition_create'",
        )
        .fetch_optional(&pool)
        .await
        .expect("read the trail");
        if let Some(origins) = found {
            break origins;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no composition_create record landed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    assert_eq!(
        create, None,
        "a commit serves no version body, so it records no origin"
    );
}
