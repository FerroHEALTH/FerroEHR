// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the DEMOGRAPHIC group: the `demographic` routes
//! served through the real [`DemographicService`] over a real `PostgreSQL`, with
//! `ETag`/`Location`/`Prefer` and the deleted-read→`204` and precondition→`412`
//! behaviour mirroring the EHR group.
//!
//! The former `Mock` backend served fixed party bodies with a hard-coded
//! `PARTY_OVID`; the real service assigns each version its own
//! `OBJECT_VERSION_ID`, so the tests create real parties through the wire and
//! read the server-assigned `version_uid` back from the `ETag` — the invariant
//! assertions (weak-`ETag` present, `Location` = `{base}/demographic/{kind}/{uid}`
//! consistent with that `ETag` **on writes only**, body `_type`, status codes).
//!
//! Header discipline (ITS-REST overview `Requests_and_responses.md`): `Location`
//! rides create/update writes alone — §Location bars it as "an alternate
//! representation of an existing resource" and §"Deprecated headers" deprecates
//! it on `GET` and `DELETE`; reads, deletes and `4xx` responses identify the
//! version through the weak `ETag` (+ `Last-Modified` where known).
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
use serde_json::{Value, json};
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::server::ServerConfig;
use ferroehr_rest::config::AppConfig;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// A spec-valid PERSON body (the shape `service_demographic.rs` commits).
///
/// Shared with the credential-separation suite, which commits a party to prove
/// the demographic domain serves on its own credential.
pub(crate) fn person_body() -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "Jane Doe" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": "Doe" }
                }]
            }
        }]
    })
}

/// A spec-valid ROLE body (needs a `performer` `PARTY_REF`; no `capabilities`).
fn role_body() -> Value {
    json!({
        "_type": "ROLE",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ROLE.role.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": "clinician" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "r" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": []
            }
        }],
        "performer": {
            "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": "cccccccc-cccc-4ccc-8ccc-cccccccccccc" }
        }
    })
}

fn relationship_body() -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "name": { "_type": "DV_TEXT", "value": "parent-of" },
        "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "aaaaaaaa-aaaa-4aaa-8aaa-aaaaaaaaaaaa" } },
        "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                    "id": { "_type": "HIER_OBJECT_ID", "value": "bbbbbbbb-bbbb-4bbb-8bbb-bbbbbbbbbbbb" } }
    })
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

/// A canonical-JSON `GET` against the composed router.
async fn get_json(app: &Router, uri: String) -> (StatusCode, header::HeaderMap, String) {
    let req = Request::builder()
        .method("GET")
        .uri(uri)
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    send(app, req).await
}

fn etag(h: &header::HeaderMap) -> Option<&str> {
    h.get(header::ETAG).and_then(|v| v.to_str().ok())
}

fn location(h: &header::HeaderMap) -> Option<&str> {
    h.get(header::LOCATION).and_then(|v| v.to_str().ok())
}

/// The bare uid inside a weak `ETag` (`W/"{uid}"`).
fn etag_uid(h: &header::HeaderMap) -> String {
    etag(h)
        .expect("ETag present")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

/// Create a party of `kind` (segment) and return its `version_uid` (`OVID`).
async fn create(app: &Router, seg: &str, body: &Value) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/{seg}"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let (status, h, _b) = send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "create {seg}");
    etag_uid(&h)
}

/// The bare versioned-object uuid from an `OBJECT_VERSION_ID`.
fn vo_of(ovid: &str) -> &str {
    ovid.split("::").next().expect("vo uuid")
}

#[tokio::test]
async fn person_create_default_is_minimal_with_headers() {
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/person"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    // 201 default (return=minimal): headers only, no body.
    assert_eq!(status, StatusCode::CREATED);
    let uid = etag_uid(&h);
    assert!(uid.ends_with("::1"), "first version_uid, got {uid}");
    assert_eq!(etag(&h), Some(format!("W/\"{uid}\"").as_str()));
    assert_eq!(
        location(&h),
        Some(format!("{BASE}/demographic/person/{uid}").as_str())
    );
    assert!(body.is_empty(), "minimal create has no body, got {body:?}");
}

