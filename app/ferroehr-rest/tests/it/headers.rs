// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `ETag` expectations use the weak form (`W/"…"`): the ITS-REST overview
//! §"`ETag` and Last-Modified" makes the `ETag` weak-type ("should have a
//! weakness indicator `W/` prefix"); the bare quoted form is deprecated.
//! End-to-end HTTP tests for the response-header + `Prefer` handling:
//! `ETag`/`Location` on the EHR / `EHR_STATUS` / COMPOSITION writes and reads, and
//! the `return=minimal` (default, header-only) vs `return=representation`
//! (full body) `Prefer` policy — driven through the assembled router over a
//! **real** `FerroEhrService` on a real `PostgreSQL`.
//!
//! The former `Mock` backend returned fixed resource ids; the real service
//! assigns each version its own `OBJECT_VERSION_ID`, so the tests create the
//! resources through the wire and read the server-assigned ids back from the
//! `ETag`/body. The wire assertions (weak `ETag`, `Location` presence/absence,
//! `Prefer` body policy, status codes) are unchanged.
#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::Value;

use crate::common;
use crate::common::BASE;

async fn app() -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (pg, common::router_with(common::api_config(false), service))
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, header::HeaderMap, String) {
    common::send(app, req).await
}

fn etag(h: &header::HeaderMap) -> Option<&str> {
    h.get(header::ETAG).and_then(|v| v.to_str().ok())
}

fn location(h: &header::HeaderMap) -> Option<&str> {
    h.get(header::LOCATION).and_then(|v| v.to_str().ok())
}

fn last_modified(h: &header::HeaderMap) -> Option<&str> {
    h.get(header::LAST_MODIFIED).and_then(|v| v.to_str().ok())
}

/// An openEHR `DV_DATE_TIME` value rendered as an HTTP-date (IMF-fixdate,
/// RFC 9110 §5.6.7) — the form `Last-Modified` carries, and the exact value
/// ITS-REST overview §"`ETag` and Last-Modified" derives from
/// `VERSION.commit_audit.time_committed.value`.
fn imf_fixdate(iso_instant: &str) -> String {
    iso_instant
        .parse::<jiff::Timestamp>()
        .expect("commit_audit.time_committed.value is an ISO 8601 instant")
        .strftime("%a, %d %b %Y %H:%M:%S GMT")
        .to_string()
}

/// `VERSION.commit_audit.time_committed.value` of a served `ORIGINAL_VERSION`.
fn commit_instant(version: &Value) -> &str {
    version["commit_audit"]["time_committed"]["value"]
        .as_str()
        .expect("ORIGINAL_VERSION.commit_audit.time_committed.value")
}

/// The bare uid inside a weak `ETag` (`W/"{uid}"`).
fn etag_uid(h: &header::HeaderMap) -> String {
    common::etag_uid(h)
}

fn vo_of(ovid: &str) -> &str {
    ovid.split("::").next().expect("vo uuid")
}

// ── wire setup helpers ───────────────────────────────────────────────────────

/// Create an EHR through the wire; return its id (from the create `ETag`).
async fn create_ehr(app: &Router) -> String {
    common::create_ehr(app).await
}

/// Upload the Demo Vitals OPT (canonical XML) through the wire.
async fn upload_opt(app: &Router) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(common::ips_opt_xml()))
        .unwrap();
    let (status, _h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
}

/// Commit the IPS composition into `ehr_id`; return its `version_uid`.
async fn commit_composition(app: &Router, ehr_id: &str) -> String {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(common::ips_canonical_composition().to_string()))
        .unwrap();
    let (status, h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "composition commit: {body}");
    etag_uid(&h)
}

/// The current `EHR_STATUS` body + its `version_uid`.
async fn current_ehr_status(app: &Router, ehr_id: &str) -> (Value, String) {
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK, "ehr_status read: {body}");
    let v: Value = serde_json::from_str(&body).expect("json ehr_status");
    let ovid = v["uid"]["value"].as_str().expect("status uid").to_owned();
    (v, ovid)
}

// ── EHR create ───────────────────────────────────────────────────────────────

#[tokio::test]
async fn ehr_create_default_is_minimal_with_headers() {
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    // 201_EHR default (return=minimal): headers only, no body.
    assert_eq!(status, StatusCode::CREATED);
    let ehr_id = etag_uid(&h);
    assert_eq!(etag(&h), Some(format!("W/\"{ehr_id}\"").as_str()));
    assert_eq!(location(&h), Some(format!("{BASE}/ehr/{ehr_id}").as_str()));
    assert!(body.is_empty(), "minimal create has no body, got {body:?}");
}

#[tokio::test]
async fn ehr_create_representation_returns_body() {
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .header("Prefer", "return=representation")
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::CREATED);
    let ehr_id = etag_uid(&h);
    assert_eq!(etag(&h), Some(format!("W/\"{ehr_id}\"").as_str()));
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["_type"], "EHR");
}

// ── EHR_STATUS update ────────────────────────────────────────────────────────

#[tokio::test]
async fn ehr_status_update_default_is_204_with_headers() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    let (mut status_body, current) = current_ehr_status(&app, &ehr_id).await;
    // Re-commit the current status (a new version) — strip the uid the server owns.
    status_body.as_object_mut().unwrap().remove("uid");

    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{current}\""))
        .body(Body::from(status_body.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    // 204_EHR_STATUS (default minimal): no body, ETag + Location of the new version.
    assert_eq!(status, StatusCode::NO_CONTENT);
    let new_ovid = etag_uid(&h);
    assert_ne!(new_ovid, current, "a new version was created");
    assert_eq!(etag(&h), Some(format!("W/\"{new_ovid}\"").as_str()));
    assert_eq!(
        location(&h),
        Some(format!("{BASE}/ehr/{ehr_id}/ehr_status/{new_ovid}").as_str())
    );
    assert!(body.is_empty());
}

#[tokio::test]
async fn ehr_status_update_representation_is_200_with_body() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    let (mut status_body, current) = current_ehr_status(&app, &ehr_id).await;
    status_body.as_object_mut().unwrap().remove("uid");

    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{current}\""))
        .header("Prefer", "return=representation")
        .body(Body::from(status_body.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    // 200_EHR_STATUS_updated (representation): body present.
    assert_eq!(status, StatusCode::OK);
    let new_ovid = etag_uid(&h);
    assert_eq!(etag(&h), Some(format!("W/\"{new_ovid}\"").as_str()));
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["_type"], "EHR_STATUS");
}

