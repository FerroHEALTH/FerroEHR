// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the ADL 1.4 **template definition** resource
//! (`POST /definition/template/adl1.4`,
//! `GET /definition/template/adl1.4/{template_id}`) — the negotiation contract
//! of `docs/specs/openehr/ITS-REST/specifications/docs/overview/Resources.md`:
//!
//! * §XML Format — "If the service cannot process the request payload as XML
//!   format, it MUST respond with HTTP status code `415 Unsupported Media
//!   Type`": an OPT upload declaring a non-XML `Content-Type` is `415`, never
//!   a `400` from the parser. An absent `Content-Type` declares nothing to
//!   refuse (the header is a client MAY) and reads as the operation's single
//!   body type.
//! * §JSON Format — "Proper header `Content-Type: application/json` MUST be
//!   present in the response of the service unless the response has no content
//!   body": the retrieval answers with the media type the client negotiated —
//!   `application/json` and `application/openehr.wt+json` both serve the Web
//!   Template document, each under its own type; `application/xml` serves the
//!   canonical OPT.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::path::PathBuf;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const TEMPLATE_ID: &str = "Demo Vitals";
const JSON_MIME: &str = "application/json";
const XML_MIME: &str = "application/xml";
const WT_MIME: &str = "application/openehr.wt+json";

fn opt_xml() -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/openehr-sdt/tests/fixtures/better/Demo Vitals.opt");
    std::fs::read_to_string(path).expect("Demo Vitals.opt vendored in openehr-its")
}

fn config() -> AppConfig {
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
        ..Default::default()
    }
}

/// A router over a fresh real service with no template loaded.
async fn empty_app() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(config(), service))
}

/// `POST /definition/template/adl1.4` with the given `Content-Type` (absent
/// when `None`) and the Demo Vitals OPT as the body.
async fn upload(app: &Router, content_type: Option<&str>) -> (StatusCode, String) {
    let mut builder = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"));
    if let Some(ct) = content_type {
        builder = builder.header(header::CONTENT_TYPE, ct);
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::from(opt_xml())).unwrap())
        .await
        .expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

/// A router over a fresh real service with the Demo Vitals OPT uploaded
/// through the real wire.
async fn app_with_template() -> (testkit::TestDb, Router) {
    let (pg, app) = empty_app().await;
    let (status, body) = upload(&app, Some(XML_MIME)).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the Demo Vitals OPT uploads as application/xml: {body}"
    );
    (pg, app)
}

fn template_uri() -> String {
    // The template id carries a space; percent-encode it in the path segment.
    format!(
        "{BASE}/definition/template/adl1.4/{}",
        TEMPLATE_ID.replace(' ', "%20")
    )
}

async fn get(app: &Router, accept: Option<&str>) -> (StatusCode, Option<String>, String) {
    let mut builder = Request::builder().method("GET").uri(template_uri());
    if let Some(a) = accept {
        builder = builder.header(header::ACCEPT, a);
    }
    let resp = app
        .clone()
        .oneshot(builder.body(Body::empty()).unwrap())
        .await
        .expect("response");
    let status = resp.status();
    let content_type = resp
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        content_type,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

// ── GET: the response type mirrors the negotiated Accept ────────────────────

#[tokio::test]
async fn template_get_default_is_the_canonical_opt() {
    let (_pg, app) = app_with_template().await;
    let (status, content_type, body) = get(&app, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(content_type.as_deref(), Some(XML_MIME));
    assert!(body.contains("<template"), "canonical OPT root: {body}");
}

#[tokio::test]
async fn template_get_as_xml_is_the_canonical_opt() {
    let (_pg, app) = app_with_template().await;
    let (status, content_type, body) = get(&app, Some(XML_MIME)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        content_type.as_deref(),
        Some(XML_MIME),
        "Resources.md §XML Format: \"Proper header Content-Type: application/xml \
         MUST be present in the response of the service\""
    );
    assert!(body.contains("<template"), "canonical OPT root: {body}");
}

#[tokio::test]
async fn template_get_as_wt_json_keeps_the_web_template_media_type() {
    let (_pg, app) = app_with_template().await;
    let (status, content_type, body) = get(&app, Some(WT_MIME)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        content_type.as_deref(),
        Some(WT_MIME),
        "Resources.md §Simplified Formats: application/openehr.wt+json is the \
         Operational Template as Web Template JSON — the negotiated type is the \
         response type"
    );
    let wt: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        wt.get("templateId").and_then(Value::as_str),
        Some(TEMPLATE_ID),
        "the Web Template document: {body}"
    );
}

/// `Accept: application/json` is honoured (both `Accept_Template.yaml` and
/// `ContentType_Template.yaml` enumerate it) and answered with the Web
/// Template document under the type the client asked for — NOT
/// `application/openehr.wt+json`, which the client never accepted
/// (`Resources.md` §JSON Format: "Proper header `Content-Type:
/// application/json` MUST be present in the response of the service unless the
/// response has no content body").
#[tokio::test]
async fn template_get_as_json_answers_with_application_json() {
    let (_pg, app) = app_with_template().await;
    let (status, content_type, body) = get(&app, Some(JSON_MIME)).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        content_type.as_deref(),
        Some(JSON_MIME),
        "Resources.md §JSON Format: \"Proper header Content-Type: application/json \
         MUST be present in the response of the service unless the response has no \
         content body\" — an Accept: application/json read must not be answered with \
         application/openehr.wt+json"
    );
    let wt: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        wt.get("templateId").and_then(Value::as_str),
        Some(TEMPLATE_ID),
        "the only JSON template representation the spec defines is the Web \
         Template document: {body}"
    );
}

