// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the ADL2 template wire (SM-2, `I_DEFINITION_ADL2`):
//! `POST /definition/template/adl2` (text/plain source upload, `Location` +
//! `Prefer` body, `422`-with-rule-codes on an invalid source),
//! `GET /definition/template/adl2/{template_id}` (text/plain source /
//! `application/json` `OperationalTemplateV2` / `406` on xml-only),
//! `GET …/{template_id}/{version}` (the deprecated versioned get),
//! `GET …/{template_id}/example` (a generated example COMPOSITION across the
//! four `Accept_LOCATABLE` forms, with `type`/`detail_level` + 400/404/406),
//! and `GET /definition/template/adl2`
//! (`TemplateMetadata` list). Driven through the assembled router over a
//! **real** `FerroEhrService` on a real `PostgreSQL` — the source is a spec-valid
//! ADL2 operational template validated by the `openehr-adl` engine, uploaded
//! through the wire and stored verbatim, so the text/plain GET echoes it
//! exactly.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::Value;
use std::fmt::Write;
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const HRID: &str = "openEHR-EHR-COMPOSITION.t_clinical_info.v1.0.0";

/// A spec-valid ADL2 operational-template source (`adl_version=2.0.6`), the same
/// shape `app/ferroehr/tests/service_definition.rs` builds: header + HRID,
/// `language`, `description` (mandatory — AOM2 master03 §Validity Rules VARD),
/// `definition` (root `id1`), `terminology` blocks. The `openehr-adl` engine
/// validates it, and the store keeps it verbatim.
fn adl2_source(keyword: &str, hrid: &str) -> String {
    let rm_type = hrid
        .split('.')
        .next()
        .and_then(|q| q.rsplit_once('-').map(|(_, e)| e))
        .expect("HRID carries an RM entity");
    format!(
        "{keyword} (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    {rm_type}[id1] matches {{ *}}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>\n        >\n    >\n"
    )
}