#[tokio::test]
async fn person_create_representation_returns_body() {
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/person"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Prefer", "return=representation")
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::CREATED);
    let uid = etag_uid(&h);
    assert_eq!(etag(&h), Some(format!("W/\"{uid}\"").as_str()));
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["_type"], "PERSON");
}

/// A `GET` carries the weak `ETag` and **no** `Location`: ITS-REST overview
/// `Requests_and_responses.md` §Location — "It MUST NOT be used to indicate an
/// alternate representation of an existing resource (e.g. via `GET` method)" and
/// "MUST ONLY be used for resource creation (e.g., `201 Created`) or redirect
/// responses"; §"Deprecated headers" deprecates `Location` on `GET`.
#[tokio::test]
async fn person_get_sets_etag_and_no_location() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "person", &person_body()).await;
    let vo = vo_of(&ovid);

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/demographic/person/{vo}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(etag(&h), Some(format!("W/\"{ovid}\"").as_str()));
    assert_eq!(
        location(&h),
        None,
        "ITS-REST overview §Location: Location MUST NOT indicate an alternate \
         representation of an existing resource (GET)"
    );
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["_type"], "PERSON");
}

#[tokio::test]
async fn deleted_person_read_is_204() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "person", &person_body()).await;
    let vo = vo_of(&ovid);

    // Delete the party (preceding version in the path).
    let del = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/demographic/person/{ovid}"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, _b) = send(&app, del).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // Reading the (now deleted) current version → Null body → 204.
    let get = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/demographic/person/{vo}"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, get).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty());
}

/// A `204` delete carries the deleted version's weak `ETag` and **no**
/// `Location`: ITS-REST overview §"Deprecated headers" — "the `Location`
/// response header was deprecated from responses of `DELETE` methods";
/// §Location scopes it to creation/redirect responses.
#[tokio::test]
async fn person_delete_is_204_with_etag_and_no_location() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "person", &person_body()).await;

    let req = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/demographic/person/{ovid}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    // ETag of the deleted version (a new version_uid).
    let deleted = etag_uid(&h);
    assert_eq!(etag(&h), Some(format!("W/\"{deleted}\"").as_str()));
    assert_eq!(
        location(&h),
        None,
        "ITS-REST overview §Deprecated headers: Location is deprecated on DELETE responses"
    );
    assert!(body.is_empty());
}

/// The versioned-object-uid delete shape (ECC-DEM-005 family): the path is the
/// bare `HIER_OBJECT_ID` and the preceding version is carried by `If-Match`.
#[tokio::test]
async fn person_delete_by_versioned_uid_with_if_match_is_204() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "person", &person_body()).await;
    let vo = vo_of(&ovid);

    let req = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/demographic/person/{vo}"))
        .header(header::IF_MATCH, format!("\"{ovid}\""))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::NO_CONTENT);
    assert!(body.is_empty());
}

/// A stale `If-Match` is `412` echoing the latest `version_uid` in `ETag`
/// (ITS-REST overview §"If-Match and accidental overwrites": on a false
/// condition the service "MUST respond with HTTP status code `412 Precondition
/// Failed`, and SHOULD return also latest `version_uid` in the `ETag` response
/// headers") — and no `Location`, which §Location scopes to creation/redirect.
#[tokio::test]
async fn stale_update_is_412_with_latest_etag_and_no_location() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "person", &person_body()).await;
    let vo = vo_of(&ovid);
    // A syntactically valid but stale OBJECT_VERSION_ID (same VO, wrong version).
    let stale = format!("{vo}::ferroehr.local::9");

    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/demographic/person/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{stale}\""))
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, _body) = send(&app, req).await;

    // Precondition failure → 412, echoing the latest version_uid.
    assert_eq!(status, StatusCode::PRECONDITION_FAILED);
    assert_eq!(etag(&h), Some(format!("W/\"{ovid}\"").as_str()));
    assert_eq!(
        location(&h),
        None,
        "ITS-REST overview §Location: no Location on an error response"
    );
}

