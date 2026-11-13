// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end HTTP tests for the ADMIN access-log routes (#5129):
//! `POST /admin/access_log/checkpoint`, `POST /admin/access_log/verify` and
//! `GET /admin/access_log/export`. **Our own extension**; no ITS-REST
//! operation governs them. The signing and the transfer themselves are
//! proven in the platform crate's `access_log_checkpoint` suite; these prove
//! the routes and their answers.

use std::sync::Arc;

use axum::body::Body;
use http::{Request, StatusCode};
use serde_json::{Value, json};
use tower::ServiceExt;

use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log_draft::access_log::access_log_entry::AccessLogEntry;
use ferroehr::versioning::signature::signer::Signer;
use ferroehr::versioning::signature::trust::TrustedCertificates;

use crate::common;
use crate::common::EXT;

/// The EHR the logged access reached.
const EHR: &str = "7d44b88c-4199-4bad-97dc-d78268e01398";

/// A coded term of the draft's `openehr` groups.
fn coded(value: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": value,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": value } })
}

/// One granted read of [`EHR`] by `system_id`.
fn entry(system_id: &str) -> AccessLogEntry {
    serde_json::from_value(json!({
        "_type": "ACCESS_LOG_ENTRY",
        "uid": { "_type": "HIER_OBJECT_ID", "value": "0193f1a2-7c1e-7b4a-9f00-2c6d1e8b5a01" },
        "system_id": system_id,
        "time_recorded": { "_type": "DV_DATE_TIME", "value": "2026-11-07T09:14:03.120Z" },
        "action": coded("read"),
        "outcome": { "_type": "ACCESS_OUTCOME", "result": coded("granted") },
        "accessor": { "_type": "ACCESSOR", "kind": coded("person"),
            "party": { "_type": "PARTY_IDENTIFIED", "name": "a professional" } },
        "ehr_id": { "_type": "HIER_OBJECT_ID", "value": EHR },
        "is_emergency": false,
        "retention": { "_type": "ACCESS_RETENTION",
            "retain_until": { "_type": "DV_DATE", "value": "2031-11-07" },
            "grounds": [coded("read")] },
        "link": { "_type": "ACCESS_LOG_LINK", "sequence": 1, "algorithm": "SHA-256",
            "previous_digest": "", "content_digest": "", "digest": "" }
    }))
    .expect("the entry reads")
}

/// A request to `method` the access-log route `path`.
fn request(method: &str, path: &str) -> Request<Body> {
    Request::builder()
        .method(method)
        .uri(format!("{EXT}/admin/access_log{path}"))
        .body(Body::empty())
        .expect("request")
}

/// The status and JSON body of `response`.
async fn answer(response: axum::response::Response) -> (StatusCode, Value) {
    let status = response.status();
    let bytes = axum::body::to_bytes(response.into_body(), usize::MAX)
        .await
        .expect("body");
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    )
}

/// With a keyring that holds no `OpenPGP` key the routes answer what they
/// can: an empty log has nothing to seal, a non-empty one cannot be sealed,
/// a verification reports no checkpoint, and an export names its refusal.
#[tokio::test]
async fn the_access_log_routes_answer_without_a_signing_key() {
    let (pg, service) = common::test_service().await;
    let checkpoints = Checkpoints::new(
        pg.pool(),
        "cdr.example.org".to_owned(),
        Arc::new(Signer::digest_default()),
        Arc::new(TrustedCertificates::default()),
        false,
    );
    let service = Arc::new(
        Arc::try_unwrap(service)
            .expect("the service is not shared yet")
            .with_access_log(checkpoints.clone()),
    );
    let app = common::router_with(common::api_config(true), service).await;

    let (status, _) = answer(
        app.clone()
            .oneshot(request("POST", "/checkpoint"))
            .await
            .expect("response"),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "an empty log seals nothing");

    checkpoints
        .store()
        .append(vec![entry("cdr.example.org")])
        .await
        .expect("append");
    let (status, _) = answer(
        app.clone()
            .oneshot(request("POST", "/checkpoint"))
            .await
            .expect("response"),
    )
    .await;
    assert_eq!(status, StatusCode::CONFLICT, "no key, no checkpoint");

    let (status, body) = answer(
        app.clone()
            .oneshot(request("POST", "/verify"))
            .await
            .expect("response"),
    )
    .await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["intact"], json!(false), "{body}");
    assert_eq!(body["checkpoint"], Value::Null, "{body}");
    assert_eq!(body["chain"], json!([]), "{body}");

    let (status, _) = answer(
        app.clone()
            .oneshot(request("GET", "/export"))
            .await
            .expect("response"),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "the EHR is required");
    let (status, _) = answer(
        app.clone()
            .oneshot(request(
                "GET",
                "/export?ehr_id=00000000-0000-4000-8000-000000000000",
            ))
            .await
            .expect("response"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::NOT_FOUND,
        "an EHR the log holds nothing of"
    );
    let (status, _) = answer(
        app.oneshot(request("GET", &format!("/export?ehr_id={EHR}")))
            .await
            .expect("response"),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CONFLICT,
        "no checkpoint covers it, and none can be sealed"
    );
}

/// A server that wired no access log says so rather than answering empty.
#[tokio::test]
async fn an_unwired_access_log_is_unavailable() {
    let (_pg, service) = common::test_service().await;
    let app = common::router_with(common::api_config(true), service).await;
    let (status, _) = answer(
        app.oneshot(request("POST", "/verify"))
            .await
            .expect("response"),
    )
    .await;
    assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
}
