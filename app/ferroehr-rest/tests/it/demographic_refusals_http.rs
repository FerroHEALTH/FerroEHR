// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals of the demographic reads on the wire: a versioned party or
//! relationship this server does not hold is a `404`, and a malformed
//! `version_at_time` is a `400`.

use axum::Router;
use http::StatusCode;

use crate::common;

/// A versioned-object id no write ever creates.
const VO: &str = "7d3c2b1a-0f9e-4d8c-b7a6-5e4d3c2b1a0f";

async fn status_of(app: &Router, relative: &str) -> (StatusCode, String) {
    common::send_body(app, common::get(&common::full(relative))).await
}

#[tokio::test]
async fn an_unknown_versioned_party_is_404() {
    let (_db, app) = common::app(common::api_config(false)).await;
    for path in [
        format!("/demographic/versioned_party/{VO}"),
        format!("/demographic/versioned_party/{VO}/revision_history"),
        format!("/demographic/versioned_party/{VO}/version"),
    ] {
        let (status, body) = status_of(&app, &path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }
}

#[tokio::test]
async fn an_unknown_versioned_relationship_is_404() {
    let (_db, app) = common::app(common::api_config(false)).await;
    for path in [
        format!("/demographic/versioned_party_relationship/{VO}"),
        format!("/demographic/versioned_party_relationship/{VO}/revision_history"),
    ] {
        let (status, body) = status_of(&app, &path).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{path}: {body}");
    }
}

#[tokio::test]
async fn a_malformed_version_at_time_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let path = format!("/demographic/person/{VO}?version_at_time=not-a-time");
    let (status, body) = status_of(&app, &path).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
}