/// The server emits weak `ETag`s (`W/"…"`, overview §"`ETag` and
/// Last-Modified"), so a client echoing that exact value back as `If-Match`
/// MUST satisfy the precondition. The bare quoted form stays supported
/// (§"Deprecated headers": implementations "MAY still support it"), and the
/// unquoted value is accepted for the same reason.
#[tokio::test]
async fn update_accepts_weak_bare_quoted_and_unquoted_if_match() {
    let (_pg, app) = app().await;

    for shape in ["weak", "quoted", "unquoted"] {
        let ovid = create(&app, "person", &person_body()).await;
        let vo = vo_of(&ovid).to_owned();
        let if_match = match shape {
            "weak" => format!("W/\"{ovid}\""),
            "quoted" => format!("\"{ovid}\""),
            _ => ovid.clone(),
        };

        let req = Request::builder()
            .method("PUT")
            .uri(format!("{BASE}/demographic/person/{vo}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_MATCH, if_match.as_str())
            .body(Body::from(person_body().to_string()))
            .unwrap();
        let (status, h, body) = send(&app, req).await;

        assert_eq!(
            status,
            StatusCode::NO_CONTENT,
            "If-Match {if_match:?} ({shape} form) must satisfy the precondition \
             (ITS-REST overview §ETag and Last-Modified + §Deprecated headers): {body}"
        );
        let v2 = etag_uid(&h);
        assert!(v2.ends_with("::2"), "second version committed, got {v2}");
    }
}

/// `creating_system_id` is a composite identifier: BASE `base_types`
/// `master05-identification_package.adoc` §"Composite Identifiers and Case" —
/// two identifiers "identical apart from case … identify the same thing". A
/// case-variant `If-Match` therefore names the current version and must NOT
/// raise a spurious `412`.
#[tokio::test]
async fn update_if_match_compares_case_insensitively() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "person", &person_body()).await;
    let vo = vo_of(&ovid).to_owned();
    let upper = ovid.to_uppercase();
    assert_ne!(upper, ovid, "the fixture uid must have case to flip");

    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/demographic/person/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("W/\"{upper}\""))
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, _h, body) = send(&app, req).await;

    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "BASE master05 §Composite Identifiers and Case: a case-variant \
         OBJECT_VERSION_ID names the same version: {body}"
    );
}

/// A syntactically invalid `If-Match` is a client error (`400`), never an
/// ignored precondition — ITS-REST overview §"If-Match and accidental
/// overwrites" requires a received `If-Match` to be honoured, so a value that
/// cannot be evaluated must not run as if none was sent.
#[tokio::test]
async fn malformed_if_match_is_400() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "person", &person_body()).await;
    let vo = vo_of(&ovid).to_owned();

    for bad in ["W/\"not-a-version-id\"", "\"\"", "\"a::b::c::3\""] {
        let req = Request::builder()
            .method("PUT")
            .uri(format!("{BASE}/demographic/person/{vo}"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::IF_MATCH, bad)
            .body(Body::from(person_body().to_string()))
            .unwrap();
        let (status, _h, body) = send(&app, req).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "If-Match {bad:?} is malformed and must be rejected, not skipped: {body}"
        );
    }
}

/// A `DELETE` addressed by the bare versioned-object uid accepts the weak
/// `If-Match` form the server emits (same normalization seam as `PUT`).
#[tokio::test]
async fn delete_accepts_the_weak_if_match_form() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "person", &person_body()).await;
    let vo = vo_of(&ovid).to_owned();

    let req = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/demographic/person/{vo}"))
        .header(header::IF_MATCH, format!("W/\"{ovid}\""))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a weak-form If-Match must satisfy the delete precondition: {body}"
    );
    assert!(etag(&h).is_some(), "the deleted version's ETag is echoed");
    assert_eq!(
        location(&h),
        None,
        "ITS-REST overview §Deprecated headers: Location is deprecated on DELETE responses"
    );
}