// ── COMPOSITION read/delete ──────────────────────────────────────────────────

#[tokio::test]
async fn composition_get_sets_etag_and_location() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;
    let ovid = commit_composition(&app, &ehr_id).await;
    let vo = vo_of(&ovid);

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    // 200_COMPOSITION_retrieved: ETag(version_uid) + Location deprecated (absent).
    assert_eq!(status, StatusCode::OK);
    assert_eq!(etag(&h), Some(format!("W/\"{ovid}\"").as_str()));
    assert_eq!(
        location(&h),
        None,
        "200_COMPOSITION_retrieved marks Location Location_deprecated — not emitted"
    );
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["_type"], "COMPOSITION");
}

#[tokio::test]
async fn versioned_ehr_status_version_at_time_sets_version_headers() {
    // 200_VERSION_of_EHR_STATUS_at_time declares ETag_VERSION (the version_uid)
    // plus a `Location_deprecated` header — the deprecated Location is no longer
    // emitted (overview §"Deprecated headers").
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    let (_body, current) = current_ehr_status(&app, &ehr_id).await;

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status/version"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(status, StatusCode::OK);
    assert_eq!(etag(&h), Some(format!("W/\"{current}\"").as_str()));
    assert_eq!(
        location(&h),
        None,
        "the OAS marks Location on this response Location_deprecated — not emitted"
    );
    let v: Value = serde_json::from_str(&body).expect("json body");
    assert_eq!(v["_type"], "ORIGINAL_VERSION");
}

#[tokio::test]
async fn composition_delete_is_204_with_headers() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;
    let ovid = commit_composition(&app, &ehr_id).await;

    let req = Request::builder()
        .method("DELETE")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{ovid}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    // 204_version_deleted: ETag of the deleted version; its Location is
    // `Location_deprecated` in the OAS and is no longer emitted.
    assert_eq!(status, StatusCode::NO_CONTENT);
    let deleted = etag_uid(&h);
    assert_eq!(etag(&h), Some(format!("W/\"{deleted}\"").as_str()));
    assert_eq!(
        location(&h),
        None,
        "the OAS marks Location on this response Location_deprecated — not emitted"
    );
    assert!(body.is_empty());
}

// ── Last-Modified (overview §"ETag and Last-Modified") ───────────────────────
//
// "The `Last-Modified` response HTTP header, indicates the date and time when
// the resource was last modified. … For openEHR resources, this value should
// be derived from `VERSION.commit_audit.time_committed.value`." and "Both
// `ETag` and `Last-Modified` SHOULD be included in responses for VERSION,
// VERSIONED_OBJECT, or other resources that have versioning or unique state
// identifiers."

/// A VERSION read (`…/versioned_ehr_status/version/{version_uid}` and the
/// at-time sibling) carries `Last-Modified` derived from the served
/// envelope's own `commit_audit.time_committed` — not merely "present", but
/// byte-equal to the IMF-fixdate rendering of that instant.
#[tokio::test]
async fn version_read_last_modified_is_the_commit_audit_instant() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;

    // …/versioned_ehr_status/version — the ORIGINAL_VERSION at the latest time.
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status/version"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json ORIGINAL_VERSION");
    assert_eq!(v["_type"], "ORIGINAL_VERSION");
    assert_eq!(
        last_modified(&h),
        Some(imf_fixdate(commit_instant(&v)).as_str()),
        "overview §\"ETag and Last-Modified\": the value \"should be derived from \
         VERSION.commit_audit.time_committed.value\""
    );

    // …/versioned_ehr_status/version/{version_uid} — the by-id VERSION read.
    let version_uid = v["uid"]["value"].as_str().expect("VERSION.uid").to_owned();
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/versioned_ehr_status/version/{version_uid}"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json ORIGINAL_VERSION");
    assert_eq!(
        etag(&h),
        Some(format!("W/\"{version_uid}\"").as_str()),
        "overview §\"ETag and Last-Modified\": both headers SHOULD accompany a VERSION"
    );
    assert_eq!(
        last_modified(&h),
        Some(imf_fixdate(commit_instant(&v)).as_str()),
        "overview §\"ETag and Last-Modified\": derived from \
         VERSION.commit_audit.time_committed.value"
    );
}