fn source() -> String {
    adl2_source("operational_template", HRID)
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

async fn app() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(config(), service))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, String) {
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

/// The `Preference-Applied` field value, which every write path declares —
/// "The service MAY include a `Preference-Applied` header in the response …
/// to indicate that the client's preference has been honored" (ITS-REST
/// overview `Requests_and_responses.md` §Representation details negotiation).
fn preference_applied(h: &header::HeaderMap) -> Option<&str> {
    h.get("preference-applied").and_then(|v| v.to_str().ok())
}

fn upload_req(prefer: Option<&str>) -> Request<Body> {
    let mut b = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain");
    if let Some(p) = prefer {
        b = b.header("Prefer", p);
    }
    b.body(Body::from(source())).unwrap()
}

#[tokio::test]
async fn upload_minimal_returns_201_and_location_only() {
    let (_pg, app) = app().await;
    let (status, headers, body) = send(&app, upload_req(None)).await;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(
        headers.get(header::LOCATION).unwrap().to_str().unwrap(),
        format!("{BASE}/definition/template/adl2/{HRID}")
    );
    assert!(
        body.is_empty(),
        "return=minimal has an empty body: {body:?}"
    );
    assert_eq!(
        preference_applied(&headers),
        Some("return=minimal"),
        "no Prefer was sent, so the applied preference is the default \
         (overview §Representation details negotiation)"
    );
}

#[tokio::test]
async fn upload_representation_returns_source_text() {
    let (_pg, app) = app().await;
    let (status, headers, body) = send(&app, upload_req(Some("return=representation"))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    assert!(headers.contains_key(header::LOCATION));
    assert_eq!(body, source(), "representation echoes the OPT source");
    assert_eq!(preference_applied(&headers), Some("return=representation"));
}

#[tokio::test]
async fn upload_identifier_returns_template_id_json() {
    let (_pg, app) = app().await;
    let (status, headers, body) = send(&app, upload_req(Some("return=identifier"))).await;
    assert_eq!(status, StatusCode::CREATED);
    assert!(headers.contains_key(header::LOCATION));
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v.get("template_id").and_then(Value::as_str), Some(HRID));
    assert_eq!(preference_applied(&headers), Some("return=identifier"));
}

#[tokio::test]
async fn get_serves_source_as_text_and_404s_unknown() {
    let (_pg, app) = app().await;
    // Upload first so the artefact exists.
    let (status, _h, _b) = send(&app, upload_req(None)).await;
    assert_eq!(status, StatusCode::CREATED);

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/definition/template/adl2/{HRID}"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    assert_eq!(body, source());

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/definition/template/adl2/openEHR-EHR-COMPOSITION.absent.v1.0.0"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn list_returns_template_metadata() {
    let (_pg, app) = app().await;
    let (status, _h, _b) = send(&app, upload_req(None)).await;
    assert_eq!(status, StatusCode::CREATED);

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/definition/template/adl2"))
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    let list = v.as_array().expect("array");
    assert_eq!(list.len(), 1);
    let row = &list[0];
    // TemplateMetadata: template_id + concept + archetype_id + created_timestamp
    // (schemas/definition/TemplateMetadata.yaml).
    assert_eq!(row.get("template_id").and_then(Value::as_str), Some(HRID));
    assert_eq!(row.get("archetype_id").and_then(Value::as_str), Some(HRID));
    assert_eq!(
        row.get("concept").and_then(Value::as_str),
        Some("t_clinical_info"),
        "concept is the HRID concept segment"
    );
    assert!(row.get("created_timestamp").is_some());
}

#[tokio::test]
async fn get_serves_operational_template_v2_json() {
    let (_pg, app) = app().await;
    let (status, _h, _b) = send(&app, upload_req(None)).await;
    assert_eq!(status, StatusCode::CREATED);

    // Accept: application/json → the OperationalTemplateV2 canonical-JSON
    // projection (200_Template_adl2_retrieved.yaml, application/json branch).
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/definition/template/adl2/{HRID}"))
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("application/json")
    );
    let v: Value = serde_json::from_str(&body).expect("OperationalTemplateV2 is a JSON object");
    // AOM2 canonical JSON self-tags every object with `_type`
    // (OperationalTemplateV2 is an opaque object; any JSON object satisfies it).
    assert!(v.is_object(), "OperationalTemplateV2 body is an object");
    assert_eq!(
        v.get("_type").and_then(Value::as_str),
        Some("OPERATIONAL_TEMPLATE")
    );
}

#[tokio::test]
async fn get_406_when_only_xml_acceptable() {
    let (_pg, app) = app().await;
    let (status, _h, _b) = send(&app, upload_req(None)).await;
    assert_eq!(status, StatusCode::CREATED);

    // application/xml has no declared response body → 406.
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/definition/template/adl2/{HRID}"))
        .header(header::ACCEPT, "application/xml")
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::NOT_ACCEPTABLE);
}

#[tokio::test]
async fn version_get_resolves_and_serves_both_representations() {
    let (_pg, app) = app().await;
    let (status, _h, _b) = send(&app, upload_req(None)).await;
    assert_eq!(status, StatusCode::CREATED);

    // The deprecated versioned get: template family + a `1` major prefix →
    // the stored v1.0.0 source (text/plain).
    let concept_family = "openEHR-EHR-COMPOSITION.t_clinical_info";
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/definition/template/adl2/{concept_family}/1"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("text/plain")
    );
    assert_eq!(body, source());

    // application/json → the OperationalTemplateV2 projection at that version.
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/definition/template/adl2/{concept_family}/1.0.0"
        ))
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(
        v.get("_type").and_then(Value::as_str),
        Some("OPERATIONAL_TEMPLATE")
    );

    // A version that does not exist → 404.
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/definition/template/adl2/{concept_family}/9"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, _, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::NOT_FOUND);
}

/// Build the example request for `HRID` with an optional `Accept` and query.
fn example_req(accept: Option<&str>, query: &str) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(format!(
        "{BASE}/definition/template/adl2/{HRID}/example{query}"
    ));
    if let Some(a) = accept {
        b = b.header(header::ACCEPT, a);
    }
    b.body(Body::empty()).unwrap()
}

