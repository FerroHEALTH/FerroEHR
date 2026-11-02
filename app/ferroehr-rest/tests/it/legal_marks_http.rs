// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the legal marks — the ADMIN routes that set them
//! and the wire answer of every released operation a restriction refuses.
//!
//! **No openEHR spec governs any of this** (the released Admin API is exactly
//! `admin_ehr_delete` + `admin_ehr_delete_all`, and no ITS-REST operation
//! defines restriction of processing, a research objection or a retention
//! period), so both the admin wire and the `403` the released operations answer
//! are our own design/extension. What the released text does settle is the
//! shape of the refusal: `403` is the status
//! `docs/specs/openehr/ITS-REST/specifications/docs/overview/Requests_and_responses.md`
//! and RFC 9110 §15.5.4 give a request the server understood and declines, and
//! it stays distinct from the `404` an unknown resource answers — a restricted
//! record exists, and saying otherwise would be a different (wrong) statement
//! about the patient.
//!
//! The obligation under test is GDPR Art. 18(2), which leaves storage as the
//! only processing a restriction admits (`docs/law/eu/gdpr/text.html`).

use axum::body::Body;
use http::{Request, StatusCode};

use crate::common;
use crate::common::BASE;
use crate::common::{create_ehr, send_body};

fn get(path: &str) -> Request<Body> {
    common::get(&common::full(path))
}

fn post_json(path: &str, body: &str) -> Request<Body> {
    common::post_json(&common::full(path), body)
}

fn put_json(path: &str, body: &str) -> Request<Body> {
    let mut req = common::post_json(&common::full(path), body);
    *req.method_mut() = http::Method::PUT;
    req
}

