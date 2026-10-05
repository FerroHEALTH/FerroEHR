// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the DIRECTORY (FOLDER) API group, driven through
//! the assembled router over a **real** `FerroEhrService` on a real
//! `PostgreSQL` 18 (auth disabled), via `tower::ServiceExt::oneshot` in-process
//! requests.
//!
//! The oracle is the vendored ITS-REST contract + the CNF Platform Conformance
//! Test Schedule (never memory, never `EHRbase` behaviour):
//!
//! * operations:
//!   `docs/specs/openehr/ITS-REST/specifications/operations/directory_{create,
//!   update,delete,get_at_time,get_by_version_id}.yaml`;
//! * responses:
//!   `.../responses/{201_directory,200_directory_updated,204_version_updated,
//!   204_deleted,204_deleted_at_time,200_FOLDER_retrieved,412_directory,
//!   404_unknown_ehr_id,404_directory_unknown_ehr_id_or_no_version_at_time_or_no_path,
//!   404_directory_unknown_ehr_id_or_no_version_uid_or_no_path}.yaml`;
//! * overview:
//!   `.../specifications/docs/overview/Requests_and_responses.md`
//!   (§"`ETag` and Last-Modified", §Location, §"If-Match and accidental
//!   overwrites", §Prefer / `Preference-Applied`);
//! * CNF: `docs/specs/openehr/CNF/docs/platform_test_schedule/
//!   master09-func_tc_ehr_directory.adoc`
//!   (E.2 create-when-directory-exists, H.2/I.1 update/delete-on-empty-EHR,
//!   D.3 the `emergency/episode-x` path structure);
//! * RM FOLDER `uid`:
//!   `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.folder.adoc`;
//! * RM FOLDER sibling-name paths: RM ehr `master04-ehr_package.adoc` §Folders.
//!
//! Every assertion is strict-spec: where the implementation disagrees the test
//! is meant to FAIL (the code is fixed, never the test — testing.md).
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http::header::HeaderName;
use http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use tower::ServiceExt;
use uuid::Uuid;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
/// A syntactically valid EHR id that is never created — the "unknown" probe.
const OTHER_EHR: &str = "11111111-2222-3333-4444-555555555555";
const FOLDER_ARCHETYPE: &str = "openEHR-EHR-FOLDER.directory.v1";

// ── request/response plumbing ────────────────────────────────────────────────

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, HeaderMap, String) {
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

fn header_str(h: &HeaderMap, name: HeaderName) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

fn raw_header(h: &HeaderMap, name: &'static str) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

/// The version uid carried by the `ETag`: the weak `W/"<uid>"` form
/// (overview §"`ETag` and Last-Modified" — the `W/` weakness indicator is a
/// MUST), stripped to the bare `OBJECT_VERSION_ID`.
fn etag_uid(h: &HeaderMap) -> String {
    let raw = header_str(h, header::ETAG).expect("ETag present");
    assert!(
        raw.starts_with("W/\""),
        "ETag MUST carry the weakness indicator W/ (overview §ETag and Last-Modified): {raw:?}"
    );
    raw.trim_start_matches("W/").trim_matches('"').to_owned()
}

fn get(uri: String, accept: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(a) = accept {
        b = b.header(header::ACCEPT, a);
    }
    b.body(Body::empty()).unwrap()
}

// ── FOLDER builders (canonical openEHR JSON) ─────────────────────────────────

fn dv_text(value: &str) -> Value {
    json!({ "_type": "DV_TEXT", "value": value })
}

/// A FOLDER `items` member — an `OBJECT_REF` to a COMPOSITION (RM ehr master04
/// §Folders: Folder structures hold references to `VERSIONED_OBJECT`s, never
/// content by value).
///
/// NOTE: the namespace is a FOREIGN system (register AMB-211, issue #1951): a
/// reference claiming THIS system must resolve, so a fabricated id rides a
/// foreign namespace — this suite's subject is the wire mechanics, not
/// referential validity (covered by `directory_item_refs` + the CNF twins).
fn composition_ref(id: &str) -> Value {
    json!({
        "_type": "OBJECT_REF",
        "namespace": "another.system.example.org",
        "type": "COMPOSITION",
        "id": { "_type": "HIER_OBJECT_ID", "value": id }
    })
}

/// A canonical FOLDER with a name, nested `folders`, and no items.
#[expect(
    clippy::needless_pass_by_value,
    reason = "a fixture builder: taking the Vec by value keeps the nested literals \
              at the call sites readable"
)]
fn folder_json(name: &str, subfolders: Vec<Value>) -> Value {
    json!({
        "_type": "FOLDER",
        "name": dv_text(name),
        "archetype_node_id": FOLDER_ARCHETYPE,
        "folders": subfolders
    })
}

/// A canonical FOLDER carrying `items` (COMPOSITION `OBJECT_REF`s).
#[expect(
    clippy::needless_pass_by_value,
    reason = "a fixture builder: taking the Vec by value keeps the nested literals \
              at the call sites readable"
)]
fn folder_with_items(name: &str, subfolders: Vec<Value>, item_ids: &[&str]) -> Value {
    json!({
        "_type": "FOLDER",
        "name": dv_text(name),
        "archetype_node_id": FOLDER_ARCHETYPE,
        "folders": subfolders,
        "items": item_ids.iter().copied().map(composition_ref).collect::<Vec<_>>()
    })
}