/// The `VERSIONED_OBJECT` container read (`…/versioned_ehr_status`) and its
/// `REVISION_HISTORY` sibling both carry the container-uid weak `ETag` AND a
/// `Last-Modified` equal to the newest held version's commit instant —
/// overview §"`ETag` and Last-Modified": "Both `ETag` and `Last-Modified`
/// SHOULD be included in responses for VERSION, `VERSIONED_OBJECT`, or other
/// resources that have versioning or unique state identifiers", the value
/// "derived from `VERSION.commit_audit.time_committed.value`".
#[tokio::test]
async fn container_reads_carry_etag_and_newest_commit_last_modified() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;

    // A second EHR_STATUS version, so "newest" is distinguishable from v1.
    let (status_body, ovid) = current_ehr_status(&app, &ehr_id).await;
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status"))
        .header("content-type", "application/json")
        .header("if-match", format!("\"{ovid}\""))
        .body(Body::from(status_body.to_string()))
        .unwrap();
    let (status, _, _) = send(&app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);

    // The newest version's commit instant, read off the served envelope.
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status/version"))
        .body(Body::empty())
        .unwrap();
    let (status, _, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let latest: Value = serde_json::from_str(&body).expect("json ORIGINAL_VERSION");
    assert!(
        latest["uid"]["value"]
            .as_str()
            .is_some_and(|v| v.ends_with("::2")),
        "the update committed version 2"
    );
    let newest_instant = imf_fixdate(commit_instant(&latest));
    let container_uid = vo_of(latest["uid"]["value"].as_str().expect("uid")).to_owned();

    // The VERSIONED_EHR_STATUS container read.
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json VERSIONED_EHR_STATUS");
    assert_eq!(v["_type"], "VERSIONED_EHR_STATUS");
    assert_eq!(
        etag(&h),
        Some(format!("W/\"{container_uid}\"").as_str()),
        "overview §\"ETag and Last-Modified\": \"VERSIONED_OBJECT.uid.value\" is an ETag source"
    );
    assert_eq!(
        last_modified(&h),
        Some(newest_instant.as_str()),
        "overview §\"ETag and Last-Modified\": a VERSIONED_OBJECT response SHOULD carry \
         Last-Modified, derived from the newest version's commit_audit.time_committed"
    );

    // The REVISION_HISTORY read — same container identity, same newest instant.
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/versioned_ehr_status/revision_history"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let rh: Value = serde_json::from_str(&body).expect("json REVISION_HISTORY");
    assert_eq!(rh["_type"], "REVISION_HISTORY");
    assert_eq!(etag(&h), Some(format!("W/\"{container_uid}\"").as_str()));
    assert_eq!(last_modified(&h), Some(newest_instant.as_str()));
}

/// The COMPOSITION VERSION read (`…/versioned_composition/{vo}/version/{uid}`)
/// and the bare COMPOSITION reads/writes all carry the same `Last-Modified`:
/// the commit instant of the version served. The bare COMPOSITION body has no
/// `commit_audit`, so this pins that the instant survives from the version row
/// (read) / the commit result (write) to the wire.
#[tokio::test]
async fn composition_read_and_write_carry_the_commit_instant() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;

    // POST /composition — the create 201.
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(common::ips_canonical_composition().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "composition commit: {body}");
    let ovid = etag_uid(&h);
    let created_lm = last_modified(&h)
        .expect(
            "overview §\"ETag and Last-Modified\": Last-Modified SHOULD accompany the \
             committed VERSION",
        )
        .to_owned();

    // The VERSION envelope names the authoritative instant.
    let vo = vo_of(&ovid).to_owned();
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/versioned_composition/{vo}/version/{ovid}"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let version: Value = serde_json::from_str(&body).expect("json ORIGINAL_VERSION");
    let committed = imf_fixdate(commit_instant(&version));
    assert_eq!(
        last_modified(&h),
        Some(committed.as_str()),
        "overview §\"ETag and Last-Modified\": derived from \
         VERSION.commit_audit.time_committed.value"
    );
    assert_eq!(
        created_lm, committed,
        "the create 201 reports the same commit instant the VERSION envelope carries"
    );

    // GET the bare COMPOSITION (latest) — same instant, though the served body
    // carries no commit_audit of its own.
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, _b) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(
        last_modified(&h),
        Some(committed.as_str()),
        "overview §\"ETag and Last-Modified\": both headers SHOULD accompany a resource \
         with a unique state identifier — the bare COMPOSITION read included"
    );

    // GET the bare COMPOSITION addressed by its version uid — same instant.
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{ovid}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, _b) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(last_modified(&h), Some(committed.as_str()));

    // PUT /composition — the update 200/204 carries the NEW version's instant.
    let mut updated = common::ips_canonical_composition();
    updated.as_object_mut().unwrap().remove("uid");
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{ovid}\""))
        .body(Body::from(updated.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    // No Prefer header => return=minimal => 204 (overview
    // Requests_and_responses.md §Prefer: "If no `Prefer` header is provided,
    // the default behavior is assumed to be `return=minimal`").
    assert_eq!(status, StatusCode::NO_CONTENT, "composition update: {body}");
    let new_ovid = etag_uid(&h);
    assert_ne!(new_ovid, ovid, "a new version was created");
    let updated_lm = last_modified(&h)
        .expect("overview §\"ETag and Last-Modified\": Last-Modified on the update response")
        .to_owned();

    // The new VERSION envelope confirms the reported instant.
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/versioned_composition/{vo}/version/{new_ovid}"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let version: Value = serde_json::from_str(&body).expect("json ORIGINAL_VERSION");
    assert_eq!(updated_lm, imf_fixdate(commit_instant(&version)));
}

/// The `EHR_STATUS` resource reads and its `PUT` carry `Last-Modified` too —
/// the bare `EHR_STATUS` body has no `commit_audit`, so the instant comes from
/// the version row (read) and the commit result (write).
#[tokio::test]
async fn ehr_status_read_and_write_carry_the_commit_instant() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;

    // GET /ehr_status.
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let current: Value = serde_json::from_str(&body).expect("json EHR_STATUS");
    let ovid = current["uid"]["value"]
        .as_str()
        .expect("EHR_STATUS.uid")
        .to_owned();
    let read_lm = last_modified(&h)
        .expect(
            "overview §\"ETag and Last-Modified\": both headers SHOULD accompany a resource \
             with a unique state identifier",
        )
        .to_owned();

    // GET /ehr_status/{version_uid} — the same version, the same instant.
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status/{ovid}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, _b) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(last_modified(&h), Some(read_lm.as_str()));

    // PUT /ehr_status — the new version's commit instant, cross-checked
    // against the VERSION envelope's commit_audit.
    let mut next = current;
    next.as_object_mut().unwrap().remove("uid");
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{ovid}\""))
        .body(Body::from(next.to_string()))
        .unwrap();
    let (status, h, _b) = send(&app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let new_ovid = etag_uid(&h);
    let write_lm = last_modified(&h)
        .expect("overview §\"ETag and Last-Modified\": Last-Modified on the EHR_STATUS write")
        .to_owned();

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/versioned_ehr_status/version/{new_ovid}"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let version: Value = serde_json::from_str(&body).expect("json ORIGINAL_VERSION");
    assert_eq!(write_lm, imf_fixdate(commit_instant(&version)));
}

