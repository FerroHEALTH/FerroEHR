// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals of the QUERY API on the wire: a paging or scope argument the
//! server cannot honour is a `400` (ITS-REST `400_Query`), and a scope or
//! stored query naming nothing is a `404`.

use axum::Router;
use http::StatusCode;

use crate::common::{self, BASE};

const EHR_IDS: &str = "SELECT%20e/ehr_id/value%20FROM%20EHR%20e";

async fn status_of(app: &Router, uri: &str) -> (StatusCode, String) {
    common::send_body(app, common::get(uri)).await
}

#[tokio::test]
async fn a_negative_fetch_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let (status, body) = status_of(&app, &format!("{BASE}/query/aql?q={EHR_IDS}&fetch=-1")).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn fetch_with_aql_top_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let uri =
        format!("{BASE}/query/aql?q=SELECT%20TOP%201%20e/ehr_id/value%20FROM%20EHR%20e&fetch=1");
    let (status, body) = status_of(&app, &uri).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}

#[tokio::test]
async fn a_scope_naming_an_unknown_ehr_is_404() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let uri = format!("{BASE}/query/aql?q={EHR_IDS}&ehr_id=6a1f8c2e-0b3d-4e5f-9a7b-1c2d3e4f5a6b");
    let (status, body) = status_of(&app, &uri).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}

#[tokio::test]
async fn an_unknown_stored_query_is_404() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let (status, body) = status_of(&app, &format!("{BASE}/query/org.example::nothing")).await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
}