// ── setup helpers ────────────────────────────────────────────────────────────

/// Create a real EHR through the wire; return its server-assigned id (parsed
/// from the create `ETag`, which for `/ehr` is the `ehr_id`).
async fn create_ehr(app: &Router) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "create ehr: {body}");
    etag_uid(&h)
}

/// `POST /ehr/{id}/directory` with the given folder + optional `Prefer`; return
/// the full response.
async fn create_directory(
    app: &Router,
    ehr: &str,
    folder: &Value,
    prefer: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut b = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr}/directory"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(p) = prefer {
        b = b.header("Prefer", p);
    }
    send(app, b.body(Body::from(folder.to_string())).unwrap()).await
}

/// `PUT /ehr/{id}/directory` with an `If-Match` value verbatim + optional
/// `Prefer`.
async fn update_directory(
    app: &Router,
    ehr: &str,
    folder: &Value,
    if_match: Option<&str>,
    prefer: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut b = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr}/directory"))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(m) = if_match {
        b = b.header(header::IF_MATCH, m);
    }
    if let Some(p) = prefer {
        b = b.header("Prefer", p);
    }
    send(app, b.body(Body::from(folder.to_string())).unwrap()).await
}

/// `DELETE /ehr/{id}/directory` with an `If-Match` value verbatim.
async fn delete_directory(
    app: &Router,
    ehr: &str,
    if_match: Option<&str>,
) -> (StatusCode, HeaderMap, String) {
    let mut b = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/ehr/{ehr}/directory"));
    if let Some(m) = if_match {
        b = b.header(header::IF_MATCH, m);
    }
    send(app, b.body(Body::empty()).unwrap()).await
}

/// The bare-quoted `If-Match` form (`"<uid>"`).
fn quoted(uid: &str) -> String {
    format!("\"{uid}\"")
}

/// The weak `If-Match`/`ETag` form (`W/"<uid>"`).
fn weak(uid: &str) -> String {
    format!("W/\"{uid}\"")
}

// ── 1. create: 201, weak ETag, Location, empty body when Prefer absent ───────

/// `directory_create.yaml` → `201_directory.yaml`: `201 Created`, `ETag`
/// (weak `W/"…"`, overview §"`ETag` and Last-Modified"), `Location`
/// (`.../ehr/{id}/directory/{version_uid}`, `Location_directory.yaml`); with
/// no `Prefer` the body is empty (`return=minimal` default).
#[tokio::test]
async fn create_returns_201_weak_etag_location_empty_body() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    #[expect(
        clippy::disallowed_methods,
        reason = "a throwaway arbitrary identifier for a negative path — never a \
                  stored key, so uuidv7 index locality is irrelevant"
    )]
    let comp_id = Uuid::new_v4().to_string();
    let folder = folder_with_items("root", vec![], &[comp_id.as_str()]);
    let (status, h, body) = create_directory(&app, &ehr, &folder, None).await;

    assert_eq!(status, StatusCode::CREATED, "create: {body}");
    let raw_etag = header_str(&h, header::ETAG).expect("201_directory declares ETag");
    assert!(
        raw_etag.starts_with("W/\""),
        "ETag MUST carry the weakness indicator W/: {raw_etag:?}"
    );
    let uid = etag_uid(&h);
    assert_eq!(
        header_str(&h, header::LOCATION).as_deref(),
        Some(&*format!("{BASE}/ehr/{ehr}/directory/{uid}")),
        "Location_directory: .../ehr/{{id}}/directory/{{version_uid}}"
    );
    assert!(
        body.is_empty(),
        "Prefer absent ⇒ return=minimal ⇒ empty body: {body:?}"
    );
}

// ── 2. create with return=representation → 201 + FOLDER body ──────────────────

/// `201_directory.yaml`: with `Prefer: return=representation` the full FOLDER
/// resource is the body (`application/json`).
#[tokio::test]
async fn create_representation_returns_folder_body() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let folder = folder_json("root-repr", vec![]);
    let (status, h, body) =
        create_directory(&app, &ehr, &folder, Some("return=representation")).await;

    assert_eq!(status, StatusCode::CREATED, "create repr: {body}");
    assert_eq!(
        header_str(&h, header::CONTENT_TYPE).as_deref(),
        Some("application/json")
    );
    let v: Value = serde_json::from_str(&body).expect("FOLDER json");
    assert_eq!(
        v["_type"], "FOLDER",
        "representation body is the FOLDER: {body}"
    );
    assert_eq!(
        v["name"]["value"], "root-repr",
        "the submitted name is echoed"
    );
}

// ── 3. create with return=identifier → 201 + {uid} + Preference-Applied ───────

/// `201_directory.yaml` (`Identifier.yaml`): `Prefer: return=identifier` ⇒ the
/// body is exactly `{"uid": "<version_uid>"}` (matching the `ETag`), with a
/// `Preference-Applied: return=identifier` header (overview §Prefer).
#[tokio::test]
async fn create_identifier_returns_uid_only() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let folder = folder_json("root-id", vec![]);
    let (status, h, body) = create_directory(&app, &ehr, &folder, Some("return=identifier")).await;

    assert_eq!(status, StatusCode::CREATED, "create identifier: {body}");
    let uid = etag_uid(&h);
    let v: Value = serde_json::from_str(&body).expect("identifier json");
    assert_eq!(v, json!({ "uid": uid }), "body is exactly {{uid}}: {body}");
    assert_eq!(
        raw_header(&h, "preference-applied").as_deref(),
        Some("return=identifier")
    );
}