/// The EHR create already carried the weak `ETag`; the EHR **reads** now do
/// too. Overview §"`ETag` and Last-Modified": the value "is usually taken from
/// e.g. `VERSIONED_OBJECT.uid.value`, `VERSION.uid.value`, `EHR.ehr_id.value`",
/// and both headers SHOULD accompany "resources that have versioning or unique
/// state identifiers". No `Location` on a GET (§Location).
#[tokio::test]
async fn ehr_get_carries_the_weak_ehr_id_etag() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;

    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}"))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let v: Value = serde_json::from_str(&body).expect("json EHR");
    assert_eq!(v["_type"], "EHR");
    assert_eq!(
        v["ehr_id"]["value"].as_str(),
        Some(ehr_id.as_str()),
        "the served EHR names the addressed ehr_id"
    );
    assert_eq!(
        etag(&h),
        Some(format!("W/\"{ehr_id}\"").as_str()),
        "overview §\"ETag and Last-Modified\": EHR.ehr_id.value is the named ETag source"
    );
    assert_eq!(
        location(&h),
        None,
        "overview §Location: Location MUST NOT indicate an alternate representation \
         of an existing resource (e.g. via GET)"
    );
}

/// The EHR create `201` carries `Last-Modified` (the creating CONTRIBUTION's
/// commit instant) alongside the `ETag`/`Location` it already emitted.
#[tokio::test]
async fn ehr_create_carries_last_modified() {
    let (_pg, app) = app().await;
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr"))
        .body(Body::empty())
        .unwrap();
    let (status, h, _b) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED);
    let lm = last_modified(&h).expect(
        "overview §\"ETag and Last-Modified\": both headers SHOULD accompany a resource \
         with a unique state identifier",
    );
    assert!(
        lm.ends_with("GMT"),
        "Last-Modified is an HTTP-date (RFC 9110 §5.6.7): {lm:?}"
    );

    // The EHR's own EHR_STATUS VERSION was committed in the same CONTRIBUTION,
    // so its commit_audit instant is the one reported.
    let ehr_id = etag_uid(&h);
    let req = Request::builder()
        .method("GET")
        .uri(format!("{BASE}/ehr/{ehr_id}/versioned_ehr_status/version"))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK);
    let version: Value = serde_json::from_str(&body).expect("json ORIGINAL_VERSION");
    assert_eq!(lm, imf_fixdate(commit_instant(&version)));
}

// ── openehr-item-tag / openehr-version-item-tag (overview §"openehr-item-tag ──
//    and openehr-version-item-tag") ────────────────────────────────────────────

/// The spec sentence these assertions encode: "`openehr-item-tag` applies to
/// *`VERSIONED_OBJECT`* targets" while "`openehr-version-item-tag` applies to a
/// specific target *VERSION* within a `VERSIONED_OBJECT`" — two DISTINCT targets,
/// each confirmed by "the actual list of `ITEM_TAGs` stored" for that target
/// (§Usage in Responses).
const DISTINCT_TARGETS: &str = "overview §\"openehr-item-tag and openehr-version-item-tag\": \
     openehr-item-tag applies to VERSIONED_OBJECT targets, openehr-version-item-tag to a \
     specific VERSION — each response header confirms only its own target's stored list";

fn raw(h: &header::HeaderMap, name: &str) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_owned)
}

/// The `key="…"` values of an item-tag wrapper header value, in order.
fn tag_keys(value: &str) -> Vec<String> {
    value
        .split(';')
        .filter_map(|entry| {
            entry
                .split(',')
                .map(str::trim)
                .find_map(|pair| pair.strip_prefix("key=").map(|k| k.trim_matches('"')))
        })
        .map(str::to_owned)
        .collect()
}

/// The stored `ITEM_TAG` keys of a COMPOSITION tag target, read back through
/// the dedicated `composition_tags_get` operation.
async fn stored_tag_keys(app: &Router, ehr_id: &str, uid_based_id: &str) -> Vec<String> {
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/composition/{uid_based_id}/tags"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK, "tags read: {body}");
    let tags: Value = serde_json::from_str(&body).expect("json ITEM_TAG list");
    let mut keys: Vec<String> = tags
        .as_array()
        .expect("ITEM_TAG list")
        .iter()
        .map(|t| t["key"].as_str().expect("ITEM_TAG.key").to_owned())
        .collect();
    keys.sort();
    keys
}

