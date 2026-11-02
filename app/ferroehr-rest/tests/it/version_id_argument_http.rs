// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A malformed version id in a call argument (a path segment, an `If-Match`
//! value) is a `400` on every resource family that takes one.
//!
//! The probes pass the BASE grammar and fail only the storage decode: a
//! `version_tree_id` past the `i32` columns, so the adapter forwards them and
//! the service's own argument parse is what answers. A malformed id inside
//! submitted content is a `422` and is pinned beside its commit tests.

use axum::Router;
use http::StatusCode;

use crate::common;

/// A well-formed `OBJECT_VERSION_ID` whose trunk version does not fit storage.
fn out_of_range(vo: &str) -> String {
    format!("{vo}::ferroehr.test::99999999999")
}

/// A versioned-object id no write ever creates.
const VO: &str = "5b4a1c1e-8f0e-4d4b-9a3a-0c0d0e0f1a2b";

/// `GET` `relative` under its own namespace and expect a `400`.
async fn assert_400(app: &Router, relative: &str, what: &str) {
    let req = common::get(&common::full(relative));
    let (status, body) = common::send_body(app, req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{what}: a malformed version-id argument is a 400: {body}"
    );
}

#[tokio::test]
async fn an_ehr_status_version_argument_out_of_range_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let ehr = common::create_ehr(&app).await;
    let uri = format!("/ehr/{ehr}/ehr_status/{}", out_of_range(VO));
    assert_400(&app, &uri, "EHR_STATUS by version").await;
    let uri = format!(
        "/ehr/{ehr}/versioned_ehr_status/version/{}",
        out_of_range(VO)
    );
    assert_400(&app, &uri, "VERSIONED_EHR_STATUS version").await;
}

#[tokio::test]
async fn a_composition_version_argument_out_of_range_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let ehr = common::create_ehr(&app).await;
    let uri = format!(
        "/ehr/{ehr}/versioned_composition/{VO}/version/{}",
        out_of_range(VO)
    );
    assert_400(&app, &uri, "VERSIONED_COMPOSITION version").await;
}

#[tokio::test]
async fn a_directory_version_argument_out_of_range_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let ehr = common::create_ehr(&app).await;
    let uri = format!("/ehr/{ehr}/directory/{}", out_of_range(VO));
    assert_400(&app, &uri, "DIRECTORY by version").await;
}

#[tokio::test]
async fn a_malformed_contribution_uid_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let ehr = common::create_ehr(&app).await;
    let uri = format!("/ehr/{ehr}/contribution/not-a-uuid");
    assert_400(&app, &uri, "EHR CONTRIBUTION").await;
    assert_400(
        &app,
        "/demographic/contribution/not-a-uuid",
        "demographic CONTRIBUTION",
    )
    .await;
}

#[tokio::test]
async fn a_party_version_argument_out_of_range_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let uri = format!("/demographic/person/{}", out_of_range(VO));
    assert_400(&app, &uri, "PERSON by version").await;
    assert_400(&app, "/demographic/person/not-a-uuid", "PERSON by uid").await;
}

#[tokio::test]
async fn a_relationship_version_argument_out_of_range_is_400() {
    let (_db, app) = common::app(common::api_config(false)).await;
    let uri = format!("/demographic/party_relationship/{}", out_of_range(VO));
    assert_400(&app, &uri, "PARTY_RELATIONSHIP by version").await;
    assert_400(
        &app,
        "/demographic/party_relationship/not-a-uuid",
        "PARTY_RELATIONSHIP by uid",
    )
    .await;
}