/// The `VERSIONED_PARTY` reads are `VERSION`/`VERSIONED_OBJECT` responses, so
/// each SHOULD carry `ETag` and `Last-Modified` (ITS-REST overview §"`ETag` and
/// Last-Modified": both "SHOULD be included in responses for `VERSION`,
/// `VERSIONED_OBJECT`, or other resources that have versioning or unique state
/// identifiers"; the `ETag` is "usually taken from e.g.
/// `VERSIONED_OBJECT.uid.value`, `VERSION.uid.value`") — and never `Location`.
/// The demographic direct routes honour BOTH halves of the committal merge —
/// the `UPDATE_AUDIT` attributes AND the VERSION `lifecycle_state`.
///
/// ITS-REST overview `Requests_and_responses.md` §"openehr-version and
/// openehr-audit-details" makes the merge a MUST on the direct commits:
/// "services MUST accept `openehr-version` and `openehr-audit-details` custom
/// request headers", and "whatever is provided it MUST be merged with the
/// default VERSION and `VERSION.audit_details` attributes on commit runtime" —
/// the VERSION attributes named FIRST. A demographic party's `UPDATE_VERSION`
/// envelope never travels in the body, so these headers are its only committal
/// channel; before this, the direct routes threaded the audit half alone and a
/// `553|incomplete|` party was reachable only through a CONTRIBUTION.
///
/// `553|incomplete|` is the openEHR `version_lifecycle_state` code for content
/// still being authored (RM common `master06-change_control_package.adoc`
/// §Version Lifecycle / §Incomplete Content).
#[tokio::test]
async fn direct_party_write_honours_the_openehr_version_lifecycle_state() {
    let (_pg, app) = app().await;

    // CREATE with the header's lifecycle half.
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/person"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("openehr-version", "lifecycle_state.code_string=\"553\"")
        .header(
            "openehr-audit-details",
            "description.value=\"still drafting\"",
        )
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "553 create: {body}");
    let ovid = etag_uid(&h);
    let vo = vo_of(&ovid).to_owned();

    // The committed version carries the client's lifecycle state, and its audit
    // the client's description — both halves of the one merge.
    let (status, _h, body) = get_json(
        &app,
        format!("{BASE}/demographic/versioned_party/{vo}/version/{ovid}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "version read: {body}");
    let version: Value = serde_json::from_str(&body).expect("ORIGINAL_VERSION json");
    assert_eq!(
        version["lifecycle_state"]["defining_code"]["code_string"], "553",
        "the openehr-version lifecycle half must reach the commit: {body}"
    );
    assert_eq!(
        version["commit_audit"]["description"]["value"], "still drafting",
        "the openehr-audit-details half still merges: {body}"
    );

    // UPDATE back to `532|complete|` through the same channel.
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/demographic/person/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{ovid}\""))
        .header("openehr-version", "lifecycle_state.code_string=\"532\"")
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    // No `Prefer: return=representation` → 204 with the identifying headers.
    assert_eq!(status, StatusCode::NO_CONTENT, "532 update: {body}");
    let ovid_v2 = etag_uid(&h);
    let (status, _h, body) = get_json(
        &app,
        format!("{BASE}/demographic/versioned_party/{vo}/version/{ovid_v2}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "v2 read: {body}");
    let version: Value = serde_json::from_str(&body).expect("ORIGINAL_VERSION json");
    assert_eq!(
        version["lifecycle_state"]["defining_code"]["code_string"], "532",
        "the update route honours the lifecycle half too: {body}"
    );
}

#[tokio::test]
async fn versioned_party_reads_emit_versioning_headers() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "person", &person_body()).await;
    let vo = vo_of(&ovid).to_owned();

    // The VERSIONED_PARTY container: ETag = VERSIONED_OBJECT.uid.value.
    let (status, h, body) =
        get_json(&app, format!("{BASE}/demographic/versioned_party/{vo}")).await;
    assert_eq!(status, StatusCode::OK, "container read: {body}");
    assert_eq!(etag(&h), Some(format!("W/\"{vo}\"").as_str()));
    assert_eq!(
        location(&h),
        None,
        "ITS-REST overview §Location: not on GET"
    );

    // The REVISION_HISTORY: ETag = the addressed VERSIONED_OBJECT uid,
    // Last-Modified = the most recent item's commit audit.
    let (status, h, body) = get_json(
        &app,
        format!("{BASE}/demographic/versioned_party/{vo}/revision_history"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "revision history: {body}");
    assert_eq!(etag(&h), Some(format!("W/\"{vo}\"").as_str()));
    assert!(
        h.contains_key(header::LAST_MODIFIED),
        "Last-Modified from REVISION_HISTORY.items.last().audits[0].time_committed"
    );
    assert_eq!(
        location(&h),
        None,
        "ITS-REST overview §Location: not on GET"
    );

    // A version read: ETag = VERSION.uid.value, Last-Modified = its commit audit.
    let (status, h, body) = get_json(
        &app,
        format!("{BASE}/demographic/versioned_party/{vo}/version/{ovid}"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "version read: {body}");
    assert_eq!(etag(&h), Some(format!("W/\"{ovid}\"").as_str()));
    assert!(
        h.contains_key(header::LAST_MODIFIED),
        "Last-Modified from ORIGINAL_VERSION.commit_audit.time_committed"
    );
    assert_eq!(
        location(&h),
        None,
        "ITS-REST overview §Location: not on GET"
    );

    // The at-time version read (no version_at_time = latest).
    let (status, h, body) = get_json(
        &app,
        format!("{BASE}/demographic/versioned_party/{vo}/version"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "version at time: {body}");
    assert_eq!(etag(&h), Some(format!("W/\"{ovid}\"").as_str()));
    assert_eq!(
        location(&h),
        None,
        "ITS-REST overview §Location: not on GET"
    );
}

/// `Resources.md` §"Datetime format": a datetime query parameter MUST be
/// extended ISO 8601, and "Timezone SHOULD be only supplied when needed,
/// otherwise the local timezone is assumed" — so an offset-LESS extended
/// datetime is a well-formed `version_at_time` on the DEMOGRAPHIC at-time read
/// too, resolved in the server's local timezone rather than rejected `400`.
/// The router under test runs in this process, so its "local timezone" is this
/// process's system zone and the assertion is independent of what that zone is.
#[tokio::test]
async fn version_at_time_without_offset_resolves_in_the_local_timezone() {
    let (pg, app) = app().await;
    let v1 = create(&app, "person", &person_body()).await;
    let vo = vo_of(&v1).to_owned();

    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/demographic/person/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("W/\"{v1}\""))
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "second version: {body}");

    // The probe instant is v1's OWN commit instant, read back from the database
    // rather than sampled from a clock (#3119). `committed_at` is stamped by
    // PostgreSQL, whose container clock need not agree with this process's, and
    // the margins this test used to sleep around were eaten under full-suite
    // load. A version is in force AT its own commit instant, so the version
    // extant at exactly that instant is v1 by construction, with no margin to
    // lose.
    let between: jiff_sqlx::Timestamp = sqlx::query_scalar(
        "SELECT committed_at FROM party.version \
         WHERE vo_id = $1 AND sys_version = 1",
    )
    .bind(vo.parse::<uuid::Uuid>().expect("the versioned object id"))
    .fetch_one(&pg.pool())
    .await
    .expect("v1's stored commit instant");
    let between = between.to_jiff();

    // The same instant written WITHOUT an offset: its civil rendering in the
    // server's local timezone (`YYYY-MM-DDThh:mm:ss.sss`, no `Z`, no `±hh:mm`).
    let offset_less = between
        .to_zoned(jiff::tz::TimeZone::system())
        .datetime()
        .to_string();
    assert!(
        !offset_less.ends_with('Z') && !offset_less.contains('+'),
        "the probe value must carry no timezone: {offset_less}"
    );

    let (status, h, body) = get_json(
        &app,
        format!("{BASE}/demographic/versioned_party/{vo}/version?version_at_time={offset_less}"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an offset-less extended datetime is a valid version_at_time: {body}"
    );
    assert_eq!(
        etag(&h),
        Some(format!("W/\"{v1}\"").as_str()),
        "the offset-less rendering names the same instant as its offset-carrying form, \
         so the version extant at it is still v1"
    );
}

/// The `PARTY_RELATIONSHIP` extension mirrors the party envelope: its versioned
/// reads carry the same versioning headers and no `Location`.
#[tokio::test]
async fn versioned_party_relationship_reads_emit_versioning_headers() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "party_relationship", &relationship_body()).await;
    let vo = vo_of(&ovid).to_owned();

    for (uri, expected_etag) in [
        (
            format!("{BASE}/demographic/versioned_party_relationship/{vo}"),
            vo.clone(),
        ),
        (
            format!("{BASE}/demographic/versioned_party_relationship/{vo}/revision_history"),
            vo.clone(),
        ),
        (
            format!("{BASE}/demographic/versioned_party_relationship/{vo}/version/{ovid}"),
            ovid.clone(),
        ),
    ] {
        let req = Request::builder()
            .method("GET")
            .uri(&uri)
            .header(header::ACCEPT, "application/json")
            .body(Body::empty())
            .unwrap();
        let (status, h, body) = send(&app, req).await;
        assert_eq!(status, StatusCode::OK, "{uri}: {body}");
        assert_eq!(
            etag(&h),
            Some(format!("W/\"{expected_etag}\"").as_str()),
            "{uri} carries the versioned-resource ETag"
        );
        assert_eq!(
            location(&h),
            None,
            "{uri}: ITS-REST overview §Location — no Location on a GET"
        );
    }
}