/// Both wrapper headers on one write address different targets, so each
/// response header echoes ONLY its own target's stored collection — never the
/// union of the two.
#[tokio::test]
async fn composition_update_echoes_each_item_tag_target_under_its_own_header() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;
    let ovid = commit_composition(&app, &ehr_id).await;
    let vo = vo_of(&ovid).to_owned();

    let mut updated = common::ips_canonical_composition();
    updated.as_object_mut().unwrap().remove("uid");
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{ovid}\""))
        .header("openehr-item-tag", "key=\"category\",value=\"final\"")
        .header(
            "openehr-version-item-tag",
            "key=\"reviewed\",value=\"true\"",
        )
        .body(Body::from(updated.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    // No Prefer header => return=minimal => 204 (overview
    // Requests_and_responses.md §Prefer: "If no `Prefer` header is provided,
    // the default behavior is assumed to be `return=minimal`").
    assert_eq!(status, StatusCode::NO_CONTENT, "composition update: {body}");
    let new_ovid = etag_uid(&h);

    let object_echo = raw(&h, "openehr-item-tag").expect("openehr-item-tag echoed");
    let version_echo =
        raw(&h, "openehr-version-item-tag").expect("openehr-version-item-tag echoed");
    assert_eq!(
        tag_keys(&object_echo),
        vec!["category".to_owned()],
        "{DISTINCT_TARGETS}; got openehr-item-tag: {object_echo:?}"
    );
    assert_eq!(
        tag_keys(&version_echo),
        vec!["reviewed".to_owned()],
        "{DISTINCT_TARGETS}; got openehr-version-item-tag: {version_echo:?}"
    );

    // …and the echo is the truth: the dedicated tag reads show the same split
    // between the VERSIONED_OBJECT and the committed VERSION.
    assert_eq!(
        stored_tag_keys(&app, &ehr_id, &vo).await,
        vec!["category".to_owned()],
        "{DISTINCT_TARGETS}"
    );
    assert_eq!(
        stored_tag_keys(&app, &ehr_id, &new_ovid).await,
        vec!["reviewed".to_owned()],
        "{DISTINCT_TARGETS}"
    );
}

/// A request that carries only one wrapper header writes — and echoes — only
/// that target: "an absent header leaves the other target's tags untouched"
/// follows from the headers addressing distinct targets, so the absent
/// header's collection is not confirmed at all.
#[tokio::test]
async fn composition_update_echoes_nothing_for_an_absent_item_tag_header() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;
    let ovid = commit_composition(&app, &ehr_id).await;
    let vo = vo_of(&ovid).to_owned();

    let mut updated = common::ips_canonical_composition();
    updated.as_object_mut().unwrap().remove("uid");
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{ovid}\""))
        .header(
            "openehr-version-item-tag",
            "key=\"reviewed\",value=\"true\"",
        )
        .body(Body::from(updated.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    // No Prefer header => return=minimal => 204 (overview
    // Requests_and_responses.md §Prefer: "If no `Prefer` header is provided,
    // the default behavior is assumed to be `return=minimal`").
    assert_eq!(status, StatusCode::NO_CONTENT, "composition update: {body}");
    let new_ovid = etag_uid(&h);

    assert_eq!(
        raw(&h, "openehr-version-item-tag").as_deref().map(tag_keys),
        Some(vec!["reviewed".to_owned()]),
        "{DISTINCT_TARGETS}"
    );
    assert!(
        raw(&h, "openehr-item-tag").is_none(),
        "{DISTINCT_TARGETS}: the VERSIONED_OBJECT collection was never addressed, so \
         nothing is confirmed for it — got {:?}",
        raw(&h, "openehr-item-tag")
    );
    assert!(
        stored_tag_keys(&app, &ehr_id, &vo).await.is_empty(),
        "{DISTINCT_TARGETS}: the VERSIONED_OBJECT target stays untagged"
    );
    assert_eq!(
        stored_tag_keys(&app, &ehr_id, &new_ovid).await,
        vec!["reviewed".to_owned()],
        "{DISTINCT_TARGETS}"
    );
}

// ── Preference-Applied (overview §Representation details negotiation) ────────

fn preference_applied(h: &header::HeaderMap) -> Option<String> {
    raw(h, "preference-applied")
}

/// "The service MAY include a `Preference-Applied` header in the response …
/// to indicate that the client's preference has been honored"; "if no `Prefer`
/// header is provided, the default behavior is assumed to be `return=minimal`"
/// — every write path declares what it applied, including the applied default.
#[tokio::test]
async fn writes_declare_the_applied_preference() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;

    // Template upload (the definition group's own identifier body).
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/definition/template/adl1.4"))
        .header(header::CONTENT_TYPE, "application/xml")
        .body(Body::from(common::ips_opt_xml()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    assert_eq!(
        preference_applied(&h).as_deref(),
        Some("return=minimal"),
        "the template upload declares the applied default"
    );

    // COMPOSITION create with an explicit representation preference.
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Prefer", "return=representation")
        .body(Body::from(common::ips_canonical_composition().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "composition commit: {body}");
    assert_eq!(
        preference_applied(&h).as_deref(),
        Some("return=representation")
    );
    let ovid = etag_uid(&h);
    let vo = vo_of(&ovid).to_owned();

    // The ITEM_TAG collection write is not a uid-versioned resource, so it
    // declares minimal / representation only.
    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}/tags"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(
            serde_json::json!([{ "key": "category", "value": "final" }]).to_string(),
        ))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "tags update: {body}");
    assert_eq!(preference_applied(&h).as_deref(), Some("return=minimal"));

    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}/tags"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Prefer", "return=representation")
        .body(Body::from(
            serde_json::json!([{ "key": "category", "value": "final" }]).to_string(),
        ))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "tags update: {body}");
    assert_eq!(
        preference_applied(&h).as_deref(),
        Some("return=representation")
    );
}