#[tokio::test]
async fn example_get_serves_all_four_accept_forms() {
    let (_pg, app) = app().await;
    assert_eq!(send(&app, upload_req(None)).await.0, StatusCode::CREATED);

    // Default (no Accept) → canonical JSON COMPOSITION.
    let (status, headers, body) = send(&app, example_req(None, "")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        headers
            .get(header::CONTENT_TYPE)
            .unwrap()
            .to_str()
            .unwrap()
            .contains("json")
    );
    let v: Value = serde_json::from_str(&body).unwrap();
    assert_eq!(v.get("_type").and_then(Value::as_str), Some("COMPOSITION"));

    // The four `Accept_LOCATABLE` representations all return 200.
    for accept in [
        "application/json",
        "application/xml",
        "application/openehr.wt.flat+json",
        "application/openehr.wt.structured+json",
    ] {
        let (status, ct, _) = send(&app, example_req(Some(accept), "")).await;
        assert_eq!(status, StatusCode::OK, "Accept {accept} → 200");
        assert!(ct.contains_key(header::CONTENT_TYPE));
    }
}

#[tokio::test]
async fn example_get_honours_type_and_detail_level() {
    let (_pg, app) = app().await;
    assert_eq!(send(&app, upload_req(None)).await.0, StatusCode::CREATED);

    // `type=output` carries a populated uid; every detail level is served.
    for query in [
        "?detail_level=required",
        "?detail_level=medium",
        "?detail_level=complete",
    ] {
        assert_eq!(send(&app, example_req(None, query)).await.0, StatusCode::OK);
    }
    let (_s, _h, body) = send(&app, example_req(None, "?type=output")).await;
    let v: Value = serde_json::from_str(&body).unwrap();
    assert!(
        v.pointer("/uid/value").is_some(),
        "output form carries a uid"
    );
}