#[tokio::test]
async fn role_create_uses_role_segment() {
    // The 5× kind fan-out routes each kind to its own segment.
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/role"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(role_body().to_string()))
        .unwrap();
    let (status, h, _body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED);
    let uid = etag_uid(&h);
    assert_eq!(
        location(&h),
        Some(format!("{BASE}/demographic/role/{uid}").as_str())
    );
}

#[tokio::test]
async fn party_relationship_create_is_mounted_with_headers() {
    // The our-own-design PARTY_RELATIONSHIP extension route is mounted and
    // reaches the seam (a create returns 201 + ETag/Location on the
    // /demographic/party_relationship segment; an unmounted route would 404).
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/party_relationship"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Prefer", "return=representation")
        .body(Body::from(relationship_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::CREATED);
    let uid = etag_uid(&h);
    assert_eq!(etag(&h), Some(format!("W/\"{uid}\"").as_str()));
    assert_eq!(
        location(&h),
        Some(format!("{BASE}/demographic/party_relationship/{uid}").as_str())
    );
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["_type"], "PARTY_RELATIONSHIP");
}

#[tokio::test]
async fn party_relationship_get_is_mounted() {
    let (_pg, app) = app().await;
    let ovid = create(&app, "party_relationship", &relationship_body()).await;
    let vo = vo_of(&ovid);

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/demographic/party_relationship/{vo}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(etag(&h), Some(format!("W/\"{ovid}\"").as_str()));
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["_type"], "PARTY_RELATIONSHIP");
}

