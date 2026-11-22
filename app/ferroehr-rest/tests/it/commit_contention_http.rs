// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Concurrent PUTs of one COMPOSITION, each naming the same version in
//! `If-Match` (#6256): one commits and every other answers `412`, as it did
//! when it waited for the per-object lock (ITS-REST overview
//! `Requests_and_responses.md` §"If-Match and accidental overwrites"). The
//! writers are held at their commit statement by a transaction that holds
//! the EHR row, so every one judges the same version. No openEHR spec governs
//! how a commit is run against the database: our own design.

use std::time::Duration;

use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::Value;

use crate::access_evaluator_http::{app, credential, ehr_with_composition};
use crate::common::{self, BASE};

/// `writers` PUTs of the EHR's composition naming its first version, held
/// until all have judged it; their statuses.
#[expect(
    clippy::disallowed_methods,
    reason = "a test holds the EHR row on a raw pool connection, outside any request"
)]
async fn puts(writers: usize) -> Vec<StatusCode> {
    let (db, pool, _svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let (status, headers, body) = common::send(
        &app,
        Request::get(format!("{BASE}/ehr/{ehr}/composition/{vo}"))
            .header(header::AUTHORIZATION, credential("root"))
            .header(header::ACCEPT, "application/json")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the read: {body}");
    let tip = headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("an ETag")
        .to_owned();
    let mut held = pool.begin().await.expect("begin the hold");
    sqlx::query("SELECT 1 FROM ehr WHERE id = $1::uuid FOR UPDATE")
        .bind(&ehr)
        .execute(&mut *held)
        .await
        .expect("hold the EHR row");
    let tasks: Vec<_> = (0..writers)
        .map(|n| {
            let mut body = common::ips_canonical_composition();
            body.as_object_mut().expect("an object").shift_remove("uid");
            body["context"]["start_time"]["value"] =
                Value::from(format!("2021-12-03T17:34:{n:02}.849379+01:00"));
            let request = Request::put(format!("{BASE}/ehr/{ehr}/composition/{vo}"))
                .header(header::AUTHORIZATION, credential("root"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::IF_MATCH, tip.clone())
                .header("Prefer", "return=minimal")
                .body(Body::from(body.to_string()))
                .unwrap();
            let app = app.clone();
            tokio::spawn(async move { common::send(&app, request).await.0 })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(500)).await;
    held.commit().await.expect("release the hold");
    let mut statuses = Vec::new();
    for task in tasks {
        statuses.push(task.await.expect("the PUT completes"));
    }
    drop(db);
    statuses
}

/// One PUT commits and every other answers `412`, none `503`.
async fn one_wins(writers: usize) {
    let statuses = puts(writers).await;
    let won = statuses.iter().filter(|s| s.is_success()).count();
    let lost = statuses
        .iter()
        .filter(|s| **s == StatusCode::PRECONDITION_FAILED)
        .count();
    assert_eq!(
        (won, lost),
        (1, writers - 1),
        "one PUT commits, every other answers 412: {statuses:?}"
    );
}

#[tokio::test]
async fn two_puts_naming_one_version_one_commits_the_other_is_412() {
    one_wins(2).await;
}

#[tokio::test]
async fn eight_puts_naming_one_version_one_commits_the_rest_are_412() {
    one_wins(8).await;
}