// ── 4. create on EHR-with-directory → 409; create on unknown EHR → 404 ────────

/// CNF E.2 (`create_directory-ehr_with_directory`) requires an error when the
/// EHR already has a directory; the operation yaml is silent on the code, so
/// 409 Conflict is our documented choice (no openEHR spec fixes the code — our
/// own design). CNF E.3 (`create_directory-bad_ehr`) → `404_unknown_ehr_id`.
#[tokio::test]
async fn create_second_directory_conflicts_and_unknown_ehr_is_404() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let (status, _h, body) = create_directory(&app, &ehr, &folder_json("root", vec![]), None).await;
    assert_eq!(status, StatusCode::CREATED, "first create: {body}");

    let (status, _h, _b) = create_directory(&app, &ehr, &folder_json("again", vec![]), None).await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "a second directory on the same EHR conflicts (CNF E.2)"
    );

    let (status, _h, _b) =
        create_directory(&app, OTHER_EHR, &folder_json("root", vec![]), None).await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "create on an unknown EHR is 404 (CNF E.3 / 404_unknown_ehr_id)"
    );
}

// ── 5. Last-Modified on create 201 and GET 200 ───────────────────────────────

/// Overview §"`ETag` and Last-Modified": both SHOULD be included on responses
/// for versioned resources. This asserts a parseable `Last-Modified`
/// (IMF-fixdate, ends "GMT") on the create `201` AND on the `GET` `200`.
#[tokio::test]
async fn last_modified_present_on_create_and_get() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let (status, h, _b) = create_directory(&app, &ehr, &folder_json("root", vec![]), None).await;
    assert_eq!(status, StatusCode::CREATED);
    let lm = header_str(&h, header::LAST_MODIFIED)
        .expect("Last-Modified SHOULD be present on the create 201");
    assert!(lm.ends_with("GMT"), "Last-Modified is an HTTP-date: {lm:?}");

    let (status, h, _b) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory"), None)).await;
    assert_eq!(status, StatusCode::OK);
    let lm = header_str(&h, header::LAST_MODIFIED)
        .expect("Last-Modified SHOULD be present on the GET 200");
    assert!(lm.ends_with("GMT"), "Last-Modified is an HTTP-date: {lm:?}");
}

// ── 6. update: 204 / 200-representation / 200-identifier ──────────────────────

/// `directory_update.yaml`: `204_version_updated` (default/minimal, new
/// `ETag`), `200_directory_updated` with the FOLDER body on
/// `return=representation`, or `{uid}` on `return=identifier`.
#[tokio::test]
async fn update_minimal_representation_identifier() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("v1", vec![]), None).await;
    let v1 = etag_uid(&h);

    // minimal → 204, new ETag ≠ old.
    let (status, h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("v2", vec![]),
        Some(&quoted(&v1)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "204_version_updated");
    let v2 = etag_uid(&h);
    assert_ne!(v2, v1, "the update mints a new version uid");

    // representation → 200 + FOLDER body.
    let (status, h, body) = update_directory(
        &app,
        &ehr,
        &folder_json("v3", vec![]),
        Some(&quoted(&v2)),
        Some("return=representation"),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "200_directory_updated: {body}");
    let v: Value = serde_json::from_str(&body).expect("FOLDER json");
    assert_eq!(v["_type"], "FOLDER");
    assert_eq!(v["name"]["value"], "v3");
    let v3 = etag_uid(&h);

    // identifier → 200 + {uid}.
    let (status, h, body) = update_directory(
        &app,
        &ehr,
        &folder_json("v4", vec![]),
        Some(&quoted(&v3)),
        Some("return=identifier"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "200_directory_updated (identifier): {body}"
    );
    let v4 = etag_uid(&h);
    let v: Value = serde_json::from_str(&body).expect("identifier json");
    assert_eq!(v, json!({ "uid": v4 }));
}

// ── 7. update with stale If-Match → 412 + latest ETag ────────────────────────

/// `412_directory.yaml`: a stale `If-Match` (the superseded version uid) fails
/// the precondition; the `412` returns the CURRENT latest `version_uid` in the
/// `ETag` header.
#[tokio::test]
async fn update_stale_if_match_is_412_with_latest_etag() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("v1", vec![]), None).await;
    let v1 = etag_uid(&h);
    let (_s, h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("v2", vec![]),
        Some(&quoted(&v1)),
        None,
    )
    .await;
    let v2 = etag_uid(&h);

    // v1 is now stale (latest is v2).
    let (status, h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("v3", vec![]),
        Some(&quoted(&v1)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "412_directory");
    assert_eq!(
        header_str(&h, header::ETAG).as_deref(),
        Some(&*weak(&v2)),
        "the 412 carries the latest version_uid in ETag"
    );
}

// ── 8. update missing If-Match → 400; both quoted forms accepted ──────────────