/// "This is a variant of preference that implies minimal response semantics,
/// but with a non-empty response body (i.e. the status will be `201 Created`
/// or `200 OK`, never `204 No Content`)" — on the `EHR_STATUS` PUT, whose
/// minimal outcome IS `204`.
#[tokio::test]
async fn ehr_status_update_identifier_is_never_204() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    let (mut status_body, current) = current_ehr_status(&app, &ehr_id).await;
    status_body.as_object_mut().unwrap().remove("uid");

    let req = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/ehr_status"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{current}\""))
        .header("Prefer", "return=identifier")
        .body(Body::from(status_body.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;

    assert_eq!(
        status,
        StatusCode::OK,
        "overview §\"Prefer only identifier\": the status will be 201 Created or 200 OK, \
         never 204 No Content — got {status} with body {body:?}"
    );
    assert_eq!(preference_applied(&h).as_deref(), Some("return=identifier"));
    let uid = etag_uid(&h);
    let v: Value = serde_json::from_str(&body).expect("json identifier body");
    assert_eq!(
        v,
        serde_json::json!({ "uid": uid }),
        "overview §\"Prefer only identifier\": a single JSON object with a single uid attribute"
    );
}

/// The CONTRIBUTION GET carries the contribution-uid weak `ETag` (the same
/// identity the 201's `ETag` carries) and a `Last-Modified` equal to the
/// contribution audit's commit instant — overview §"`ETag` and Last-Modified"
/// (both SHOULD accompany resources with "versioning or unique state
/// identifiers"); the released `200_CONTRIBUTION` declares neither, so the
/// reach of the SHOULD is the adjudicated reading.
#[tokio::test]
async fn contribution_get_carries_etag_and_last_modified() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    let (mut status_body, v1) = current_ehr_status(&app, &ehr_id).await;
    status_body.as_object_mut().unwrap().remove("uid");

    let committer = serde_json::json!({ "_type": "PARTY_IDENTIFIED", "name": "test" });
    let contribution = serde_json::json!({
        "versions": [{
            "data": status_body,
            "preceding_version_uid": { "_type": "OBJECT_VERSION_ID", "value": v1 },
            "lifecycle_state": {
                "_type": "DV_CODED_TEXT", "value": "complete",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "532" }
            },
            "commit_audit": {
                "change_type": {
                    "_type": "DV_CODED_TEXT", "value": "modification",
                    "defining_code": { "_type": "CODE_PHRASE",
                        "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                        "code_string": "251" }
                },
                "committer": committer
            }
        }],
        "audit": {
            "change_type": {
                "_type": "DV_CODED_TEXT", "value": "modification",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "251" }
            },
            "committer": committer
        }
    });
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr_id}/contribution"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(contribution.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    let contribution_uid = etag_uid(&h);

    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/contribution/{contribution_uid}"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, h, body) = send(&app, req).await;
    assert_eq!(status, StatusCode::OK, "get: {body}");
    assert_eq!(
        etag(&h),
        Some(format!("W/\"{contribution_uid}\"").as_str()),
        "the GET's ETag is the contribution uid, weak form"
    );
    let v: Value = serde_json::from_str(&body).expect("json CONTRIBUTION");
    assert_eq!(
        last_modified(&h),
        Some(
            imf_fixdate(
                v["audit"]["time_committed"]["value"]
                    .as_str()
                    .expect("instant")
            )
            .as_str()
        ),
        "Last-Modified is the contribution audit's commit instant"
    );
}

/// A CONTRIBUTION wire body modifying the `EHR_STATUS` `preceding` version.
fn contribution_modifying_status(status_body: &Value, preceding: &str) -> Value {
    let committer = serde_json::json!({ "_type": "PARTY_IDENTIFIED", "name": "test" });
    let modification = serde_json::json!({
        "_type": "DV_CODED_TEXT", "value": "modification",
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": "251" }
    });
    serde_json::json!({
        "versions": [{
            "data": status_body,
            "preceding_version_uid": { "_type": "OBJECT_VERSION_ID", "value": preceding },
            "lifecycle_state": {
                "_type": "DV_CODED_TEXT", "value": "complete",
                "defining_code": { "_type": "CODE_PHRASE",
                    "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                    "code_string": "532" }
            },
            "commit_audit": { "change_type": modification, "committer": committer }
        }],
        "audit": { "change_type": modification, "committer": committer }
    })
}

/// Commit `contribution` with the given `Prefer` value, returning the response
/// headers and body.
async fn commit_contribution(
    app: &Router,
    ehr_id: &str,
    contribution: &Value,
    prefer: &str,
) -> (header::HeaderMap, String) {
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr_id}/contribution"))
        .header(header::CONTENT_TYPE, "application/json")
        .header("Prefer", prefer)
        .body(Body::from(contribution.to_string()))
        .unwrap();
    let (status, h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "commit ({prefer}): {body}");
    (h, body)
}

/// The stored CONTRIBUTION audit's `time_committed`, read back over the wire.
async fn stored_commit_instant(app: &Router, ehr_id: &str, contribution_uid: &str) -> String {
    let req = Request::builder()
        .method("GET")
        .uri(format!(
            "{BASE}/ehr/{ehr_id}/contribution/{contribution_uid}"
        ))
        .body(Body::empty())
        .unwrap();
    let (status, _h, body) = send(app, req).await;
    assert_eq!(status, StatusCode::OK, "contribution read: {body}");
    let v: Value = serde_json::from_str(&body).expect("json CONTRIBUTION");
    v["audit"]["time_committed"]["value"]
        .as_str()
        .expect("CONTRIBUTION.audit.time_committed.value")
        .to_owned()
}