#[tokio::test]
async fn versioned_party_relationship_is_mounted() {
    // RE-TARGET: the old Mock returned a blanket `501` (route mounted, unbuilt).
    // The versioned-party-relationship read is a real implementation now, so a
    // created relationship reads back its VERSIONED_PARTY_RELATIONSHIP → `200`
    // (an unmounted path would 404), which still proves the route is mounted.
    let (_pg, app) = app().await;
    let ovid = create(&app, "party_relationship", &relationship_body()).await;
    let vo = vo_of(&ovid);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/demographic/versioned_party_relationship/{vo}"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "body: {body}");
    let v: Value = serde_json::from_str(&body).expect("json body");
    // The ehr-less relationship version spine serves the base RM
    // `VERSIONED_OBJECT` — no `VERSIONED_PARTY_RELATIONSHIP` class exists in
    // the RM demographic package — and its mandatory `owner_id` names the
    // SERVING SYSTEM: an `OBJECT_REF` `{namespace: local, type: SYSTEM}` over a
    // `HIER_OBJECT_ID`, per the released `VersionedParty` example (vendored
    // ITS-REST OAS `demographic-codegen.openapi.yaml`). It is NOT a
    // self-reference to the relationship's own container, which would merely
    // duplicate the sibling `uid`.
    assert_eq!(v["_type"], "VERSIONED_OBJECT");
    assert_eq!(v["owner_id"]["_type"], "OBJECT_REF");
    assert_eq!(v["owner_id"]["namespace"], "local");
    assert_eq!(v["owner_id"]["type"], "SYSTEM");
    assert_eq!(v["owner_id"]["id"]["_type"], "HIER_OBJECT_ID");
    assert_ne!(
        v["owner_id"]["id"]["value"], v["uid"]["value"],
        "owner_id must name the owning system, never re-state the container uid"
    );
}

