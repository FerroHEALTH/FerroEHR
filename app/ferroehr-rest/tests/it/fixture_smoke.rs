// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Smoke test for the shared real-PG fixture: the assembled router answers on
//! a spec route over a real service + database.

use axum::body::Body;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use tower::ServiceExt;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

#[tokio::test]
async fn ehr_get_unknown_is_404_and_create_roundtrips() {
    let (_pg, app) = common::test_router().await;

    let resp = app
        .clone()
        .oneshot(
            Request::get(format!("{BASE}/ehr/3fa85f64-5717-4562-b3fc-2c963f66afa6"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::NOT_FOUND);

    // Default `Prefer: return=minimal`: 201, empty body, ETag carries the
    // new ehr_id (ITS-REST EHR API, `POST /ehr`).
    let resp = app
        .clone()
        .oneshot(
            Request::post(format!("{BASE}/ehr"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    assert!(resp.headers().contains_key(http::header::ETAG));

    // `Prefer: return=representation`: the created EHR comes back.
    let resp = app
        .oneshot(
            Request::post(format!("{BASE}/ehr"))
                .header("Prefer", "return=representation")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(resp.status(), StatusCode::CREATED);
    let body = resp.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["_type"], "EHR");
}