#[tokio::test]
async fn example_get_400_on_bad_enum_404_unknown_406_wrong_accept() {
    let (_pg, app) = app().await;
    assert_eq!(send(&app, upload_req(None)).await.0, StatusCode::CREATED);

    // Out-of-enum detail_level → 400.
    assert_eq!(
        send(&app, example_req(None, "?detail_level=full")).await.0,
        StatusCode::BAD_REQUEST
    );
    // Out-of-enum type → 400.
    assert_eq!(
        send(&app, example_req(None, "?type=bogus")).await.0,
        StatusCode::BAD_REQUEST
    );
    // An Accept outside the four example forms → 406.
    assert_eq!(
        send(&app, example_req(Some("text/csv"), "")).await.0,
        StatusCode::NOT_ACCEPTABLE
    );
    // Unknown template_id → 404.
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/definition/template/adl2/openEHR-EHR-COMPOSITION.nope.v9.9.9/example"
        ))
        .body(Body::empty())
        .unwrap();
    assert_eq!(send(&app, req).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn upload_invalid_source_is_422_with_rule_codes() {
    let (_pg, app) = app().await;
    // A source missing the mandatory `description` section → VARD (AOM2
    // master03 §Validity Rules). The engine rejects it; the wire renders a 422
    // `Error` object whose `validationErrors` carry the rule code.
    let invalid = "operational_template (adl_version=2.0.6; rm_release=1.1.0)\n    \
                   openEHR-EHR-COMPOSITION.t_no_desc.v1.0.0\n\n\
                   language\n    original_language = <[ISO_639-1::en]>\n\n\
                   definition\n    COMPOSITION[id1] matches { *}\n\n\
                   terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
                   [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>\n        >\n    >\n";
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(invalid))
        .unwrap();
    let (status, _headers, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
    let v: Value = serde_json::from_str(&body).unwrap();
    let errors = v
        .get("validationErrors")
        .and_then(Value::as_array)
        .expect("the Error object carries validationErrors");
    assert!(
        errors
            .iter()
            .any(|e| e.as_str().is_some_and(|s| s.contains("VARD"))),
        "the rule code VARD is reported in the 422 body: {errors:?}"
    );
}

/// ITS-REST overview `Requests_and_responses.md` §"`ETag` and Last-Modified":
/// "Both `ETag` and `Last-Modified` SHOULD be included in responses for
/// VERSION, `VERSIONED_OBJECT`, or other resources that have versioning or
/// unique state identifiers", and the `ETag` "is considered to be of weak-type
/// and should have a weakness indicator `W/` prefix". An ADL2 operational
/// template's unique state identifier is its versioned `ARCHETYPE_HRID`, so
/// the upload `201` and every retrieval carry `W/"<hrid>"` — the ADL 1.4
/// sibling's behaviour, which the ADL2 routes previously omitted.
///
/// A partially addressed template (`…/{concept_family}/1`) resolves to a
/// concrete artefact, so the `ETag` names the RESOLVED full HRID, not the
/// addressed prefix — otherwise one `ETag` would span two different versions.
#[tokio::test]
async fn etag_is_the_resolved_hrid_on_upload_and_every_get() {
    let (_pg, app) = app().await;
    let expected = format!("W/\"{HRID}\"");

    let (status, headers, body) = send(&app, upload_req(None)).await;
    assert_eq!(status, StatusCode::CREATED, "upload: {body}");
    assert_eq!(
        headers.get(header::ETAG).and_then(|v| v.to_str().ok()),
        Some(expected.as_str()),
        "overview §\"ETag and Last-Modified\": the weak ETag SHOULD accompany a \
         resource with a unique state identifier (the template HRID) — \
         headers: {headers:?}, body: {body}"
    );

    // text/plain source GET.
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/definition/template/adl2/{HRID}"))
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        headers.get(header::ETAG).and_then(|v| v.to_str().ok()),
        Some(expected.as_str()),
        "overview §\"ETag and Last-Modified\": the source GET carries the weak ETag — \
         headers: {headers:?}, body: {body}"
    );

    // application/json OperationalTemplateV2 GET — the ETag "is independent of
    // its resource serialization format" (same section).
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/definition/template/adl2/{HRID}"))
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        headers.get(header::ETAG).and_then(|v| v.to_str().ok()),
        Some(expected.as_str()),
        "overview §\"ETag and Last-Modified\": the ETag is serialization-independent — \
         headers: {headers:?}, body: {body}"
    );

    // The partial-version GET resolves to the concrete artefact; the ETag is
    // the resolved HRID, never the addressed `…/1` prefix.
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/definition/template/adl2/openEHR-EHR-COMPOSITION.t_clinical_info/1"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, headers, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(
        headers.get(header::ETAG).and_then(|v| v.to_str().ok()),
        Some(expected.as_str()),
        "the ETag names the resolved artefact — it must change when the served \
         version changes (overview §\"ETag and Last-Modified\") — \
         headers: {headers:?}, body: {body}"
    );
}

// ── POST: a non-text/plain payload type is 415, never a parse-time 400 ──────
// The operation declares `text/plain` as its single request body type
// (`operations/definition_template_adl2_upload.yaml`); a payload DECLARING
// another media type cannot be processed as it — `Resources.md` §format rules:
// "If the service cannot process the request payload as … format, it MUST
// respond with HTTP status code 415 Unsupported Media Type". Mirrors the
// ADL 1.4 sibling's guard.

#[tokio::test]
async fn adl2_upload_with_xml_content_type_is_415() {
    let (_pg, app) = app().await;
    let (status, _headers, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/definition/template/adl2"))
            .header(header::CONTENT_TYPE, "application/xml")
            .body(Body::from(source()))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE,
        "a declared non-text/plain payload type is refused before parsing: {body}"
    );
}

#[tokio::test]
async fn adl2_upload_without_content_type_is_created() {
    let (_pg, app) = app().await;
    let (status, _headers, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/definition/template/adl2"))
            .body(Body::from(source()))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "an absent Content-Type declares nothing to refuse (the header is a \
         client MAY): {body}"
    );
}

