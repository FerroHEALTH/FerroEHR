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
use axum::body::Body;
use http::{Request, StatusCode};

use crate::common::{self, BASE};

/// A well-formed `OBJECT_VERSION_ID` whose trunk version does not fit storage.
fn out_of_range(vo: &str) -> String {
    format!("{vo}::ferroehr.test::99999999999")
}

/// A versioned-object id no write ever creates.
const VO: &str = "5b4a1c1e-8f0e-4d4b-9a3a-0c0d0e0f1a2b";

async fn app() -> (testkit::TestDb, Router) {
    let (db, service) = common::test_service().await;
    (
        db,
        common::router_with(common::api_config(false), service).await,
    )
}

async fn assert_400(app: &Router, req: Request<Body>, what: &str) {
    let (status, body) = common::send_body(app, req).await;
    assert_eq!(
        status,
        StatusCode::BAD_REQUEST,
        "{what}: a malformed version-id argument is a 400: {body}"
    );
}

#[tokio::test]
async fn an_ehr_status_version_argument_out_of_range_is_400() {
    let (_db, app) = app().await;
    let ehr = common::create_ehr(&app).await;
    let uri = format!("{BASE}/ehr/{ehr}/ehr_status/{}", out_of_range(VO));
    assert_400(&app, common::get(&uri), "EHR_STATUS by version").await;
    let uri = format!(
        "{BASE}/ehr/{ehr}/versioned_ehr_status/version/{}",
        out_of_range(VO)
    );
    assert_400(&app, common::get(&uri), "VERSIONED_EHR_STATUS version").await;
}

#[tokio::test]
async fn a_composition_version_argument_out_of_range_is_400() {
    let (_db, app) = app().await;
    let ehr = common::create_ehr(&app).await;
    let uri = format!(
        "{BASE}/ehr/{ehr}/versioned_composition/{VO}/version/{}",
        out_of_range(VO)
    );
    assert_400(&app, common::get(&uri), "VERSIONED_COMPOSITION version").await;
}

#[tokio::test]
async fn a_directory_version_argument_out_of_range_is_400() {
    let (_db, app) = app().await;
    let ehr = common::create_ehr(&app).await;
    let uri = format!("{BASE}/ehr/{ehr}/directory/{}", out_of_range(VO));
    assert_400(&app, common::get(&uri), "DIRECTORY by version").await;
}

#[tokio::test]
async fn a_malformed_contribution_uid_is_400() {
    let (_db, app) = app().await;
    let ehr = common::create_ehr(&app).await;
    let uri = format!("{BASE}/ehr/{ehr}/contribution/not-a-uuid");
    assert_400(&app, common::get(&uri), "EHR CONTRIBUTION").await;
    let uri = format!("{BASE}/demographic/contribution/not-a-uuid");
    assert_400(&app, common::get(&uri), "demographic CONTRIBUTION").await;
}

#[tokio::test]
async fn a_party_version_argument_out_of_range_is_400() {
    let (_db, app) = app().await;
    let uri = format!("{BASE}/demographic/person/{}", out_of_range(VO));
    assert_400(&app, common::get(&uri), "PERSON by version").await;
    let uri = format!("{BASE}/demographic/person/not-a-uuid");
    assert_400(&app, common::get(&uri), "PERSON by uid").await;
}

#[tokio::test]
async fn a_relationship_version_argument_out_of_range_is_400() {
    let (_db, app) = app().await;
    let uri = format!("{BASE}/demographic/party_relationship/{}", out_of_range(VO));
    assert_400(&app, common::get(&uri), "PARTY_RELATIONSHIP by version").await;
    let uri = format!("{BASE}/demographic/party_relationship/not-a-uuid");
    assert_400(&app, common::get(&uri), "PARTY_RELATIONSHIP by uid").await;
}
