// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests: routing, authentication (401/403), and content
//! negotiation, exercised through the assembled router over a **real**
//! `FerroEhrService` on a real `PostgreSQL` (the `common` fixture).
//!
//! All API groups are mounted; these tests exercise routing, auth, and
//! negotiation across representative operations. The former `Mock` backend's
//! blanket `501 Not Implemented` default is gone — every group is now a real
//! implementation, so the old "reaches the stub → 501" probes are re-targeted
//! to the real server's behaviour for a fresh, empty database (a missing EHR is
//! a `404`, an invalid AQL a `400`, an empty template list a `200 []`).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use argon2::Argon2;
use argon2::password_hash::PasswordHasher;
use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};
use serde_json::json;
use tower::ServiceExt;

use ferroehr::config::auth::{AuthConfig, BasicConfig, BasicUser, OidcConfig};
use ferroehr::config::server::{AdminConfig, BodyLimits, RateLimitConfig, ServerConfig};
use ferroehr_rest::config::AppConfig;

use crate::common;

const ISSUER: &str = "https://issuer.test";
/// The audience every fixture token is minted for: `audiences` is mandatory
/// whenever `[auth.oidc]` is present, so a token for another resource server
/// can never authenticate here.
const AUDIENCE: &str = "ferroehr";
const SECRET: &str = "integration-secret";
const BASE: &str = "/ferroehr/rest/openehr/v1";
/// A syntactically valid EHR id that does not exist: the EHR dispatcher decodes
/// `ehr_id` into a `Uuid` and consults the backend, so a "reaches the handler"
/// probe needs a real UUID (an invalid one is a 400 at the adapter). A fresh
/// database has no such EHR → the real handler answers `404`.
const EHR: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

fn argon2_hash(pw: &str) -> String {
    Argon2::default()
        .hash_password_with_salt(pw.as_bytes(), b"1234567890123456")
        .unwrap()
        .to_string()
}

fn config(enabled: bool) -> AppConfig {
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
            enabled,
            basic: Some(BasicConfig {
                users: vec![BasicUser {
                    username: "alice".to_owned(),
                    password_hash: ferroehr::config::secret::Secret::new(argon2_hash("pw")),
                    password_hash_file: None,
                    roles: vec!["USER".to_owned()],
                }],
            }),
            oidc: Some(OidcConfig {
                issuer: ISSUER.to_owned(),
                audiences: vec![AUDIENCE.to_owned()],
                algorithms: vec!["HS256".to_owned()],
                hmac_secret: Some(ferroehr::config::secret::Secret::new(SECRET.to_owned())),
                jwks_json: None,
                ..OidcConfig::default()
            }),
            ..AuthConfig::default()
        },
        // The admin group must be reachable here: `admin_route_reachable_without_rbac`
        // exercises the real admin delete (not the config gate's 404).
        admin: AdminConfig { enabled: true },
        ..Default::default()
    }
}

/// Build the router over a fresh real service (unique database per test).
async fn app(enabled: bool) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(config(enabled), service))
}

/// The current POSIX second — the unit JWT `exp`/`iat` claims use (RFC 7519
/// §2 `NumericDate`). Wall-clock time comes from `jiff`, the pinned time
/// library.
fn now() -> i64 {
    jiff::Timestamp::now().as_second()
}

fn bearer(scope: &str) -> String {
    let claims = serde_json::json!({
        "sub": "alice",
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": now() + 3600,
        "scope": scope,
    });
    let token = encode(
        &Header::new(Algorithm::HS256),
        &claims,
        &EncodingKey::from_secret(SECRET.as_bytes()),
    )
    .unwrap();
    format!("Bearer {token}")
}

fn basic(user: &str, pw: &str) -> String {
    format!("Basic {}", b64(format!("{user}:{pw}").as_bytes()))
}

/// Minimal standard base64 encoder (avoids adding a base64 dev-dep).
fn b64(bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    // The 6-bit group is masked to 0..=63, so both conversions are lossless
    // (`u8::try_from` cannot fail, `usize::from(u8)` is total).
    let sextet = |n: u32, shift: u32| -> char {
        let group = u8::try_from((n >> shift) & 63).expect("a 6-bit group should fit u8");
        char::from(T[usize::from(group)])
    };
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let b = [
            chunk[0],
            *chunk.get(1).unwrap_or(&0),
            *chunk.get(2).unwrap_or(&0),
        ];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(sextet(n, 18));
        out.push(sextet(n, 12));
        out.push(if chunk.len() > 1 { sextet(n, 6) } else { '=' });
        out.push(if chunk.len() > 2 { sextet(n, 0) } else { '=' });
    }
    out
}