/// The ITS-REST overview committal-header merge requirement holds on the
/// demographic wire exactly as on the EHR APIs: attributes supplied via
/// `openEHR-AUDIT_DETAILS.*` request headers "MUST be merged with the default
/// `VERSION` and `VERSION.audit_details` attributes on commit runtime". The
/// persisted `ORIGINAL_VERSION`'s `commit_audit` must reflect the caller's
/// description and committer (`change_type` stays operation-owned).
#[tokio::test]
async fn demographic_committal_headers_merge_into_the_commit() {
    let (_pg, app) = app().await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/person"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            "openEHR-AUDIT_DETAILS.description",
            "value=\"Registered at intake\"",
        )
        .header(
            "openEHR-AUDIT_DETAILS.committer",
            "name=\"John Doe\", external_ref.id=\"BC8132EA-8F4A-11E7-BB31-BE2E44B06B34\", \
             external_ref.namespace=\"demographic\", external_ref.type=\"PERSON\"",
        )
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, _b) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED);
    let v1 = etag_uid(&h);
    let vo = vo_of(&v1).to_owned();

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/demographic/versioned_party/{vo}/version/{v1}"
        ))
        .header(header::ACCEPT, "application/json")
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "version read: {body}");
    let ver: Value = serde_json::from_str(&body).expect("original_version json");
    assert_eq!(ver["_type"], "ORIGINAL_VERSION");
    let audit = &ver["commit_audit"];
    assert_eq!(
        audit["description"]["value"], "Registered at intake",
        "openEHR-AUDIT_DETAILS.description merged: {ver}"
    );
    assert_eq!(
        audit["committer"]["name"], "John Doe",
        "openEHR-AUDIT_DETAILS.committer merged"
    );
    assert_eq!(
        audit["change_type"]["defining_code"]["code_string"], "249",
        "change_type stays operation-owned (creation)"
    );
}

// ── Prefer / Preference-Applied (overview §Representation details ────────────
//    negotiation, §"Prefer only identifier") ────────────────────────────────

fn preference_applied(h: &header::HeaderMap) -> Option<String> {
    h.get("preference-applied")
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// Demographic writes go through the same `Prefer` seam as the EHR group:
/// "The service MAY include a `Preference-Applied` header in the response …
/// to indicate that the client's preference has been honored", and "if no
/// `Prefer` header is provided, the default behavior is assumed to be
/// `return=minimal`".
#[tokio::test]
async fn demographic_writes_declare_the_applied_preference() {
    let (_pg, app) = app().await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/person"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "person create: {body}");
    assert_eq!(preference_applied(&h).as_deref(), Some("return=minimal"));

    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/person"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Prefer", "return=representation")
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "person create: {body}");
    assert_eq!(
        preference_applied(&h).as_deref(),
        Some("return=representation")
    );

    // The party ITEM_TAG collection write declares its outcome too.
    let uid = create(&app, "person", &person_body()).await;
    let vo = vo_of(&uid).to_owned();
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/demographic/person/{vo}/tags"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!([{ "key": "reviewed", "value": "true" }]).to_string(),
        ))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "person tags update: {body}");
    assert_eq!(preference_applied(&h).as_deref(), Some("return=minimal"));
}