#[tokio::test]
async fn template_get_unsupported_accept_is_406() {
    let (_pg, app) = app_with_template().await;
    let (status, _content_type, _body) = get(&app, Some("application/pdf")).await;
    assert_eq!(
        status,
        StatusCode::NOT_ACCEPTABLE,
        "Resources.md §JSON Format: \"If the service cannot fulfil this aspect of \
         the request, it MUST respond with HTTP status code 406 Not Acceptable\""
    );
}

// ── POST: a non-XML payload type is 415, never a parse-time 400 ─────────────

#[tokio::test]
async fn opt_upload_as_xml_is_created() {
    let (_pg, app) = empty_app().await;
    let (status, body) = upload(&app, Some(XML_MIME)).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

#[tokio::test]
async fn opt_upload_with_json_content_type_is_415() {
    let (_pg, app) = empty_app().await;
    let (status, body) = upload(&app, Some(JSON_MIME)).await;
    assert_eq!(
        status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "Resources.md §XML Format: \"If the service cannot process the request \
         payload as XML format, it MUST respond with HTTP status code 415 \
         Unsupported Media Type\" — not the parser's 400: {body}"
    );
}

#[tokio::test]
async fn opt_upload_with_text_xml_content_type_is_created() {
    let (_pg, app) = empty_app().await;
    let (status, body) = upload(&app, Some("text/xml")).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "text/xml is the XML payload format: {body}"
    );
}

/// An absent `Content-Type` declares nothing to refuse — `Resources.md`
/// §XML Format makes the header a client MAY ("A client MAY use the header
/// `Content-Type: application/xml` in the requests to specify the XML payload
/// format") — so the operation's single body type applies.
#[tokio::test]
async fn opt_upload_without_content_type_is_created() {
    let (_pg, app) = empty_app().await;
    let (status, body) = upload(&app, None).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
}

// ── POST: the 400-vs-422 invalid-payload split ───────────────────────────────
// `responses/400.yaml`: "400 Bad Request is returned when the request could
// not be parsed or is invalid (e.g. … syntactically invalid … content)" — the
// released branch for a payload that is not well-formed XML. A WELL-FORMED
// document that is not a valid OPT is a semantic error (overview
// `Requests_and_responses.md` §HTTP status codes, the 422 row; no template
// operation declares 422, so the semantic branch is register-adjudicated).

/// `POST /definition/template/adl1.4` with an arbitrary body under
/// `Content-Type: application/xml`.
async fn upload_body(app: &Router, body: &str) -> (StatusCode, String) {
    let resp = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(format!("{BASE}/definition/template/adl1.4"))
                .header(header::CONTENT_TYPE, XML_MIME)
                .body(Body::from(body.to_owned()))
                .unwrap(),
        )
        .await
        .expect("response");
    let status = resp.status();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (status, String::from_utf8_lossy(&bytes).into_owned())
}

#[tokio::test]
async fn opt_upload_malformed_xml_is_400() {
    let (_pg, app) = empty_app().await;
    let (status, body) = upload_body(&app, "<template><language></template>").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "mismatched tags are syntactically invalid content — the released 400 \
         branch (responses/400.yaml), never 422: {body}"
    );
}

#[tokio::test]
async fn opt_upload_empty_body_is_400() {
    let (_pg, app) = empty_app().await;
    let (status, body) = upload_body(&app, "").await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an empty document has no root element — not well-formed XML, the \
         released 400 branch (responses/400.yaml): {body}"
    );
}

#[tokio::test]
async fn opt_upload_well_formed_non_opt_is_422() {
    let (_pg, app) = empty_app().await;
    let (status, body) = upload_body(&app, "<not_an_opt/>").await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "well-formed XML that does not decode as an OPT is the semantic 422 \
         branch (overview §HTTP status codes), not the syntactic 400: {body}"
    );
}

/// An OPT whose typed `default_value` has no content, the shape Archetype
/// Designer emits by itself, is refused `422` with a message that names the
/// element, its line and the class attribute the absent child realises, so
/// the author can find an element that is not there (#3067). The refusal
/// itself is unchanged: `DV_IDENTIFIER.id` is mandatory (RM `Id_valid`).
#[tokio::test]
async fn opt_upload_with_an_empty_typed_default_value_names_the_place() {
    let (_pg, app) = empty_app().await;
    // The overlay Archetype Designer emits for a blank default value, appended
    // before the closing `</template>`; its `default_value` is the block's
    // fifth line, indented eight spaces.
    let source = common::ips_opt_xml();
    let closing = source
        .lines()
        .position(|l| l.trim() == "</template>")
        .expect("the IPS OPT closes its template element");
    let block = "  <constraints>\n    <attributes>\n      <rm_attribute_name>value</rm_attribute_name>\n      \
                 <children>\n        <default_value xsi:type=\"DV_IDENTIFIER\"/>\n      </children>\n      \
                 <differential_path>/content[openEHR-EHR-SECTION.medications_ips.v0]</differential_path>\n    \
                 </attributes>\n  </constraints>\n";
    let mut body = String::with_capacity(source.len() + block.len());
    for (index, l) in source.lines().enumerate() {
        if index == closing {
            body.push_str(block);
        }
        body.push_str(l);
        body.push('\n');
    }
    let line = closing + 5;
    let (status, text) = upload_body(&app, &body).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{text}");
    let message = serde_json::from_str::<Value>(&text).expect("an error document")["message"]
        .as_str()
        .expect("message")
        .to_owned();
    for expected in [
        r#"element <default_value xsi:type="DV_IDENTIFIER">"#,
        &format!("at line {line}, column 9"),
        "is missing mandatory child <id> (DV_IDENTIFIER.id)",
    ] {
        assert!(
            message.contains(expected),
            "{expected:?} not in {message:?}"
        );
    }
    assert!(!message.contains("xml parse error"), "{message}");
}