/// Overview §"If-Match and accidental overwrites": when the server expects
/// `If-Match` but the client omits it, the response SHOULD be `400 Bad
/// Request`. Both the bare-quoted (`"…"`) and weak (`W/"…"`) forms are accepted
/// on the happy path (RFC 9110 weak comparison; overview allows both).
#[tokio::test]
async fn update_without_if_match_is_400_and_both_forms_accepted() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("v1", vec![]), None).await;
    let v1 = etag_uid(&h);

    let (status, _h, _b) =
        update_directory(&app, &ehr, &folder_json("v2", vec![]), None, None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "missing required If-Match → 400"
    );

    // bare quoted form accepted.
    let (status, h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("v2", vec![]),
        Some(&quoted(&v1)),
        None,
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "bare-quoted If-Match accepted"
    );
    let v2 = etag_uid(&h);

    // weak form accepted.
    let (status, _h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("v3", vec![]),
        Some(&weak(&v2)),
        None,
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "weak W/ If-Match accepted");
}

// ── 9. update/delete on an EHR with no directory → error (404 or 412) ─────────

/// CNF H.2 (`update_directory-empty_ehr`) and I.1 (`delete_directory-empty_ehr`)
/// require an error indicating the non-existent directory — never a 2xx. Our
/// ladder answers 404 (`NotFound`); the assertion accepts either 404 or 412 as
/// the spec-visible "the precondition/target is absent" outcome.
#[tokio::test]
async fn update_and_delete_without_directory_error() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    // A well-formed but arbitrary OBJECT_VERSION_ID satisfies the If-Match parse
    // so the request reaches the "no directory" check.
    #[expect(
        clippy::disallowed_methods,
        reason = "a throwaway arbitrary identifier for a negative path — never a \
                  stored key, so uuidv7 index locality is irrelevant"
    )]
    let dummy = format!("{}::ferroehr::1", Uuid::new_v4());

    let (status, _h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("v1", vec![]),
        Some(&quoted(&dummy)),
        None,
    )
    .await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::PRECONDITION_FAILED,
        "update on an EHR with no directory must error (CNF H.2), got {status}"
    );

    let (status, _h, _b) = delete_directory(&app, &ehr, Some(&quoted(&dummy))).await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::PRECONDITION_FAILED,
        "delete on an EHR with no directory must error (CNF I.1), got {status}"
    );
}

// ── 10. delete: 204; subsequent GET reflects the logical deletion ─────────────

/// `directory_delete.yaml` → `204_deleted.yaml`. After a logical delete the
/// latest directory version is a deleted version (CNF I.2 NOTE: the directory
/// exists as `VERSION.lifecycle_state=deleted`), so `directory_get_at_time`
/// returns `204 No Content` per `204_deleted_at_time.yaml` — NOT 404 (the
/// directory versioned object still exists; 404 is reserved for an EHR/version/
/// path that never existed).
#[tokio::test]
async fn delete_then_get_is_204_deleted() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("v1", vec![]), None).await;
    let v1 = etag_uid(&h);

    let (status, _h, body) = delete_directory(&app, &ehr, Some(&quoted(&v1))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "204_deleted");
    assert!(body.is_empty(), "204 carries no body: {body:?}");

    let (status, _h, body) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory"), None)).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "GET after delete is 204_deleted_at_time (the deleted version is the latest)"
    );
    assert!(body.is_empty(), "204 carries no body: {body:?}");
}

// ── 10b. create after logical delete opens a NEW hierarchy ────────────────────

/// After a logical delete the deleted container remains (RM common master06
/// §Logical Deletion) but the directory slot is vacant: a new `POST` succeeds
/// and opens a NEW hierarchy (RM ehr master04 §Folders — "an entirely new
/// Folder hierarchy may be added"; CNF master09 E.2's conflict governs a
/// LIVE directory only). The deleted hierarchy's history stays readable by
/// `version_uid`.
#[tokio::test]
async fn create_after_delete_opens_a_new_hierarchy() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("first", vec![]), None).await;
    let v1 = etag_uid(&h);
    let (status, _h, _b) = delete_directory(&app, &ehr, Some(&quoted(&v1))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete the first hierarchy");

    let (status, h, _b) = create_directory(&app, &ehr, &folder_json("second", vec![]), None).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "create after logical delete must succeed (vacant slot)"
    );
    let v2 = etag_uid(&h);
    assert_ne!(
        v1.split("::").next(),
        v2.split("::").next(),
        "the new directory is a NEW VERSIONED_FOLDER, not a version of the deleted one"
    );

    // The current directory is the new hierarchy…
    let (status, _h, body) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory"), None)).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(v["name"]["value"], "second");

    // …and the deleted hierarchy's history stays readable by version_uid.
    let (status, _h, body) =
        send(&app, get(format!("{BASE}/ehr/{ehr}/directory/{v1}"), None)).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the pre-delete version remains readable: {body}"
    );
}

// ── 11. delete with stale If-Match → 412 + latest ETag ───────────────────────

/// `directory_delete.yaml` → `412_directory.yaml`: a stale `If-Match` fails the
/// precondition and returns the current latest `version_uid` in `ETag`.
#[tokio::test]
async fn delete_stale_if_match_is_412_with_latest_etag() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("v1", vec![]), None).await;
    let v1 = etag_uid(&h);
    let (_s, h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("v2", vec![]),
        Some(&quoted(&v1)),
        None,
    )
    .await;
    let v2 = etag_uid(&h);

    let (status, h, _b) = delete_directory(&app, &ehr, Some(&quoted(&v1))).await;
    assert_eq!(status, StatusCode::PRECONDITION_FAILED, "412_directory");
    assert_eq!(
        header_str(&h, header::ETAG).as_deref(),
        Some(&*weak(&v2)),
        "the 412 carries the latest version_uid in ETag"
    );
}