/// Every released operation a whole-EHR restriction reaches answers `403`, and
/// the same operations answer normally once it is lifted.
///
/// One case per operation rather than one broad assertion: a red row then names
/// the operation whose gate is missing.
#[tokio::test]
async fn a_restricted_ehr_refuses_every_released_read_and_write_with_403() {
    let (_pg, app) = common::app(common::api_config(true)).await;
    let ehr_id = create_ehr(&app).await;

    // The directory has to exist before the restriction, or its refusal would
    // be indistinguishable from a 404.
    let (status, body) = send_body(
        &app,
        post_json(
            &format!("/ehr/{ehr_id}/directory"),
            r#"{"_type":"FOLDER","archetype_node_id":"openEHR-EHR-FOLDER.generic.v1", "archetype_details": { "_type": "ARCHETYPED", "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.generic.v1" }, "rm_version": "1.1.0" },
                "name":{"_type":"DV_TEXT","value":"root"}}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "directory create: {body}");

    let reads = [
        format!("/ehr/{ehr_id}/ehr_status"),
        format!("/ehr/{ehr_id}/versioned_ehr_status/revision_history"),
        format!("/ehr/{ehr_id}/versioned_ehr_status/version"),
        format!("/ehr/{ehr_id}/directory"),
    ];
    let mut status_body = serde_json::Value::Null;
    let mut status_uid = String::new();
    for path in &reads {
        let (status, body) = send_body(&app, get(path)).await;
        assert_eq!(status, StatusCode::OK, "before the mark, {path}: {body}");
        if path.ends_with("/ehr_status") {
            let mut value: serde_json::Value = serde_json::from_str(&body).expect("status json");
            status_uid = value["uid"]["value"]
                .as_str()
                .expect("status uid")
                .to_owned();
            value.as_object_mut().expect("status object").remove("uid");
            status_body = value;
        }
    }

    let (status, body) = send_body(
        &app,
        post_json(
            "/admin/restriction",
            &format!(r#"{{"ehr_id":"{ehr_id}","ground":"gdpr-18-1-a"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "restrict: {body}");

    for path in &reads {
        let (status, body) = send_body(&app, get(path)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
        assert!(
            body.contains("restricted"),
            "the body names the restriction rather than an authorization \
             outcome, {path}: {body}"
        );
    }

    // A write is refused with the same status, and nothing is committed. The
    // body and its precondition are the ones read back before the mark, so the
    // refusal is the restriction's and not a precondition standing in for it.
    let req = common::req("PUT", format!("{BASE}/ehr/{ehr_id}/ehr_status"))
        .header(http::header::CONTENT_TYPE, "application/json")
        .header(http::header::IF_MATCH, format!("\"{status_uid}\""))
        .body(status_body.to_string())
        .build();
    let (status, body) = send_body(&app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a write is refused: {body}");

    // Lifting restores every one of them.
    let (status, body) = send_body(
        &app,
        post_json(
            "/admin/restriction/lift",
            &format!(r#"{{"ehr_id":"{ehr_id}"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "lift: {body}");
    for path in &reads {
        let (status, body) = send_body(&app, get(path)).await;
        assert_eq!(status, StatusCode::OK, "after the lift, {path}: {body}");
    }
}

/// The register is readable, and a lift stamps the row rather than removing it
/// — the sequence GDPR Art. 18(3) turns on.
#[tokio::test]
async fn the_register_keeps_the_request_after_the_lift() {
    let (_pg, app) = common::app(common::api_config(true)).await;
    let ehr_id = create_ehr(&app).await;

    send_body(
        &app,
        post_json(
            "/admin/restriction",
            &format!(r#"{{"ehr_id":"{ehr_id}","ground":"gdpr-18-1-c","note":"legal claim"}}"#),
        ),
    )
    .await;
    send_body(
        &app,
        post_json(
            "/admin/restriction/lift",
            &format!(r#"{{"ehr_id":"{ehr_id}"}}"#),
        ),
    )
    .await;

    let (status, body) = send_body(&app, get(&format!("/admin/restriction?ehr_id={ehr_id}"))).await;
    assert_eq!(status, StatusCode::OK, "register: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).expect("register json");
    let rows = value["restrictions"].as_array().expect("restrictions");
    assert_eq!(rows.len(), 1, "the lift did not erase the request: {body}");
    assert_eq!(rows[0]["ground"], serde_json::json!("gdpr-18-1-c"));
    assert_eq!(rows[0]["note"], serde_json::json!("legal claim"));
    assert!(!rows[0]["lifted_at"].is_null(), "the lift is stamped");
}

/// The admin routes refuse a malformed body, an unknown id and a ground outside
/// the closed list before anything is recorded.
#[tokio::test]
async fn the_mark_routes_refuse_a_bad_request_before_recording_anything() {
    let (_pg, app) = common::app(common::api_config(true)).await;
    let ehr_id = create_ehr(&app).await;

    let cases = [
        (
            "/admin/restriction",
            format!(r#"{{"ehr_id":"{ehr_id}"}}"#),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/admin/restriction",
            format!(r#"{{"ehr_id":"{ehr_id}","ground":"whatever"}}"#),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/admin/restriction",
            r#"{"ehr_id":"00000000-0000-0000-0000-000000000000","ground":"national"}"#.to_owned(),
            StatusCode::NOT_FOUND,
        ),
        (
            "/admin/research-objection",
            format!(r#"{{"ehr_id":"{ehr_id}"}}"#),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/admin/research-objection",
            format!(r#"{{"ehr_id":"{ehr_id}","objected":false,"ground":"still important"}}"#),
            StatusCode::BAD_REQUEST,
        ),
        (
            "/admin/retention/hold",
            r#"{"vo_id":"00000000-0000-0000-0000-000000000000","held":true}"#.to_owned(),
            StatusCode::NOT_FOUND,
        ),
    ];
    for (path, body, expected) in cases {
        let (status, answer) = send_body(&app, post_json(path, &body)).await;
        assert_eq!(status, expected, "{path} with {body}: {answer}");
    }

    let (status, body) = send_body(&app, get(&format!("/admin/restriction?ehr_id={ehr_id}"))).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("\"restrictions\":[]"),
        "a refused request recorded nothing: {body}"
    );
}

/// The retention register round-trips over the wire, and the due list carries
/// the per-object hold counts.
#[tokio::test]
async fn the_retention_register_round_trips_and_lists_what_is_due() {
    let (_pg, app) = common::app(common::api_config(true)).await;
    let ehr_id = create_ehr(&app).await;

    let (status, body) = send_body(
        &app,
        put_json(
            "/admin/retention/policy",
            r#"{"kind":"EHR","jurisdiction":"CH","period":"20 years",
                "anchor":"last_commit","source":"EPDV Art. 10 Abs. 1 lit. d"}"#,
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "declare: {body}");

    let (status, body) = send_body(&app, get("/admin/retention/policy")).await;
    assert_eq!(status, StatusCode::OK);
    assert!(
        body.contains("EPDV Art. 10 Abs. 1 lit. d"),
        "the register carries the citation the period rests on: {body}"
    );

    // A hold with only half its pair is refused: a hold with no stated reason
    // is not a record of anything.
    let (status, body) = send_body(
        &app,
        put_json(
            "/admin/retention/anchor",
            &format!(
                r#"{{"ehr_id":"{ehr_id}","jurisdiction":"CH","hold_at":"2026-09-15T00:00:00Z"}}"#
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "half a hold: {body}");

    let (status, body) = send_body(
        &app,
        put_json(
            "/admin/retention/anchor",
            &format!(
                r#"{{"ehr_id":"{ehr_id}","jurisdiction":"CH",
                    "anchored_at":"1990-01-01T00:00:00Z"}}"#
            ),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "anchor: {body}");

    let (status, body) = send_body(&app, get("/admin/retention/due?limit=10")).await;
    assert_eq!(status, StatusCode::OK, "due: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).expect("due json");
    let rows = value["due"].as_array().expect("due rows");
    assert_eq!(rows.len(), 1, "the anchored EHR is due: {body}");
    assert_eq!(rows[0]["ehr_id"], serde_json::json!(ehr_id));
    assert_eq!(rows[0]["objects_held"], serde_json::json!(0));

    // Listing deleted nothing: the record still serves its content.
    let (status, body) = send_body(&app, get(&format!("/ehr/{ehr_id}/ehr_status"))).await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the due EHR is a list entry, not a disposal: {body}"
    );

    let (status, body) = send_body(&app, get("/admin/retention/due?limit=nonsense")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "bad limit: {body}");
}

/// The whole family answers `405` while the admin API is off, like every other
/// route under the same switch.
#[tokio::test]
async fn the_mark_routes_are_behind_the_admin_switch() {
    let (pg, app) = common::app(common::api_config(false)).await;
    drop(pg);

    for req in [
        get("/admin/restriction?ehr_id=00000000-0000-0000-0000-000000000000"),
        get("/admin/retention/policy"),
        get("/admin/retention/due"),
        post_json(
            "/admin/restriction",
            r#"{"ehr_id":"x","ground":"national"}"#,
        ),
        post_json(
            "/admin/research-objection",
            r#"{"ehr_id":"x","objected":true}"#,
        ),
    ] {
        let (status, body) = send_body(&app, req).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED, "{body}");
    }
}

/// A restriction at SINGLE-OBJECT grain, on the operations that address a
/// COMPOSITION: the restricted composition is refused while the rest of the
/// record stays in use, which is the case GDPR Art. 18(1)(a) describes — a
/// subject contesting the accuracy of one entry.
#[tokio::test]
async fn a_restricted_composition_is_refused_while_the_record_stays_in_use() {
    let (_pg, app) = common::app(common::api_config(true)).await;

    let req = common::req("POST", format!("{BASE}/definition/template/adl1.4"))
        .header(http::header::CONTENT_TYPE, "application/xml")
        .body(common::ips_opt_xml())
        .build();
    let (status, body) = send_body(&app, req).await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");

    let ehr_id = create_ehr(&app).await;
    let mut commit = post_json(
        &format!("/ehr/{ehr_id}/composition"),
        &common::ips_canonical_composition().to_string(),
    );
    commit.headers_mut().insert(
        "Prefer",
        http::HeaderValue::from_static("return=representation"),
    );
    let (status, body) = send_body(&app, commit).await;
    assert_eq!(status, StatusCode::CREATED, "composition commit: {body}");
    let value: serde_json::Value = serde_json::from_str(&body).expect("composition json");
    let ovid = value["uid"]["value"].as_str().expect("uid").to_owned();
    let vo_id = ovid.split("::").next().expect("object id").to_owned();

    let reads = [
        format!("/ehr/{ehr_id}/composition/{vo_id}"),
        format!("/ehr/{ehr_id}/versioned_composition/{vo_id}/revision_history"),
        format!("/ehr/{ehr_id}/versioned_composition/{vo_id}/version"),
        format!("/ehr/{ehr_id}/versioned_composition/{vo_id}/version/{ovid}"),
    ];
    for path in &reads {
        let (status, body) = send_body(&app, get(path)).await;
        assert_eq!(status, StatusCode::OK, "before the mark, {path}: {body}");
    }

    let (status, body) = send_body(
        &app,
        post_json(
            "/admin/restriction",
            &format!(r#"{{"ehr_id":"{ehr_id}","vo_id":"{vo_id}","ground":"gdpr-18-1-a"}}"#),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "restrict: {body}");

    for path in &reads {
        let (status, body) = send_body(&app, get(path)).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{path}: {body}");
        assert!(body.contains("restricted"), "{path}: {body}");
    }

    // A DELETE of the restricted composition is refused too: a logical delete
    // commits a new version, which is processing Art. 18(2) does not admit.
    let req = common::req("DELETE", format!("{BASE}/ehr/{ehr_id}/composition/{ovid}")).build();
    let (status, body) = send_body(&app, req).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "a delete is refused: {body}");
    // The write raises the restriction as a `ServiceError`, and it answers
    // with the restriction's own problem type, as the reads above do.
    let kind = serde_json::from_str::<serde_json::Value>(&body).expect("problem json")["type"]
        .as_str()
        .expect("a problem type")
        .to_owned();
    assert!(
        kind.ends_with("processing-restricted"),
        "a restricted write names the restriction, not an authorization outcome: {kind}"
    );

    // The rest of the record is untouched: this is the grain the whole issue
    // turns on.
    let (status, body) = send_body(&app, get(&format!("/ehr/{ehr_id}/ehr_status"))).await;
    assert_eq!(status, StatusCode::OK, "the EHR_STATUS still reads: {body}");

    send_body(
        &app,
        post_json(
            "/admin/restriction/lift",
            &format!(r#"{{"ehr_id":"{ehr_id}","vo_id":"{vo_id}"}}"#),
        ),
    )
    .await;
    let (status, body) = send_body(&app, get(&reads[0])).await;
    assert_eq!(status, StatusCode::OK, "after the lift: {body}");
}