/// The CONTRIBUTION commit response carries `Last-Modified` beside its
/// `ETag`/`Location` under BOTH `Prefer` branches, and its value is the commit
/// audit instant the server actually stored — overview §"`ETag` and
/// Last-Modified": "Both `ETag` and `Last-Modified` SHOULD be included in
/// responses for VERSION, `VERSIONED_OBJECT`, or other resources that have
/// versioning or unique state identifiers", the value "derived from
/// `VERSION.commit_audit.time_committed.value`". The released
/// `201_CONTRIBUTION` declares neither header, so the reach of the SHOULD is
/// the same adjudicated reading the CONTRIBUTION GET applies.
#[tokio::test]
async fn contribution_commit_carries_last_modified_on_both_prefer_branches() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;

    // `return=minimal` — a headers-only 201: the header is the only place the
    // commit instant can reach the client, so it must be there.
    let (mut status_body, v1) = current_ehr_status(&app, &ehr_id).await;
    status_body.as_object_mut().unwrap().remove("uid");
    let (h, _) = commit_contribution(
        &app,
        &ehr_id,
        &contribution_modifying_status(&status_body, &v1),
        "return=minimal",
    )
    .await;
    let minimal_uid = etag_uid(&h);
    let stored = stored_commit_instant(&app, &ehr_id, &minimal_uid).await;
    assert_eq!(
        last_modified(&h),
        Some(imf_fixdate(&stored).as_str()),
        "return=minimal: Last-Modified is the stored contribution audit instant"
    );

    // `return=representation` — the same header beside the served body, equal
    // to the body's own `audit.time_committed`.
    let (mut status_body, v2) = current_ehr_status(&app, &ehr_id).await;
    status_body.as_object_mut().unwrap().remove("uid");
    let (h, body) = commit_contribution(
        &app,
        &ehr_id,
        &contribution_modifying_status(&status_body, &v2),
        "return=representation",
    )
    .await;
    let served: Value = serde_json::from_str(&body).expect("json CONTRIBUTION");
    let instant = served["audit"]["time_committed"]["value"]
        .as_str()
        .expect("CONTRIBUTION.audit.time_committed.value");
    assert_eq!(
        etag(&h),
        Some(format!("W/\"{}\"", etag_uid(&h)).as_str()),
        "the 201 ETag is the contribution uid, weak form"
    );
    assert_eq!(
        last_modified(&h),
        Some(imf_fixdate(instant).as_str()),
        "return=representation: Last-Modified is the served audit's commit instant"
    );
}

/// The tag target guard runs on GET and DELETE too: the released 404 trigger
/// ("…or when the `uid_based_id` does not exist") covers a nonexistent
/// target, a foreign EHR's target, and a wrong-kind target on a typed route
/// (route-kind discipline, adjudicated) — while an EXISTING target
/// with no tags stays an empty `200 []`.
#[tokio::test]
async fn tag_get_and_delete_guard_the_target() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;
    let version_uid = commit_composition(&app, &ehr_id).await;
    let vo = vo_of(&version_uid).to_owned();
    let (_status_body, status_uid) = current_ehr_status(&app, &ehr_id).await;
    let status_vo = vo_of(&status_uid).to_owned();

    // Existing target, no tags → 200 [].
    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}/tags"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "[]", "an existing target with no tags is []");

    // Nonexistent target → 404 (the released trigger).
    let (status, _h, _b) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/composition/00000000-0000-4000-8000-000000000000/tags"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "nonexistent target");

    // Wrong-kind target on a typed route → 404 (an EHR_STATUS container on
    // the composition route).
    let (status, _h, _b) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{status_vo}/tags"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "wrong-kind target");

    // Foreign EHR's target → 404 ("owned by EHR identified by ehr_id").
    let other_ehr = create_ehr(&app).await;
    let (status, _h, _b) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{other_ehr}/composition/{vo}/tags"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "foreign EHR's target");

    // DELETE is kind-checked too: the composition-route DELETE must not
    // touch an EHR_STATUS container's tags.
    let (status, _h, _b) = send(
        &app,
        Request::builder()
            .method("DELETE")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/composition/{status_vo}/tags/anykey"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "wrong-kind DELETE");
}

/// `target_path: ""` on a PUT normalizes to ABSENT (RM models `target_path`
/// 0..1 with no non-empty invariant while the released `EHR_STATUS` example
/// writes "" — one identity, adjudicated): a "" tag and an absent-path
/// tag with the same key are ONE tag, last-wins.
#[tokio::test]
async fn tag_empty_target_path_normalizes_to_absent() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;
    let version_uid = commit_composition(&app, &ehr_id).await;
    let vo = vo_of(&version_uid).to_owned();

    let body = serde_json::json!([
        { "key": "flag", "value": "first", "target_path": "" },
        { "key": "flag", "value": "second" }
    ]);
    let (status, _h, resp) = send(
        &app,
        Request::builder()
            .method("PUT")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}/tags"))
            .header(header::CONTENT_TYPE, "application/json")
            .header("Prefer", "return=representation")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "tag put: {resp}");
    let tags: Value = serde_json::from_str(&resp).expect("tag list");
    let list = tags.as_array().expect("array");
    assert_eq!(
        list.len(),
        1,
        "\"\" and absent are ONE identity — last wins: {resp}"
    );
    assert_eq!(list[0]["value"], "second");
    assert!(
        list[0].get("target_path").is_none(),
        "the normalized tag carries no target_path: {resp}"
    );
}

/// The EHR-wide tag listing guards the EHR itself: an unknown `ehr_id` is
/// the released 404 (`404_unknown_ehr_id`), while an existing EHR with no
/// matching tags stays `200 []`.
#[tokio::test]
async fn ehr_wide_tag_listing_guards_the_ehr() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!("{BASE}/ehr/{ehr_id}/tags"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "[]", "an existing EHR with no tags is []");

    let (status, _h, _b) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/00000000-0000-4000-8000-00000000dead/tags"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "unknown EHR is the released 404"
    );
}