// ── 12. GET ?version_at_time time-travel + malformed/absent/deleted ───────────

/// `directory_get_at_time.yaml` (`version_at_time`): time between the two
/// commits → the v1 body (CNF G.5/G.8); absent time → the latest (v2); a time
/// before the directory's first version → `404`
/// (`404_directory_unknown_ehr_id_or_no_version_at_time_or_no_path.yaml`); a
/// malformed time → `400` (an argument-validity precondition failure); after a
/// delete, a time past the delete → `204` (`204_deleted_at_time.yaml`).
#[tokio::test]
async fn get_version_at_time_variants() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    // A time strictly before the directory's first version was committed.
    let before = jiff::Timestamp::now();
    tokio::time::sleep(Duration::from_millis(150)).await;

    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("dir-v1", vec![]), None).await;
    let v1 = etag_uid(&h);

    // A time firmly inside v1's validity window (after v1, before v2) — the
    // testcontainer PG and this process share one system clock, so the client
    // instant and the server commit instant are drawn from the same clock; the
    // 150 ms margins keep `between` strictly between the two commit instants.
    tokio::time::sleep(Duration::from_millis(150)).await;
    let between = jiff::Timestamp::now();
    tokio::time::sleep(Duration::from_millis(150)).await;

    let (_s, _h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("dir-v2", vec![]),
        Some(&quoted(&v1)),
        None,
    )
    .await;

    // between → v1 body.
    let (status, _h, body) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr}/directory?version_at_time={between}"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "at_time in v1 window: {body}");
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(
        v["name"]["value"], "dir-v1",
        "the version extant at `between` is v1"
    );

    // absent time → latest (v2).
    let (status, _h, body) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory"), None)).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(v["name"]["value"], "dir-v2", "absent time ⇒ latest");

    // a time before the directory's first version → 404 (no version at that time).
    let (status, _h, _b) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr}/directory?version_at_time={before}"),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "no version at a pre-creation time"
    );

    // malformed time → 400.
    let (status, _h, _b) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr}/directory?version_at_time=not-a-time"),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "malformed version_at_time is 400"
    );

    // delete, then a time past the delete → 204.
    let latest = {
        let (_s, h, _b) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory"), None)).await;
        etag_uid(&h)
    };
    let (status, _h, _b) = delete_directory(&app, &ehr, Some(&quoted(&latest))).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "204_deleted");
    tokio::time::sleep(Duration::from_millis(50)).await;
    let after_delete = jiff::Timestamp::now();
    let (status, _h, _b) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr}/directory?version_at_time={after_delete}"),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "a time past the delete ⇒ 204_deleted_at_time"
    );
}

/// `Resources.md` §"Datetime format": a datetime query parameter MUST be
/// extended ISO 8601, and "Timezone SHOULD be only supplied when needed,
/// otherwise the local timezone is assumed" — so an offset-LESS extended
/// datetime is a well-formed `version_at_time`, resolved in the server's local
/// timezone, never a `400`. Asserted by rendering the same instant both ways
/// and requiring the same version back; the router under test runs in this
/// process, so its "local timezone" is this process's system zone and the test
/// is independent of what that zone happens to be.
#[tokio::test]
async fn version_at_time_without_offset_resolves_in_the_local_timezone() {
    let (pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("dir-v1", vec![]), None).await;
    let v1 = etag_uid(&h);

    // The probe instant is v1's OWN commit instant, read back from the database
    // rather than sampled from a clock (#3119). `committed_at` is stamped by
    // PostgreSQL, whose container clock need not agree with this process's, and
    // the margins this test used to sleep around were eaten under full-suite
    // load. A version is in force AT its own commit instant, so the version
    // extant at exactly that instant is v1 by construction, with no margin to
    // lose.
    let between: jiff_sqlx::Timestamp = sqlx::query_scalar(
        "SELECT committed_at FROM version \
         WHERE kind = 'FOLDER' AND sys_version = 1 AND ehr_id = $1",
    )
    .bind(ehr.parse::<Uuid>().expect("the EHR id"))
    .fetch_one(&pg.pool())
    .await
    .expect("v1's stored commit instant");
    let between = between.to_jiff();

    let (_s, _h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("dir-v2", vec![]),
        Some(&quoted(&v1)),
        None,
    )
    .await;

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

    let (status, _h, body) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr}/directory?version_at_time={offset_less}"),
            None,
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an offset-less extended datetime is a valid version_at_time: {body}"
    );
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(
        v["name"]["value"], "dir-v1",
        "the offset-less rendering names the same instant as its offset-carrying form"
    );
}

// ── 13. ?path= sub-folder navigation on the current read ──────────────────────