/// `Prefer: return=identifier` on a demographic create: "the status will be
/// `201 Created` or `200 OK`, never `204 No Content`" and "the response body
/// … will be a single JSON object with a single `uid` attribute"
/// (§"Prefer only identifier").
#[tokio::test]
async fn person_create_identifier_returns_the_uid_body() {
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/person"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Prefer", "return=identifier")
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(preference_applied(&h).as_deref(), Some("return=identifier"));
    let uid = etag_uid(&h);
    let v: Value = serde_json::from_str(&body).expect("json identifier body");
    assert_eq!(
        v,
        serde_json::json!({ "uid": uid }),
        "overview §\"Prefer only identifier\": a single JSON object with a single uid attribute"
    );
}

// ── group-12 close triage: the two header-echo defects ───────────────────────

/// The stale-version DELETE answers `409` AND echoes the latest `version_uid`
/// in `ETag` (`responses/409_PERSON_with_uid_based_id.yaml`: "Returns also
/// latest `version_uid` in the `ETag` header").
#[tokio::test]
async fn stale_delete_conflict_echoes_latest_version_etag() {
    let (_pg, app) = app().await;
    let v1 = create(&app, "person", &person_body()).await;
    // Supersede v1 with an update.
    let put = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/demographic/person/{}", vo_of(&v1)))
        .header(header::CONTENT_TYPE, "application/json")
        .header("If-Match", format!("\"{v1}\""))
        .body(Body::from(person_body().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, put).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let v2 = etag_uid(&h);
    // Delete at the superseded uid → 409 + the latest version's ETag.
    let del = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/demographic/person/{v1}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, del).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(
        etag_uid(&h),
        v2,
        "the 409 returns the latest version_uid in ETag \
         (409_PERSON_with_uid_based_id.yaml)"
    );
}

/// The demographic CONTRIBUTION read carries the weak `ETag` (the contribution
/// uid — the same identity the 201's `ETag` carries) and `Last-Modified` from
/// `audit.time_committed`, mirroring the EHR sibling's adjudicated
/// reading of the overview §"`ETag` and Last-Modified" SHOULD.
#[tokio::test]
async fn demographic_contribution_get_carries_etag_and_last_modified() {
    let (_pg, app) = app().await;
    let commit = serde_json::json!({
        "versions": [{
            "data": person_body(),
            "lifecycle_state": {
                "_type": "DV_CODED_TEXT",
                "value": "complete",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "532" }
            },
            "commit_audit": {
                "change_type": {
                    "_type": "DV_CODED_TEXT",
                    "value": "creation",
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                        "code_string": "249" }
                },
                "committer": { "_type": "PARTY_IDENTIFIED", "name": "committer" }
            }
        }],
        "audit": {
            "change_type": {
                "_type": "DV_CODED_TEXT",
                "value": "creation",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "249" }
            },
            "committer": { "_type": "PARTY_IDENTIFIED", "name": "committer" }
        }
    });
    let post = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/demographic/contribution"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(commit.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, post).await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let cid = etag_uid(&h);
    let (status, h, body) = get_json(&app, format!("{BASE}/demographic/contribution/{cid}")).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(etag_uid(&h), cid, "ETag = the contribution uid");
    assert!(
        h.get(header::LAST_MODIFIED).is_some(),
        "Last-Modified from audit.time_committed"
    );
    assert!(location(&h).is_none(), "no Location on a GET");
}
