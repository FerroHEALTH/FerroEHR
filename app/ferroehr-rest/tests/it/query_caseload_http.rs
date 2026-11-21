// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caseload members at the wire (#6056): `caseload` and `caseload_roles`
//! beside `q` in the body of `POST /query/aql` and of a stored query's `POST`
//! execution, refused `400` naming the member when they do not read, and
//! narrowing the query when they do (draft access control, Queries and the
//! Caseload; FerroEHR's extension).

use http::StatusCode;
use serde_json::{Value, json};

use crate::access_evaluator_http::{app, ehr_with_composition, send};
use crate::common::BASE;

/// The status and body of `POST /query/aql` with `members` beside `q`, as
/// the nurse.
async fn query(app: &axum::Router, members: &Value) -> (StatusCode, String) {
    let mut body = json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" });
    if let (Some(body), Some(members)) = (body.as_object_mut(), members.as_object()) {
        body.extend(members.clone());
    }
    send(
        app,
        "POST",
        &format!("{BASE}/query/aql"),
        "nurse",
        None,
        Some(("application/json", body.to_string())),
    )
    .await
}

#[tokio::test]
async fn the_caseload_members_narrow_or_are_refused_naming_the_member() {
    let (_db, _pool, _svc, app) = Box::pin(app()).await;
    ehr_with_composition(&app).await;
    let rows = |body: &str| {
        serde_json::from_str::<Value>(body).expect("RESULT_SET")["rows"]
            .as_array()
            .map_or(0, Vec::len)
    };
    let (status, body) = query(&app, &json!({})).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(rows(&body), 1, "no caseload: {body}");
    let (status, body) = query(&app, &json!({ "caseload": "none" })).await;
    assert_eq!((status, rows(&body)), (StatusCode::OK, 1), "none: {body}");
    let (status, body) = query(&app, &json!({ "caseload": "all" })).await;
    assert_eq!(
        (status, rows(&body)),
        (StatusCode::OK, 0),
        "the nurse holds no care relationship: {body}"
    );
    for (members, member) in [
        (json!({ "caseload": "indirect" }), "caseload:"),
        (
            json!({ "caseload": "none", "caseload_roles": ["RESP"] }),
            "caseload_roles",
        ),
        (
            json!({ "caseload": "direct", "caseload_roles": [] }),
            "caseload_roles",
        ),
    ] {
        let (status, body) = query(&app, &members).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{members}: {body}");
        assert!(body.contains(member), "{members} names {member}: {body}");
    }
}