/// `directory_get_at_time.yaml` (`path`): the sub-FOLDER at a slash-separated
/// name path is returned; a non-existent path → `404`
/// (`404_directory_unknown_ehr_id_or_no_version_at_time_or_no_path.yaml`). CNF
/// D.3 uses the `emergency/episode-x` structure.
#[tokio::test]
async fn get_path_navigates_subfolders() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let episode = folder_json("episode_x", vec![]);
    let emergency = folder_json("emergency", vec![episode]);
    let root = folder_json("root", vec![emergency]);
    let (status, _h, body) = create_directory(&app, &ehr, &root, None).await;
    assert_eq!(status, StatusCode::CREATED, "create nested: {body}");

    // ?path=emergency → the emergency sub-FOLDER.
    let (status, _h, body) = send(
        &app,
        get(format!("{BASE}/ehr/{ehr}/directory?path=emergency"), None),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "path=emergency: {body}");
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(
        v["name"]["value"], "emergency",
        "the sub-FOLDER named emergency"
    );
    assert_eq!(
        v["folders"][0]["name"]["value"], "episode_x",
        "the emergency folder still contains episode_x"
    );

    // ?path=emergency/episode_x → the leaf FOLDER (slash percent-encoded).
    let (status, _h, body) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr}/directory?path=emergency%2Fepisode_x"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "path=emergency/episode_x: {body}");
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(v["name"]["value"], "episode_x");

    // ?path=missing → 404.
    let (status, _h, _b) = send(
        &app,
        get(format!("{BASE}/ehr/{ehr}/directory?path=missing"), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an absent path is 404");
}

// ── 14. ?path= sub-folder navigation on the VERSION read ──────────────────────

/// `directory_get_by_version_id.yaml` declares the `path` query parameter: "If
/// `path` is supplied, retrieves from the directory only the sub-FOLDER that is
/// associated with that path." A `?path=emergency` on the version read must
/// return the `emergency` sub-FOLDER, exactly as on the at-time read.
#[tokio::test]
async fn get_by_version_id_path_navigates_subfolders() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let emergency = folder_json("emergency", vec![folder_json("episode_x", vec![])]);
    let root = folder_json("root", vec![emergency]);
    let (_s, h, _b) = create_directory(&app, &ehr, &root, None).await;
    let uid = etag_uid(&h);

    let (status, _h, body) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr}/directory/{uid}?path=emergency"),
            None,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "version read with path: {body}");
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(
        v["name"]["value"], "emergency",
        "the version read must honour ?path= and return the sub-FOLDER"
    );
}

// ── 15. GET /directory/{version_uid} — historic version + 404s ────────────────

/// `directory_get_by_version_id.yaml`: the named (superseded) version resolves
/// to its own body; a well-formed-but-unknown `version_uid`, or a version read
/// on an unknown EHR, is `404`
/// (`404_directory_unknown_ehr_id_or_no_version_uid_or_no_path.yaml`).
#[tokio::test]
async fn get_by_version_id_historic_and_not_found() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("dir-v1", vec![]), None).await;
    let v1 = etag_uid(&h);
    let (_s, _h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("dir-v2", vec![]),
        Some(&quoted(&v1)),
        None,
    )
    .await;

    // the v1 uid still resolves to the v1 body after the update to v2.
    let (status, _h, body) =
        send(&app, get(format!("{BASE}/ehr/{ehr}/directory/{v1}"), None)).await;
    assert_eq!(status, StatusCode::OK, "historic version read: {body}");
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(
        v["name"]["value"], "dir-v1",
        "the v1 uid returns the v1 body"
    );

    // a well-formed but unknown version_uid → 404 (a fresh vo-uuid, same
    // creating-system + version-tree tail, so the OBJECT_VERSION_ID parses).
    let (_vo, rest) = v1.split_once("::").expect("ovid has ::");
    #[expect(
        clippy::disallowed_methods,
        reason = "a throwaway arbitrary identifier for a negative path — never a \
                  stored key, so uuidv7 index locality is irrelevant"
    )]
    let unknown = format!("{}::{rest}", Uuid::new_v4());
    let (status, _h, _b) = send(
        &app,
        get(format!("{BASE}/ehr/{ehr}/directory/{unknown}"), None),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "unknown version_uid is 404");

    // a version read on an unknown EHR → 404.
    let (status, _h, _b) = send(
        &app,
        get(format!("{BASE}/ehr/{OTHER_EHR}/directory/{v1}"), None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "version read on an unknown EHR is 404"
    );
}

// ── 16. XML retrieval ────────────────────────────────────────────────────────

/// `200_FOLDER_retrieved.yaml` + `ContentType_LOCATABLE`: a `GET` with
/// `Accept: application/xml` returns canonical XML (Resources.md §Data
/// representation). (Create-via-XML is not exercised — the sibling suites drive
/// canonical objects through JSON.)
#[tokio::test]
async fn get_directory_as_xml() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (_s, _h, _b) = create_directory(&app, &ehr, &folder_json("root-xml", vec![]), None).await;

    let (status, h, body) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr}/directory"),
            Some("application/xml"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "xml get: {body}");
    assert_eq!(
        header_str(&h, header::CONTENT_TYPE).as_deref(),
        Some("application/xml")
    );
    assert!(
        body.trim_start().starts_with('<'),
        "an XML document: {body}"
    );
    assert!(
        body.contains("root-xml"),
        "the folder name is present: {body}"
    );
}

// ── 17. Root FOLDER uid population ────────────────────────────────────────────

