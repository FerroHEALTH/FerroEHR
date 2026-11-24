// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A write that reuses an identifier already in use (#6394): a CONTRIBUTION
//! whose client-supplied uid is taken is refused `409` (ITS-REST
//! `contribution_create`: the `uid` "will be accepted in case is not in-use,
//! otherwise error will be returned"; `409.yaml`), for an EHR and a
//! demographic CONTRIBUTION alike, and leaves nothing of its own.

use axum::Router;
use http::{StatusCode, header};
use serde_json::{Value, json};

use crate::common::{self, BASE, create_ehr};

/// `body` sent to `path` with `POST`: its status, content type and problem details,
/// with `instance` (each refusal's own urn) checked and dropped and every
/// `ids` text replaced by its name.
async fn answer(
    app: &Router,
    path: &str,
    body: &Value,
    ids: &[(&str, &str)],
) -> (StatusCode, String, Value) {
    let (status, headers, text) = common::send(
        app,
        common::req("POST", path.to_owned())
            .json(body.to_string())
            .build(),
    )
    .await;
    let content_type = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned();
    let text = ids
        .iter()
        .fold(text, |text, (id, name)| text.replace(id, name));
    let mut problem: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
    if let Some(instance) = problem
        .as_object_mut()
        .and_then(|details| details.shift_remove("instance"))
    {
        assert!(
            instance
                .as_str()
                .is_some_and(|i| i.starts_with("urn:uuid:")),
            "the refusal's own urn: {instance}"
        );
    }
    (status, content_type, problem)
}

/// The refusal of a CONTRIBUTION whose uid `{uid}` is in use.
fn in_use() -> (StatusCode, String, Value) {
    (
        StatusCode::CONFLICT,
        "application/problem+json".to_owned(),
        json!({
            "type": "https://ferroehr.eu/docs/latest/using-the-api/problems.html#conflict",
            "title": "The request conflicts with the resource's current state",
            "status": 409,
            "detail": "CONTRIBUTION uid Some({uid}) is already in use"
        }),
    )
}

/// A CONTRIBUTION of `uid` holding one version of `data`, a creation unless it
/// names its `preceding` version.
fn contribution(uid: &str, data: &Value, preceding: Option<&str>) -> Value {
    let (code, value) = match preceding {
        Some(_) => ("251", "modification"),
        None => ("249", "creation"),
    };
    let committer = json!({ "_type": "PARTY_IDENTIFIED", "name": "committer" });
    let change = json!({
        "_type": "DV_CODED_TEXT",
        "value": value,
        "defining_code": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
            "code_string": code }
    });
    let mut version = json!({
        "data": data,
        "lifecycle_state": {
            "_type": "DV_CODED_TEXT",
            "value": "complete",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "532" }
        },
        "commit_audit": { "change_type": change, "committer": committer }
    });
    if let Some(preceding) = preceding {
        version["preceding_version_uid"] =
            json!({ "_type": "OBJECT_VERSION_ID", "value": preceding });
    }
    json!({
        "uid": { "_type": "HIER_OBJECT_ID", "value": uid },
        "versions": [version],
        "audit": { "change_type": change, "committer": committer }
    })
}

/// The `EHR_STATUS` of `ehr` without its uid, and its version uid.
async fn status_of(app: &Router, ehr: &str) -> (Value, String) {
    let (code, mut status) = common::send_json(
        app,
        common::req("GET", format!("{BASE}/ehr/{ehr}/ehr_status")).build(),
    )
    .await;
    assert_eq!(code, StatusCode::OK, "the status: {status}");
    let uid = status["uid"]["value"]
        .as_str()
        .expect("a version uid")
        .to_owned();
    status
        .as_object_mut()
        .expect("an object")
        .shift_remove("uid");
    (status, uid)
}

/// The rows of `table` in `pool`.
async fn rows(pool: &sqlx::PgPool, table: &str) -> i64 {
    sqlx::query_scalar(sqlx::AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .expect("a count")
}

/// An EHR CONTRIBUTION reusing the uid of one committed before is `409`,
/// though its version is otherwise valid.
#[tokio::test]
async fn an_ehr_contribution_reusing_a_uid_is_409() {
    let (db, app) = common::app(common::api_config(false)).await;
    let pool = db.pool();
    let ehr = create_ehr(&app).await;
    let uid = uuid::Uuid::now_v7().to_string();
    let path = format!("{BASE}/ehr/{ehr}/contribution");
    let (status, v1) = status_of(&app, &ehr).await;
    let (first, ..) = answer(&app, &path, &contribution(&uid, &status, Some(&v1)), &[]).await;
    assert_eq!(first, StatusCode::CREATED);
    let (status, v2) = status_of(&app, &ehr).await;
    let before = [
        rows(&pool, "commit_audit").await,
        rows(&pool, "version").await,
    ];
    let second = answer(
        &app,
        &path,
        &contribution(&uid, &status, Some(&v2)),
        &[(&uid, "{uid}")],
    )
    .await;
    let after = [
        rows(&pool, "commit_audit").await,
        rows(&pool, "version").await,
    ];
    assert_eq!(second, in_use());
    assert_eq!(
        before, after,
        "the refused CONTRIBUTION leaves no audit and no version"
    );
}

/// A demographic CONTRIBUTION sent twice is `409` the second time.
#[tokio::test]
async fn a_demographic_contribution_reusing_a_uid_is_409() {
    let (db, app) = common::app(common::api_config(false)).await;
    let pool = db.pool();
    let uid = uuid::Uuid::now_v7().to_string();
    let path = format!("{BASE}/demographic/contribution");
    let body = contribution(&uid, &crate::demographic_http::person_body(), None);
    let (first, ..) = answer(&app, &path, &body, &[]).await;
    assert_eq!(first, StatusCode::CREATED);
    let before = rows(&pool, "party.commit_audit").await;
    let second = answer(&app, &path, &body, &[(&uid, "{uid}")]).await;
    let after = rows(&pool, "party.commit_audit").await;
    assert_eq!(second, in_use());
    assert_eq!(before, after, "the refused CONTRIBUTION leaves no audit");
}