async fn send(app: Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, String) {
    let resp = app.oneshot(req).await.expect("response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn get(path: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(path)
        .body(Body::empty())
        .unwrap()
}

#[tokio::test]
async fn unauthenticated_request_is_401_with_challenge() {
    let (_pg, app) = app(true).await;
    let (status, headers, body) = send(app, get(&format!("{BASE}/ehr/abc"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    assert!(headers.contains_key(header::WWW_AUTHENTICATE));
    // JSON error body.
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"], "Unauthorized");
}

#[tokio::test]
async fn status_endpoint_is_public() {
    let (_pg, app) = app(true).await;
    let (status, _h, body) = send(app, get("/ferroehr/rest/status")).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "UP");
    // The served identity is the released ITS-REST contract version — the
    // `openehr-its` crate version, via the single provenance constant.
    assert_eq!(
        v["openehr_rest_api_version"],
        ferroehr::telemetry::provenance::ITS_REST
    );
}

#[tokio::test]
async fn health_endpoint_is_public() {
    let (_pg, app) = app(true).await;
    let (status, _h, body) = send(app, get("/health")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body, "OK");
}

/// The whole health family is mounted unconditionally and ungated: these apps
/// run with authentication ENABLED and the management surface at its default
/// (disabled), and all three probes still answer without credentials.
/// `/health/liveness` is a byte-identical alias of `/health`; `/health/readiness`
/// renders the indicator aggregate (empty registry in a test build → `UP`).
/// This family is also the ONLY health surface — the retired REST-root
/// `/ferroehr/rest/status/health` name answers `404`.
#[tokio::test]
async fn health_family_is_public_without_the_management_surface() {
    let (_pg, app) = app(true).await;
    for path in ["/health", "/health/liveness"] {
        let (status, _h, body) = send(app.clone(), get(path)).await;
        assert_eq!(status, StatusCode::OK, "{path}");
        assert_eq!(body, "OK", "{path}");
    }

    let (status, headers, body) = send(app.clone(), get("/health/readiness")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["status"], "UP");
    assert!(v.get("components").is_some(), "indicator body: {body}");

    // There is no second name for health under the REST root: the retired
    // `/ferroehr/rest/status/health` alias is not routed at all.
    let (status, _h, _body) = send(app, get("/ferroehr/rest/status/health")).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn valid_basic_reaches_handler() {
    // RE-TARGET: the old Mock backend answered a blanket `501`; the real EHR
    // service answers `404` for a syntactically valid but non-existent EHR —
    // which still proves the authenticated request reached the real handler.
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{EHR}"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn wrong_basic_password_is_401() {
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/abc"))
        .header(header::AUTHORIZATION, basic("alice", "wrong"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn valid_bearer_reaches_handler() {
    // RE-TARGET: was a Mock `501`; the real handler answers `404` for the
    // non-existent EHR (the bearer token reached the handler).
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{EHR}"))
        .header(header::AUTHORIZATION, bearer("openid"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn admin_route_reachable_without_rbac() {
    // The legacy path-string `admin_scope` gate was removed; this harness builds
    // without an RBAC handle, so an authenticated caller reaches the admin
    // dispatcher regardless of scope. RE-TARGET: was a Mock `501`; the real
    // admin delete physically removes a real EHR → `204`, proving the route is
    // reachable and not RBAC-gated. The role-based admin gate is exercised
    // end-to-end in `rbac_e2e`.
    let (_pg, app) = app(true).await;

    // Create a real EHR (authenticated), then physically delete it via admin.
    let create = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, _b) = send(app.clone(), create).await;
    assert_eq!(status, StatusCode::CREATED);
    let ehr_id = etag_uid(&headers).expect("ETag carries the new ehr_id");

    let req = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/admin/ehr/{ehr_id}"))
        .header(header::AUTHORIZATION, bearer("openid"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
}

/// Extract the bare uid carried in a weak `ETag` (`W/"{uid}"`).
fn etag_uid(h: &header::HeaderMap) -> Option<String> {
    let raw = h.get(header::ETAG)?.to_str().ok()?;
    Some(raw.trim_start_matches("W/").trim_matches('"').to_owned())
}

#[tokio::test]
async fn json_composition_body_is_accepted_and_reaches_handler() {
    // The JSON body is decoded into the RM type and the handler is reached:
    // the EHR does not exist, so the real service answers `404` — not a
    // negotiation error, which is what "reaches the handler" asserts. The body
    // must therefore be a COMPLETE COMPOSITION: a `{"_type":"COMPOSITION"}`
    // stub carries none of the class's mandatory attributes, and since the
    // typed commit seam the strict reader refuses that at the door (`400`,
    // pinned by `bare_type_composition_stub_is_400` below) — which would prove
    // nothing about the handler.
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(minimal_composition_json()))
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// The refusing twin of the body above: a bare `{"_type":"COMPOSITION"}` names
/// the class but carries none of its mandatory attributes (`language`,
/// `territory`, `category`, `composer`, `name`, `archetype_node_id` — RM
/// composition `org.openehr.rm.composition.composition.adoc` §Attributes), so
/// it is not a COMPOSITION at all. The strict canonical reader refuses it at
/// the commit seam's door, which the ITS-REST overview
/// (`Requests_and_responses.md` §HTTP status codes) answers `400`.
#[tokio::test]
async fn bare_type_composition_stub_is_400() {
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"_type":"COMPOSITION"}"#))
        .unwrap();
    let (status, _h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.contains("missing field"),
        "the refusal names the absent mandatory attribute: {body}"
    );
}

/// A minimal COMPOSITION that satisfies every mandatory attribute of the RM
/// class, as canonical JSON (the wire form a client posts).
fn minimal_composition_json() -> String {
    json!({
        "_type": "COMPOSITION",
        "name": { "_type": "DV_TEXT", "value": "Encounter" },
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
            "rm_version": "1.2.0"
        },
        "language": { "_type": "CODE_PHRASE",
                      "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
                      "code_string": "en" },
        "territory": { "_type": "CODE_PHRASE",
                       "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
                       "code_string": "NL" },
        "category": { "_type": "DV_CODED_TEXT", "value": "event",
                      "defining_code": { "_type": "CODE_PHRASE",
                                         "terminology_id": { "_type": "TERMINOLOGY_ID",
                                                             "value": "openehr" },
                                         "code_string": "433" } },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "Dr Author" }
    })
    .to_string()
}

#[tokio::test]
async fn malformed_xml_composition_body_is_400() {
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from("<not-a-composition>"))
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    // The XML branch runs and fails to parse a COMPOSITION → BadRequest (before
    // the backend, so it is independent of whether the EHR exists).
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

/// End-to-end ITS-XML lineage negotiation on a real resource read.
///
/// openEHR publishes canonical XML in two lineages differing only by the root
/// namespace (`docs/specs/openehr/ITS-XML/README.adoc` §"Releases and IM
/// Versions"); a client picks one with the `version` media-type parameter.
/// NOTE: no openEHR spec governs the parameter — our own design/extension —
/// but its refusal branch is the released MUST (ITS-REST overview
/// `Resources.md` §"XML Format": "If the service cannot fulfill this aspect of
/// the request, it MUST respond with HTTP status code `406 Not Acceptable`").
#[tokio::test]
async fn xml_response_lineage_is_negotiated_by_the_version_parameter() {
    fn read(ehr_id: &str, accept: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{ehr_id}"))
            .header(header::AUTHORIZATION, basic("alice", "pw"))
            .header(header::ACCEPT, accept)
            .body(Body::empty())
            .unwrap()
    }

    let (_pg, app) = app(true).await;

    let create = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, _b) = send(app.clone(), create).await;
    assert_eq!(status, StatusCode::CREATED);
    let ehr_id = etag_uid(&headers).expect("ETag carries the new ehr_id");

    // Default (bare `application/xml`): the v2 lineage (owner ruling
    // 2026-08-03, #1666), bare Content-Type.
    let (status, headers, xml) = send(app.clone(), read(&ehr_id, "application/xml")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/xml"
    );
    assert!(
        xml.contains("xmlns=\"http://schemas.openehr.org/v2\""),
        "the default XML response is v2 (#1666): {xml}"
    );

    // Negotiated non-default v1: the v1 root namespace, and the response says so.
    let (status, headers, xml) =
        send(app.clone(), read(&ehr_id, "application/xml; version=1")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/xml; version=1"
    );
    assert!(
        xml.contains("xmlns=\"http://schemas.openehr.org/v1\""),
        "the negotiated XML response is v1: {xml}"
    );

    // A lineage this server does not serve cannot be fulfilled → 406.
    let (status, _h, _b) = send(app, read(&ehr_id, "application/xml; version=3")).await;
    assert_eq!(status, StatusCode::NOT_ACCEPTABLE);
}

/// The request-side mirror: a canonical-XML payload declaring a lineage the
/// service cannot process is `415` before any parsing (ITS-REST overview
/// `Resources.md` §"XML Format": "If the service cannot process the request
/// payload as XML format, it MUST respond with HTTP status code `415
/// Unsupported Media Type`").
#[tokio::test]
async fn xml_request_payload_in_an_unserved_lineage_is_415() {
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .header(header::CONTENT_TYPE, "application/xml; version=3")
        .body(Body::from("<composition/>"))
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::UNSUPPORTED_MEDIA_TYPE);
}

#[tokio::test]
async fn unknown_route_is_404() {
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/nonexistent"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// A known path called with a method it does not serve answers `405` with the
/// openEHR `{ error, message }` body AND the `Allow` header.
///
/// ITS-REST `docs/overview/Requests_and_responses.md` §HTTP Methods: "If a
/// method is recognized but not allowed for the target resource, the response
/// SHOULD be `405 Method Not Allowed` status code." RFC 9110 §15.5.6 — the
/// authority that section cites for the method semantics — makes the header
/// mandatory: "The origin server MUST generate an Allow header field in a 405
/// response containing a list of the target resource's currently supported
/// methods." `/ehr/{ehr_id}` serves `GET` and `PUT`, so both must be listed.
#[tokio::test]
async fn recognized_but_unallowed_method_is_405_with_allow() {
    let (_pg, app) = app(false).await;
    let req = Request::builder()
        .method("TRACE")
        .uri(format!("{BASE}/ehr/{EHR}"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(app, req).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    let allow = headers
        .get(header::ALLOW)
        .and_then(|v| v.to_str().ok())
        .expect("405 MUST carry Allow (RFC 9110 §15.5.6)");
    assert!(
        allow.contains("GET"),
        "Allow lists the served methods: {allow}"
    );
    assert!(
        allow.contains("PUT"),
        "Allow lists the served methods: {allow}"
    );
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v["error"], "Method Not Allowed");
}

/// The same `405` + `Allow` discipline for a method the server does not
/// recognize at all. ITS-REST §HTTP Methods SHOULDs `501` for an unrecognized
/// method; we answer `405` instead — a settled deviation: the router matches
/// on path + method and has no unrecognized-method
/// seam, and `405` is a predefined, non-conflicting code in the spec's own
/// status table (§HTTP status codes). What must never happen is a `405` without
/// `Allow`.
#[tokio::test]
async fn unrecognized_method_is_405_with_allow() {
    let (_pg, app) = app(false).await;
    let req = Request::builder()
        .method("FROBNICATE")
        .uri(format!("{BASE}/ehr/{EHR}"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
    assert!(
        headers.contains_key(header::ALLOW),
        "405 MUST carry Allow (RFC 9110 §15.5.6)"
    );
}

/// A request whose declared body size exceeds the server limit is refused by
/// the `tower-http` `RequestBodyLimitLayer` — an additional, non-conflicting
/// status code (ITS-REST `Requests_and_responses.md` §HTTP status codes:
/// "Additional status codes MAY be used as long as they do not conflict with
/// the predefined codes"). The refusal must still leave the server in the
/// openEHR `{ error, message }` shape, not tower-http's `text/plain` default.
#[tokio::test]
async fn oversized_request_body_is_413_with_the_openehr_error_body() {
    let (_pg, app) = app(false).await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header(header::CONTENT_TYPE, "application/json")
        // The limit is read from Content-Length, so the declared size alone
        // triggers the refusal — no 16 MiB allocation needed in the test.
        .header(header::CONTENT_LENGTH, "999999999")
        .body(Body::from(r#"{"_type":"COMPOSITION"}"#))
        .unwrap();
    let (status, headers, body) = send(app, req).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json"
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("openEHR error body");
    assert_eq!(v["error"], "Payload Too Large");
    assert!(
        v.get("message")
            .and_then(serde_json::Value::as_str)
            .is_some()
    );
}

#[tokio::test]
async fn auth_disabled_lets_requests_through() {
    // RE-TARGET: was a Mock `501`; with auth disabled the request reaches the
    // real handler, which answers `404` for the non-existent EHR.
    let (_pg, app) = app(false).await;
    let (status, _h, _b) = send(app, get(&format!("{BASE}/ehr/{EHR}"))).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

// The other groups mount identically; smoke-test one representative route of
// each (unauthenticated → 401; authenticated → reaches the real handler).

#[tokio::test]
async fn query_group_is_mounted_and_authenticated() {
    let (_pg, app) = app(true).await;
    let (status, _h, _b) = send(app.clone(), get(&format!("{BASE}/query/aql?q=SELECT%20c"))).await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/query/aql?q=SELECT%20c"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    // RE-TARGET: was a Mock `501`. The real query engine parses the AQL; the
    // incomplete `SELECT c` fails to parse → `400` (PreconditionViolation),
    // which still proves the authenticated request reached the real handler.
    assert_eq!(status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn demographic_group_is_mounted() {
    // RE-TARGET: was a Mock `501`; the real demographic service answers `404`
    // for a non-existent agent (the request reached the real handler).
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/demographic/agent/{EHR}"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn implemented_groups_are_reachable() {
    // The demographic / query / admin groups are real implementations, so this
    // probes their per-endpoint behaviour on a fresh database rather than a
    // uniform status: a missing agent → 404, an invalid AQL → 400, a missing
    // admin EHR → 404.
    // The POST case carries a well-formed JSON body so body deserialization
    // (which runs before dispatch) does not short-circuit to a 400.
    let cases: [(&str, String, &str, Option<&str>, StatusCode); 3] = [
        (
            "GET",
            format!("{BASE}/demographic/agent/{EHR}"),
            "openid",
            None,
            StatusCode::NOT_FOUND,
        ),
        (
            "GET",
            format!("{BASE}/query/aql?q=SELECT%20c"),
            "openid",
            None,
            StatusCode::BAD_REQUEST,
        ),
        (
            "DELETE",
            format!("{BASE}/admin/ehr/{EHR}"),
            "ferroehr:admin",
            None,
            StatusCode::NOT_FOUND,
        ),
    ];
    let (_pg, app) = app(true).await;
    for (method, uri, scope, body, expected) in cases {
        let mut builder = Request::builder()
            .method(method)
            .uri(&uri)
            .header(header::AUTHORIZATION, bearer(scope));
        if body.is_some() {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        let req = builder
            .body(body.map_or_else(Body::empty, Body::from))
            .unwrap();
        let (status, _headers, _body) = send(app.clone(), req).await;
        assert_eq!(status, expected, "{method} {uri}");
        // None of these reach the retired not-implemented default.
        assert_ne!(status, StatusCode::NOT_IMPLEMENTED, "{method} {uri}");
    }
}

#[tokio::test]
async fn definition_group_is_mounted_with_dotted_route() {
    // Exercises the dotted path segment (`adl1.4`) and dotted operation id.
    // RE-TARGET: was a Mock `501`; the real definition service lists the stored
    // OPT 1.4 templates → `200` with an empty JSON array on a fresh database.
    let (_pg, app) = app(true).await;
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK);
    let v: serde_json::Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v.as_array().map(Vec::len), Some(0), "no templates uploaded");
}

/// A default-committer audit (a write whose request carries NO committal
/// headers) is attributed to the AUTHENTICATED principal, not the system
/// identity (`AUDIT_DETAILS.committer` 1..1 — RM common master04 §Audit
/// Details; the committal-header merge only overrides what the caller
/// supplies).
#[tokio::test]
async fn authenticated_write_attributes_the_default_committer() {
    let (_pg, app) = app(true).await;

    // Create an EHR as `alice` (Basic), no committal headers.
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let ehr_id = resp
        .headers()
        .get("ETag")
        .and_then(|v| v.to_str().ok())
        .unwrap()
        .trim_matches(['W', '/', '"'])
        .to_owned();

    // The EHR_STATUS's stored commit audit names the principal.
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/versioned_ehr_status/revision_history"
        ))
        .header(header::AUTHORIZATION, basic("alice", "pw"))
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    let resp = app.clone().oneshot(req).await.unwrap();
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
        .await
        .unwrap();
    let history: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    let audit = &history["items"][0]["audits"][0];
    assert_eq!(
        audit["committer"]["name"], "alice",
        "default committer is the authenticated principal: {history}"
    );
    assert_eq!(
        audit["committer"]["identifiers"][0]["type"], "basic",
        "identifier records the mechanism"
    );
    // A Basic credential is held by this deployment, so the authority that
    // issues that kind of id (`DV_IDENTIFIER.issuer` — RM data_types
    // UML/classes/org.openehr.rm.data_types.dv_identifier.adoc §Attributes) is
    // the product itself. No openEHR spec governs the concrete string.
    assert_eq!(
        audit["committer"]["identifiers"][0]["issuer"], "ferroehr",
        "a locally-held credential names this deployment as issuer"
    );
}

/// The group-9 POST-body disciplines (#481): a stored-query POST accepts `{}`
/// (all three body members are OPTIONAL — the released OAS required-list loses
/// to the docs text: offset defaults 0, fetch is implementation-default), the
/// POSTs accept the URL parameter forms (the docs-text SHOULD-list draws no
/// GET/POST distinction), and a body-vs-URL conflict is a 400 — the same rule
/// the two `ehr_id` carriers follow.
#[tokio::test]
async fn query_post_body_optionality_and_url_forms() {
    let (_pg, app) = app(false).await;

    // Store a parameterless query, then execute it with `{}`.
    let store = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/definition/query/org.test::everything"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("SELECT e/ehr_id/value FROM EHR e"))
        .unwrap();
    let (status, _h, body) = send(app.clone(), store).await;
    assert_eq!(status, StatusCode::OK, "store: {body}");

    let exec_empty = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/query/org.test::everything"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _h, body) = send(app.clone(), exec_empty).await;
    assert_eq!(status, StatusCode::OK, "empty body executes: {body}");

    // URL forms on the POST: fetch from the URL applies.
    let exec_url = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/query/org.test::everything?fetch=1"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from("{}"))
        .unwrap();
    let (status, _h, body) = send(app.clone(), exec_url).await;
    assert_eq!(status, StatusCode::OK, "URL fetch on POST: {body}");

    // Conflict: the same key in both places with different values → 400.
    let exec_conflict = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/query/org.test::everything?fetch=1"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"fetch": 2}"#))
        .unwrap();
    let (status, _h, _b) = send(app.clone(), exec_conflict).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body-vs-URL conflict");

    // Equal values in both places are accepted.
    let exec_equal = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/query/org.test::everything?fetch=2"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(r#"{"fetch": 2}"#))
        .unwrap();
    let (status, _h, body) = send(app.clone(), exec_equal).await;
    assert_eq!(status, StatusCode::OK, "equal values agree: {body}");
}

// ── Response security headers (OWASP HTTP Headers Cheat Sheet, issue #2015) ──

/// Every response carries the header set, on a handler response and on a
/// transport-layer one alike — the layer is outermost precisely so a `413` or a
/// `408`, which never reach a handler, are covered too.
#[tokio::test]
async fn every_response_carries_the_security_headers() {
    let (_pg, app) = app(false).await;
    let req = Request::builder()
        .uri(format!("{BASE}/ehr/{EHR}"))
        .body(Body::empty())
        .unwrap();
    let (_status, headers, _body) = send(app, req).await;
    for (name, expected) in [
        ("cache-control", "no-store"),
        ("x-content-type-options", "nosniff"),
        ("referrer-policy", "strict-origin-when-cross-origin"),
        ("cross-origin-resource-policy", "same-site"),
        ("x-frame-options", "DENY"),
        (
            "content-security-policy",
            "default-src 'none'; frame-ancestors 'none'",
        ),
    ] {
        assert_eq!(
            headers.get(name).map(|v| v.to_str().unwrap_or_default()),
            Some(expected),
            "{name} must be present with the audited value"
        );
    }
}

/// A router with the Swagger UI mounted — the shared fixture turns it off, and
/// these two tests are about the page it serves.
async fn app_with_swagger() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    let mut cfg = config(false);
    cfg.server.swagger_ui = ferroehr::config::management::AccessLevel::Public;
    (pg, common::router_with(cfg, service))
}

/// Swagger UI must get a policy it can actually run under.
///
/// The API's `default-src 'none'` is right for JSON and fatal for the one surface
/// on this origin that is rendered: the vendored bundle's own same-origin scripts
/// and styles would be blocked and the page would come up blank. Under the
/// default configuration `swagger_ui` is ON, so this was a shipped regression
/// waiting to be reported as "the docs are broken".
#[tokio::test]
async fn swagger_ui_gets_a_policy_it_can_run_under() {
    let (_pg, app) = app_with_swagger().await;
    let req = Request::builder()
        .uri("/ferroehr/rest/swagger-ui/index.html")
        .body(Body::empty())
        .unwrap();
    let (status, headers, _body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK);
    let csp = headers
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        csp.contains("script-src 'self'"),
        "the rendered surface needs its own script policy, got {csp:?}"
    );
    assert!(
        !csp.contains("default-src 'none'"),
        "the API policy would blank the page: {csp:?}"
    );
}

/// The Swagger CSP carries NO inline allowance, and the served page needs none.
///
/// This asserts both halves, because the second is what makes the first safe: if
/// the vendored distribution ever starts inlining a script or a style, this test
/// fails and says so — rather than the page silently coming up blank in a
/// CSP-enforcing browser, which is the failure mode that gets "fixed" by adding
/// `'unsafe-inline'` back.
#[tokio::test]
async fn the_swagger_page_needs_no_inline_allowance() {
    let (_pg, app) = app_with_swagger().await;
    let req = Request::builder()
        .uri("/ferroehr/rest/swagger-ui/index.html")
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK);

    let csp = headers
        .get(header::CONTENT_SECURITY_POLICY)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    assert!(
        !csp.contains("'unsafe-inline'"),
        "the policy must not permit inline: {csp:?}"
    );

    assert!(
        !body.contains("<style"),
        "an inline <style> would need style-src 'unsafe-inline'"
    );
    // Split rather than slice: `clippy::string_slice` is denied workspace-wide
    // because a byte range can land mid-character, and this walks HTML.
    for fragment in body.split("<script").skip(1) {
        let Some((_attributes, rest)) = fragment.split_once('>') else {
            continue;
        };
        let Some((inline_body, _)) = rest.split_once("</script>") else {
            continue;
        };
        assert!(
            inline_body.trim().is_empty(),
            "an inline <script> body would need script-src 'unsafe-inline'"
        );
    }
}

/// And the JSON surface keeps the strict policy — the Swagger exception must not
/// leak onto the API responses next to it.
#[tokio::test]
async fn the_api_keeps_the_strict_policy() {
    let (_pg, app) = app(false).await;
    let req = Request::builder()
        .uri(format!("{BASE}/ehr/{EHR}"))
        .body(Body::empty())
        .unwrap();
    let (_status, headers, _body) = send(app, req).await;
    assert_eq!(
        headers.get(header::CONTENT_SECURITY_POLICY).unwrap(),
        "default-src 'none'; frame-ancestors 'none'"
    );
}

/// Two properties the header audit asked about explicitly.
///
/// No `Server` header: `hyper` emits none, and nothing in the stack adds one, so
/// the version-disclosure the OWASP HTTP Headers Cheat Sheet warns about does not
/// arise. Asserted rather than assumed, because a future middleware could add one.
///
/// And `Cache-Control: no-store` does not disturb the spec's concurrency control.
/// `ETag` is a PRECONDITION mechanism — the client echoes it in `If-Match` on the
/// next write (ITS-REST overview `Requests_and_responses.md`) — not a caching one,
/// so refusing to store a response and identifying its version are independent.
/// This pins both appearing on the same response.
#[tokio::test]
async fn no_server_header_and_no_store_coexists_with_etag() {
    let (_pg, app) = app(false).await;
    let create = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(app, create).await;
    assert!(
        !headers.contains_key(header::SERVER),
        "a Server header would disclose the implementation for no benefit"
    );
    assert_eq!(status, StatusCode::CREATED, "{body}");
    assert!(
        headers.contains_key(header::ETAG),
        "the spec-required ETag must survive Cache-Control: no-store"
    );
    assert_eq!(headers.get(header::CACHE_CONTROL).unwrap(), "no-store");
}

/// `Strict-Transport-Security` is deliberately NOT sent: RFC 6797 §7.2 requires
/// a browser to ignore it over plain HTTP, which is how this server is commonly
/// reached behind a terminating proxy, and the TLS edge owns the header. This
/// asserts the deliberate absence so nobody "fixes" it by adding an inert
/// header here.
#[tokio::test]
async fn hsts_is_deliberately_absent() {
    let (_pg, app) = app(false).await;
    let req = Request::builder()
        .uri(format!("{BASE}/ehr/{EHR}"))
        .body(Body::empty())
        .unwrap();
    let (_status, headers, _body) = send(app, req).await;
    assert!(
        !headers.contains_key("strict-transport-security"),
        "HSTS belongs to the TLS edge, not this listener"
    );
}

// ── Request-body limits (issue #2019; the chunked defect is issue #2045) ─────

/// A body over the limit with NO `Content-Length` — a chunked upload — must be
/// refused `413`. It used to reach the dispatcher as a silently emptied body and
/// answer `400`, because the collector turned the limit error into a default
/// value (issue #2045).
#[tokio::test]
async fn an_over_limit_chunked_body_is_413_not_400() {
    let (pg, service) = common::test_service().await;
    let mut cfg = config(false);
    cfg.server.limits = BodyLimits {
        body_bytes: 1024,
        bulk_body_bytes: 1024,
    };
    let app = common::router_with(cfg, service);
    let _keep = pg;

    // A stream body has no Content-Length, so the declared-size check cannot
    // fire and the limit must be enforced while reading.
    let oversized = vec![b'x'; 4096];
    let stream = futures::stream::once(async move { Ok::<_, std::io::Error>(oversized) });
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from_stream(stream))
        .unwrap();
    let (status, headers, body) = send(app, req).await;
    assert_eq!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "an over-limit chunked body must be 413, not a malformed-body 400: {body}"
    );
    assert_eq!(
        headers.get(header::CONTENT_TYPE).unwrap(),
        "application/json",
        "the refusal keeps the openEHR error body"
    );
}

/// The bulk tier really is more permissive than the clinical tier: the same
/// payload that a clinical route refuses is accepted for reading on a bulk
/// route (it then fails on its own merits, not on size).
#[tokio::test]
async fn the_bulk_tier_accepts_what_the_clinical_tier_refuses() {
    let (pg, service) = common::test_service().await;
    let mut cfg = config(false);
    cfg.server.limits = BodyLimits {
        body_bytes: 512,
        bulk_body_bytes: 65536,
    };
    let app = common::router_with(cfg, service);
    let _keep = pg;

    let payload = vec![b'x'; 4096];
    let clinical = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{EHR}/composition"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(payload.clone()))
        .unwrap();
    let (status, _h, _b) = send(app.clone(), clinical).await;
    assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);

    let bulk = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(payload))
        .unwrap();
    let (status, _h, _b) = send(app, bulk).await;
    assert_ne!(
        status,
        StatusCode::PAYLOAD_TOO_LARGE,
        "the bulk tier must not refuse this on size"
    );
}

// ── Rate limiting (issue #2020) ─────────────────────────────────────────────

/// Past its rate, a caller is refused `429` with `Retry-After` and the openEHR
/// error body. Driven with a rate of one per second and a burst of one, so the
/// second request is already over.
#[tokio::test]
async fn past_the_rate_a_caller_is_429_with_retry_after() {
    let (pg, service) = common::test_service().await;
    let mut cfg = config(false);
    cfg.server.rate_limit = RateLimitConfig {
        enabled: true,
        principal_per_second: 1,
        principal_burst: 1,
        address_per_second: 1,
        address_burst: 1,
    };
    let app = common::router_with(cfg, service);
    let _keep = pg;

    let mut refused = None;
    for _ in 0..4 {
        let req = Request::builder()
            .uri(format!("{BASE}/ehr/{EHR}"))
            .body(Body::empty())
            .unwrap();
        let (status, headers, body) = send(app.clone(), req).await;
        if status == StatusCode::TOO_MANY_REQUESTS {
            refused = Some((headers, body));
            break;
        }
    }
    let (headers, body) = refused.expect("a burst of 1 must produce a 429 within four requests");
    assert!(
        headers.contains_key(header::RETRY_AFTER),
        "429 must tell the caller when to retry"
    );
    let v: serde_json::Value = serde_json::from_str(&body).expect("openEHR error body");
    assert_eq!(v["error"], "Too Many Requests");
    assert!(
        v["message"]
            .as_str()
            .is_some_and(|m| m.contains("retry in")),
        "the message states the delay: {body}"
    );
}

/// Disabled, the limiter must produce zero wire drift — no refusal and no
/// `x-ratelimit-*` headers at all.
#[tokio::test]
async fn a_disabled_limiter_leaves_no_trace_on_the_wire() {
    let (pg, service) = common::test_service().await;
    let mut cfg = config(false);
    cfg.server.rate_limit = RateLimitConfig {
        enabled: false,
        principal_per_second: 1,
        principal_burst: 1,
        address_per_second: 1,
        address_burst: 1,
    };
    let app = common::router_with(cfg, service);
    let _keep = pg;

    for _ in 0..4 {
        let req = Request::builder()
            .uri(format!("{BASE}/ehr/{EHR}"))
            .body(Body::empty())
            .unwrap();
        let (status, headers, _body) = send(app.clone(), req).await;
        assert_ne!(status, StatusCode::TOO_MANY_REQUESTS);
        assert!(
            headers
                .keys()
                .all(|k| !k.as_str().starts_with("x-ratelimit")),
            "a disabled limiter must add no headers"
        );
    }
}

// ── Sensitive data in URLs (issue #2044) ────────────────────────────────────

/// `subject_id` is an external patient identifier that the specification puts in
/// a QUERY parameter, so the request span must not record the query string. The
/// span is built by `router::path_only_span`; this asserts the endpoint still
/// works while the span carries only the path — the unit-level guarantee is that
/// no code path formats `uri()` in full.
#[tokio::test]
async fn subject_lookup_by_query_parameter_still_answers() {
    let (_pg, app) = app(false).await;
    let req = Request::builder()
        .uri(format!(
            "{BASE}/ehr?subject_id=patient-12345&subject_namespace=hospital"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, _headers, _body) = send(app, req).await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::OK,
        "subject lookup answers on its own merits, got {status}"
    );
}

// ── The served document describes the LIVE router (issue #2073) ─────────────

/// Under the DEFAULT configuration — Swagger off, SMART off — the served
/// document must not advertise the paths those features would have mounted.
///
/// This is the property the whole "serve only what we generate" rule exists for:
/// a client generated from the document must not receive endpoints that answer
/// `404`. It was broken inside the generator itself.
#[tokio::test]
async fn the_served_document_omits_paths_whose_features_are_off() {
    let (pg, service) = common::test_service().await;
    let mut cfg = config(false);
    cfg.server.swagger_ui = ferroehr::config::management::AccessLevel::Off;
    cfg.smart.enabled = false;
    let app = common::router_with(cfg, service);
    let _keep = pg;

    let req = Request::builder()
        .uri(format!("{BASE}/ehr/{EHR}"))
        .body(Body::empty())
        .unwrap();
    let (_status, _headers, _body) = send(app.clone(), req).await;

    let document = ferroehr_rest::extensions::openapi::extensions_document(&config(false));
    let paths: Vec<String> = document.paths.paths.keys().cloned().collect();
    for absent in [
        "/ferroehr/rest/api-docs/openapi.json",
        "/ferroehr/rest/swagger-ui",
        "/ferroehr/rest/.well-known/smart-configuration",
    ] {
        assert!(
            !paths.iter().any(|p| p == absent),
            "{absent} answers 404 in this configuration and must not be advertised"
        );
    }
}