/// RM FOLDER (`org.openehr.rm.common.folder.adoc`): the root FOLDER's `uid`
/// SHOULD be the enclosing VERSION's `OBJECT_VERSION_ID`. After a create the
/// GET'd root FOLDER's `uid.value` must equal the create's version uid; after
/// an update it must equal the v2 version uid.
#[tokio::test]
async fn root_folder_uid_is_the_version_uid() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("v1", vec![]), None).await;
    let v1 = etag_uid(&h);

    let (status, _h, body) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory"), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(
        v["uid"]["value"], v1,
        "root FOLDER.uid.value == the create version uid"
    );

    let (_s, h, _b) = update_directory(
        &app,
        &ehr,
        &folder_json("v2", vec![]),
        Some(&quoted(&v1)),
        None,
    )
    .await;
    let v2 = etag_uid(&h);
    let (status, _h, body) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory"), None)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    let v: Value = serde_json::from_str(&body).expect("folder json");
    assert_eq!(
        v["uid"]["value"], v2,
        "after update, root FOLDER.uid.value == the v2 version uid"
    );
}

// ── 18. Committal headers do not break the commit ─────────────────────────────

/// Overview §"openehr-version and openehr-audit-details": the
/// `openEHR-AUDIT_DETAILS.*` committal request headers "MUST be merged with the
/// default VERSION and `VERSION.audit_details` attributes on commit". The
/// directory write path threads them through `mk_update_version`; their presence
/// must not break the create (`201`). (The merged values are not observable via
/// the DIRECTORY endpoints — there is no versioned-directory version read — so
/// this is the accept-level assertion.)
#[tokio::test]
async fn create_with_committal_headers_is_201() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr}/directory"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(
            "openEHR-AUDIT_DETAILS.description",
            "value=\"initial directory\"",
        )
        .header("openEHR-AUDIT_DETAILS.committer", "name=\"Jane Roe\"")
        .body(Body::from(folder_json("root", vec![]).to_string()))
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "committal headers must not break the commit: {body}"
    );
}

// ── 19. Duplicate sibling folder names accepted ──────────────────────────────

/// RM ehr master04 §Folders / RM common FOLDER: there is no uniqueness
/// invariant on sibling Folder names (the slash-name path is a convention, not
/// a key), so two sibling folders both named "twin" are accepted (`201`).
#[tokio::test]
async fn duplicate_sibling_folder_names_accepted() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let root = folder_json(
        "root",
        vec![folder_json("twin", vec![]), folder_json("twin", vec![])],
    );
    let (status, _h, body) = create_directory(&app, &ehr, &root, None).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "duplicate sibling folder names are valid: {body}"
    );
}

// ── 21. round-trip: PUT the FETCHED body back (the viewer's edit flow) ──────

/// The read decorates the root FOLDER with its `uid` (`OBJECT_VERSION_ID`,
/// RM FOLDER class NOTE); an editor that fetches, mutates, and PUTs the SAME
/// body back (the viewer's flow) must succeed — the stale embedded
/// `uid` is versioning metadata, not client intent (`directory_update.yaml`).
#[tokio::test]
async fn update_with_a_fetched_body_round_trips() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let sub = folder_json("episode_x", vec![]);
    let (_s, h, _b) = create_directory(&app, &ehr, &folder_json("root", vec![sub]), None).await;
    let v1 = etag_uid(&h);

    // Fetch the directory the way a client editor does.
    let (status, _h, body) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory"), None)).await;
    assert_eq!(status, StatusCode::OK);
    let mut fetched: Value = serde_json::from_str(&body).expect("folder json");
    assert!(
        fetched.get("uid").is_some(),
        "the read carries the root uid (RM FOLDER class NOTE)"
    );

    // Mutate: add one more sub-folder, then PUT the fetched body back.
    fetched["folders"]
        .as_array_mut()
        .expect("folders array")
        .push(folder_json("added", vec![]));
    let (status, h, body) = update_directory(&app, &ehr, &fetched, Some(&quoted(&v1)), None).await;
    assert_eq!(
        status,
        StatusCode::NO_CONTENT,
        "PUT of a fetched body must succeed: {body}"
    );
    let v2 = etag_uid(&h);
    assert!(v2.ends_with("::2"), "trunk v2, got {v2}");
}

/// The directory DELETE 204 carries the NEW `523|deleted|` version's weak
/// `ETag` + `Last-Modified` (overview §"`ETag` and Last-Modified": both
/// SHOULD accompany versioned resources; RM common master06 §Logical
/// Deletion: the delete commits a new version).
#[tokio::test]
async fn delete_204_carries_the_deleted_versions_identity() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (status, h, _b) = create_directory(&app, &ehr, &folder_json("root", vec![]), None).await;
    assert_eq!(status, StatusCode::CREATED);
    let v1 = etag_uid(&h);

    let (status, h, _b) = delete_directory(&app, &ehr, Some(&quoted(&v1))).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let deleted_uid = etag_uid(&h);
    assert_ne!(deleted_uid, v1, "the 204 names the NEW deleted version");
    assert!(
        deleted_uid.ends_with("::2"),
        "the delete committed the next trunk version: {deleted_uid}"
    );
    assert!(
        header_str(&h, header::LAST_MODIFIED).is_some(),
        "Last-Modified accompanies the deleted version's identity"
    );
}