#[tokio::test]
async fn adl2_upload_with_charset_parameter_is_created() {
    let (_pg, app) = app().await;
    let (status, _headers, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/definition/template/adl2"))
            .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
            .body(Body::from(source()))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "media-type parameters do not change the declared type: {body}"
    );
}

#[tokio::test]
async fn upload_unparseable_source_is_400() {
    let (_pg, app) = app().await;
    // Content that fails the ADL2 grammar outright is *syntactically invalid
    // content* — the released 400 branch declared on the upload
    // (`responses/400.yaml`: "the request could not be parsed or is invalid
    // (e.g. ... syntactically invalid ... content)") — never the semantic 422
    // that AOM2 validation-phase failures (V-codes) carry.
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from("this is not adl2 at all {{{"))
        .unwrap();
    let (status, _headers, body) = send(&app, req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an unparseable ADL2 source is the syntactic 400 branch: {body}"
    );
}

#[tokio::test]
async fn upload_empty_source_is_400() {
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::empty())
        .unwrap();
    let (status, _headers, body) = send(&app, req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "an empty body matches no ADL2 production — syntactically invalid \
         content (responses/400.yaml): {body}"
    );
}

/// A specialised upload whose `specialise` clause names a parent the repository
/// does not hold is refused as `422` carrying VASID: without the flat parent
/// the archetype cannot validate (AOM2 master03 VASID, master08 §Phase 2; SM
/// `upload_artefact` "The artefact must validate"), and storing it unchecked
/// would let a child uploaded before its parent skip conformance for good.
#[tokio::test]
async fn upload_with_absent_parent_is_422_with_vasid() {
    let (_db, app) = app().await;
    let src = "archetype (adl_version=2.0.6; rm_release=1.1.0)\n    \
         openEHR-EHR-CLUSTER.orphan-child.v1.0.0\n\n\
         specialise\n    openEHR-EHR-CLUSTER.orphan.v1.0.0\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    CLUSTER[id1.1] matches { *}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1.1\"] = <text = <\"Child\"> description = <\"Child.\">>\n        >\n    >\n";
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(src))
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "{body}");
    assert!(
        body.contains("VASID") && body.contains("openEHR-EHR-CLUSTER.orphan.v1.0.0"),
        "the 422 names VASID and the missing parent: {body}"
    );
}

/// An upload nested past the engine's bound (`openehr_lang::nesting`) is a
/// syntactic `400` naming the bound: the parser refuses at the bound instead
/// of recursing until the thread's stack ends, which no layer could catch
/// (#3062). The ITS-REST `400` branch covers "syntactically invalid …
/// content" (`docs/specs/openehr/ITS-REST/specifications/responses/400.yaml`).
#[tokio::test]
async fn upload_nested_past_the_engine_bound_is_a_400_naming_the_bound() {
    let (_pg, app) = app().await;
    let levels = openehr_lang::nesting::MAX_NESTING_DEPTH + 2;
    let mut open = String::new();
    for k in 0..levels {
        write!(open, "CLUSTER[id{}] matches {{ items matches {{ ", k + 1).expect("a String write");
    }
    let close = " } }".repeat(levels);
    let definition = format!("{open}ELEMENT[id99999] matches {{*}}{close}");
    let hrid = "openEHR-EHR-CLUSTER.too_deep.v1.0.0";
    let body = format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    {definition}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n            \
         [\"id1\"] = <text = <\"Root\"> description = <\"Root.\">>\n        >\n    >\n"
    );
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl2"))
        .header(header::CONTENT_TYPE, "text/plain")
        .body(Body::from(body))
        .unwrap();
    let (status, _, text) = send(&app, req).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "body: {text}");
    let json: Value = serde_json::from_str(&text).expect("an error document");
    let message = json["message"].as_str().expect("message");
    assert!(
        message.contains("SUNK"),
        "the S-code bucket is named: {message}"
    );
    assert!(
        message.contains(&format!(
            "exceeds the limit of {} levels",
            openehr_lang::nesting::MAX_NESTING_DEPTH
        )),
        "the bound is named: {message}"
    );
}