/// The remaining group-8 tag disciplines in one battery: `[]` clear-all (the
/// released sentence), `Prefer: return=identifier` resolving to the applied
/// minimal default (an `ITEM_TAG` has no uid), delete-by-key as a
/// SET delete (the released "resource(s)" plural — every `target_path` under
/// the key goes, and the second delete is the released non-idempotent 404),
/// and the `ehr_tags_get` filters (AND-combined, exact — our own handling).
#[tokio::test]
async fn tag_collection_disciplines() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;
    let version_uid = commit_composition(&app, &ehr_id).await;
    let vo = vo_of(&version_uid).to_owned();
    let put = |body: Value, prefer: Option<&'static str>| {
        let mut b = Request::builder()
            .method("PUT")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}/tags"))
            .header(header::CONTENT_TYPE, "application/json");
        if let Some(p) = prefer {
            b = b.header("Prefer", p);
        }
        b.body(Body::from(body.to_string())).unwrap()
    };

    // Seed: one key on two paths + a second key.
    let seed = serde_json::json!([
        { "key": "flag", "value": "a", "target_path": "/content[0]" },
        { "key": "flag", "value": "b", "target_path": "/content[1]" },
        { "key": "workflow", "value": "open" }
    ]);
    let (status, h, _b) = send(&app, put(seed, Some("return=identifier"))).await;
    // return=identifier resolves to the APPLIED minimal default.
    assert_eq!(status, StatusCode::NO_CONTENT);
    assert_eq!(
        h.get("preference-applied").and_then(|v| v.to_str().ok()),
        Some("return=minimal"),
        "identifier resolves to the applied minimal default"
    );

    // ehr_tags_get filters: AND-combined, exact.
    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("GET")
            .uri(format!(
                "{BASE}/ehr/{ehr_id}/tags?tag_key=flag&tag_target_path=/content%5B1%5D"
            ))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    let filtered: Value = serde_json::from_str(&body).expect("filtered list");
    assert_eq!(
        filtered.as_array().map(Vec::len),
        Some(1),
        "AND + exact: {body}"
    );
    assert_eq!(filtered[0]["value"], "b");

    // DELETE by key is a SET delete: both /content paths go; workflow stays.
    let del = |key: &str| {
        Request::builder()
            .method("DELETE")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}/tags/{key}"))
            .body(Body::empty())
            .unwrap()
    };
    let (status, _h, _b) = send(&app, del("flag")).await;
    assert_eq!(status, StatusCode::NO_CONTENT);
    let keys = stored_tag_keys(&app, &ehr_id, &vo).await;
    assert_eq!(keys, vec!["workflow"], "both flag paths gone in one delete");
    // The released third 404 trigger — deliberately non-idempotent.
    let (status, _h, _b) = send(&app, del("flag")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "second delete is 404");

    // [] clear-all (the released sentence) with the representation split.
    let (status, _h, body) = send(
        &app,
        put(serde_json::json!([]), Some("return=representation")),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body.trim(), "[]", "an empty list clears all: {body}");
}

/// A request header whose value is not decodable as text is REFUSED, never
/// silently skipped — a skipped committal/tag header would commit a version
/// whose attributes differ from the ones the client sent, with nothing on the
/// wire saying so. (No openEHR spec governs undecodable header bytes — our own
/// design.)
#[tokio::test]
async fn an_undecodable_header_value_is_refused_not_dropped() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;

    // `0xFF` is not valid in a `to_str`-decodable header value.
    let opaque = http::HeaderValue::from_bytes(&[0xff]).expect("a byte header value");

    for header_name in [
        "openehr-audit-details",
        "openehr-item-tag",
        "openEHR-VERSION.lifecycle_state",
    ] {
        let req = Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(header_name, opaque.clone())
            .body(Body::from(common::ips_canonical_composition().to_string()))
            .unwrap();
        let (status, _h, body) = send(&app, req).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "an undecodable {header_name} value must be refused, got {status} {body}"
        );
    }

    // The twin: the same write with no undecodable header commits.
    let req = Request::builder()
        .method("POST")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
        .header(header::CONTENT_TYPE, "application/json")
        .body(Body::from(common::ips_canonical_composition().to_string()))
        .unwrap();
    let (status, _h, body) = send(&app, req).await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the clean twin commits: {body}"
    );
}

/// An EMPTIED stored collection is echoed as NO header at all: the empty
/// wrapper-header value is the release's "remove all `ITEM_TAG`s" REQUEST
/// instruction (overview `Requests_and_responses.md` §"openehr-item-tag and
/// openehr-version-item-tag": "Providing an empty value for this header will
/// effectively remove all `ITEM_TAG`s"), so a response echoing one would hand
/// a mirroring client the destructive form as if it were state. The guard is
/// `params::emit_item_tag_header`, shared by both echo paths.
#[tokio::test]
async fn an_emptied_item_tag_collection_echoes_no_header_not_an_empty_one() {
    let (_pg, app) = app().await;
    let ehr_id = create_ehr(&app).await;
    upload_opt(&app).await;
    let ovid = commit_composition(&app, &ehr_id).await;
    let vo = vo_of(&ovid).to_owned();

    // Tag the VERSIONED_OBJECT, then update with an EMPTY openehr-item-tag —
    // the remove-all instruction — so the stored collection empties.
    let mut updated = common::ips_canonical_composition();
    updated.as_object_mut().unwrap().remove("uid");
    let tag_first = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{ovid}\""))
        .header("openehr-item-tag", "key=\"category\",value=\"final\"")
        .body(Body::from(updated.clone().to_string()))
        .unwrap();
    let (status, h, body) = send(&app, tag_first).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "tagging update: {body}");
    let second_ovid = etag_uid(&h);

    let wipe = Request::builder()
        .method("PUT")
        .uri(format!("{BASE}/ehr/{ehr_id}/composition/{vo}"))
        .header(header::CONTENT_TYPE, "application/json")
        .header(header::IF_MATCH, format!("\"{second_ovid}\""))
        .header("openehr-item-tag", "")
        .body(Body::from(updated.to_string()))
        .unwrap();
    let (status, h, body) = send(&app, wipe).await;
    assert_eq!(status, StatusCode::NO_CONTENT, "remove-all update: {body}");
    assert!(
        raw(&h, "openehr-item-tag").is_none(),
        "an emptied collection must echo NO header — an empty value is the \
         remove-all REQUEST instruction, not state; got {:?}",
        raw(&h, "openehr-item-tag")
    );
    // …and the wipe really happened.
    assert_eq!(
        stored_tag_keys(&app, &ehr_id, &vo).await,
        Vec::<String>::new(),
        "the empty header removed the VERSIONED_OBJECT's tags"
    );
}
