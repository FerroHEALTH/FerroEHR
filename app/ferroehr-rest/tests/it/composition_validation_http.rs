// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the COMPOSITION semantic-validation response —
//! the `422` an RM class-invariant violation produces, driven through the
//! assembled router over a **real** `FerroEhrService` on a real `PostgreSQL`.
//!
//! Wire oracle: ITS-REST 1.1.0
//! `specifications/responses/422.yaml` ("content could be
//! converted to a COMPOSITION, but there are semantic validation errors") and
//! `schemas/others/Error.yaml` (the `Error` object's `validationErrors`
//! list). The RM rule under test is `data_structures` §ELEMENT
//! `Inv_null_flavour_indicated`.
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

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

fn opt_xml() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt"),
    )
    .expect("ips.v0.opt vendored in openehr-its")
}

fn canonical_composition() -> Value {
    let text = std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
            "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
        ),
    )
    .expect("ips_canonical.json vendored in openehr-its");
    serde_json::from_str(&text).expect("valid canonical composition")
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

fn etag_uid(h: &header::HeaderMap) -> String {
    h.get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag present")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

/// A fresh EHR with the IPS operational template uploaded.
async fn app_with_template() -> (testkit::TestDb, Router, String) {
    let (pg, app) = common::test_router().await;
    let (status, h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "ehr create: {body}");
    let ehr_id = etag_uid(&h);

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/definition/template/adl1.4"))
            .header(header::CONTENT_TYPE, "application/xml")
            .body(Body::from(opt_xml()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    (pg, app, ehr_id)
}

/// Strip the `value` of the first ELEMENT that has one, leaving no
/// `null_flavour` behind — the shape RM `data_structures` §ELEMENT
/// `Inv_null_flavour_indicated` forbids.
fn strip_first_element_value(v: &mut Value) -> bool {
    match v {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("ELEMENT")
                && map.get("null_flavour").is_none()
                && map.remove("value").is_some()
            {
                return true;
            }
            let keys: Vec<String> = map.keys().cloned().collect();
            keys.into_iter()
                .any(|k| map.get_mut(&k).is_some_and(strip_first_element_value))
        }
        Value::Array(items) => items.iter_mut().any(strip_first_element_value),
        _ => false,
    }
}

/// A COMPOSITION holding an ELEMENT with neither `value` nor `null_flavour`
/// violates RM `data_structures` §ELEMENT:
///
/// > `Inv_null_flavour_indicated`: `is_null() xor null_flavour = Void`
/// > (`RM/docs/UML/classes/org.openehr.rm.data_structures.element.adoc`)
///
/// The commit is a `422` (ITS-REST `responses/422.yaml`: converts,
/// but does not validate) whose `Error` body carries the violation in
/// `validationErrors`, keyed by the offending RM path and naming the
/// invariant — so the client learns which node is wrong and why, instead of
/// the element being silently dropped.
#[tokio::test]
async fn composition_with_a_value_less_element_is_422_naming_the_invariant() {
    let (_pg, app, ehr_id) = app_with_template().await;

    let mut broken = canonical_composition();
    assert!(
        strip_first_element_value(&mut broken),
        "the IPS composition carries a valued ELEMENT to empty"
    );

    let (status, _h, body) = send(
        &app,
        Request::builder()
            .method("POST")
            .uri(format!("{BASE}/ehr/{ehr_id}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(broken.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::UNPROCESSABLE_ENTITY,
        "an RM-invalid ELEMENT must not commit: {body}"
    );

    let error: Value = serde_json::from_str(&body).expect("the 422 carries an Error object");
    let entries = error["validationErrors"]
        .as_array()
        .expect("the Error object lists validationErrors");
    let named = entries
        .iter()
        .filter_map(Value::as_str)
        .find(|e| e.contains("Inv_null_flavour_indicated"))
        .unwrap_or_else(|| panic!("no entry names the invariant: {entries:?}"));
    assert!(
        named.contains(": "),
        "each validationErrors entry is `<rm path>: <message>`, got {named:?}"
    );
}