/// The by-version read verifies the FULL addressed identity: a fabricated
/// `creating_system_id` names no VERSION in this repository → 404
/// (Resources.md §Identifier types; BASE master05 case rule) — while the
/// stored identity still serves 200.
#[tokio::test]
async fn by_version_read_rejects_fabricated_creating_system_id() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let (status, h, _b) = create_directory(&app, &ehr, &folder_json("root", vec![]), None).await;
    assert_eq!(status, StatusCode::CREATED);
    let v1 = etag_uid(&h);

    let (status, _h, _b) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory/{v1}"), None)).await;
    assert_eq!(status, StatusCode::OK, "the stored identity serves");

    let (vo, tree) = (
        v1.split("::").next().unwrap(),
        v1.rsplit("::").next().unwrap(),
    );
    let fabricated = format!("{vo}::not.this.system::{tree}");
    let (status, _h, _b) = send(
        &app,
        get(format!("{BASE}/ehr/{ehr}/directory/{fabricated}"), None),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "a fabricated creating_system_id names no VERSION"
    );
}

// ── 22. FOLDER.details is an ITEM_STRUCTURE slot ─────────────────────────────

/// A valid `FOLDER.details` — an `ITEM_TREE`, one of the four concrete
/// `ITEM_STRUCTURE` subtypes (RM `data_structures` `master04`).
fn item_tree_details() -> Value {
    json!({
        "_type": "ITEM_TREE",
        "name": dv_text("details"),
        "archetype_node_id": "at0001",
        "items": [{
            "_type": "ELEMENT",
            "name": dv_text("note"),
            "archetype_node_id": "at0002",
            "value": dv_text("ward 4")
        }]
    })
}

/// The ACCEPTING twin: `FOLDER.details` (0..1) is typed `ITEM_STRUCTURE` —
/// "Any individual Folder may contain meta-data in its `details` attribute
/// (type `ITEM_STRUCTURE`)" (RM common `master05-directory_package.adoc`
/// §Overview; `org.openehr.rm.common.folder.adoc`). A concrete `ITEM_TREE`
/// commits and reads back verbatim, in JSON and in canonical XML.
#[tokio::test]
async fn details_item_structure_commits_and_reads_back() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let mut folder = folder_json("root", vec![]);
    folder["details"] = item_tree_details();
    let (status, _h, body) = create_directory(&app, &ehr, &folder, None).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "ITEM_TREE details commit: {body}"
    );

    let (status, _h, body) = send(&app, get(format!("{BASE}/ehr/{ehr}/directory"), None)).await;
    assert_eq!(status, StatusCode::OK, "JSON read: {body}");
    let read: Value = serde_json::from_str(&body).expect("a JSON FOLDER body");
    assert_eq!(
        read["details"], folder["details"],
        "FOLDER.details round-trips verbatim"
    );

    let (status, _h, body) = send(
        &app,
        get(
            format!("{BASE}/ehr/{ehr}/directory"),
            Some("application/xml"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "XML read: {body}");
    assert!(
        body.contains("ward 4"),
        "the canonical XML carries the details ITEM_TREE: {body}"
    );
}

/// The REFUSING twin: a `DV_TEXT` is not an `ITEM_STRUCTURE`, so it may not
/// occupy `FOLDER.details` (RM common `master05-directory_package.adoc`
/// §Overview; `org.openehr.rm.common.folder.adoc`
/// `details: ITEM_STRUCTURE [0..1]`).
///
/// The slot is a polymorphic one over a CLOSED subtype set, so the refusal is
/// the strict canonical reader's: the body never becomes a FOLDER, which the
/// ITS-REST overview (`Requests_and_responses.md` §HTTP status codes) answers
/// `400` — content that "could not be parsed or is invalid" — rather than the
/// `422` it reserves for a body that is "well-formed but was unable to be
/// followed due to semantic errors". The refusal still has to NAME the slot
/// and its declared type, which is what makes it actionable.
#[tokio::test]
async fn details_not_item_structure_refused() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;
    let mut folder = folder_json("root", vec![]);
    folder["details"] = json!({ "_type": "DV_TEXT", "value": "x" });
    let (status, _h, body) = create_directory(&app, &ehr, &folder, None).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "a DV_TEXT in the ITEM_STRUCTURE slot is refused: {body}"
    );
    assert!(
        body.contains("ITEM_STRUCTURE") && body.contains("details"),
        "the 400 names the FOLDER.details slot and its declared type: {body}"
    );
}

/// A root FOLDER whose `archetype_node_id` does not equal the stringified
/// `archetype_details.archetype_id` converts fine and fails the RM invariant
/// pass → `422` (RM common `locatable.adoc` §`Archetyped_valid` family — the
/// root-identity rule; overview `Requests_and_responses.md` §HTTP status
/// codes, row 422).
#[tokio::test]
async fn create_rejects_root_archetype_id_mismatch_as_422() {
    let (_pg, app) = common::test_router().await;
    let ehr = create_ehr(&app).await;

    let folder = json!({
        "_type": "FOLDER",
        "name": {"_type": "DV_TEXT", "value": "root"},
        "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": {"_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.other.v1"},
            "rm_version": "1.1.0"
        }
    });
    let (status, _h, body) = create_directory(&app, &ehr, &folder, None).await;
    assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY, "got body: {body}");
    assert!(
        body.contains("archetype"),
        "the 422 names the root archetype-identity rule: {body}"
    );
}
