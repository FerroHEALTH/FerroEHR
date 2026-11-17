// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A read by version id whose `If-None-Match` names the version answers
//! `304 Not Modified` with its versioning headers and no body (RFC 9110
//! §13.1.2, §15.4.5), and is still judged and recorded as the access it is
//! (#5778). The version never changes once committed (RM common master06
//! §Version Identification), and the server's `ETag` is weak (overview
//! §"`ETag` and Last-Modified"). Recording a `304` as the same access as a
//! `200` is our own design.

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};

use crate::access_evaluator_http::{app, credential, ehr_with_composition};
use crate::common;
use crate::common::BASE;

/// A GET of `uri` as the administrator, with `If-None-Match: tag` when given.
async fn get(
    app: &Router,
    uri: &str,
    tag: Option<&str>,
) -> (StatusCode, header::HeaderMap, String) {
    let mut req = Request::get(uri)
        .header(header::AUTHORIZATION, credential("root"))
        .header(header::ACCEPT, "application/json");
    if let Some(tag) = tag {
        req = req.header(header::IF_NONE_MATCH, tag);
    }
    common::send(app, req.body(Body::empty()).unwrap()).await
}

/// The `composition_get` records of the administrator, waiting for `count`.
async fn reads(pool: &sqlx::PgPool, count: usize) -> Vec<String> {
    for _ in 0..50 {
        let rows: Vec<String> = sqlx::query_scalar(
            "SELECT outcome::text FROM audit.access_trail \
             WHERE operation = 'composition_get' AND principal = 'root' ORDER BY recorded_at",
        )
        .fetch_all(pool)
        .await
        .expect("the access trail reads");
        if rows.len() >= count {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    panic!("fewer than {count} composition reads were recorded");
}

#[tokio::test]
async fn a_read_naming_the_version_it_holds_answers_not_modified_and_is_recorded() {
    let (_db, pool, _svc, app) = Box::pin(app()).await;
    let (ehr, vo) = ehr_with_composition(&app).await;
    let (status, headers, _) = get(&app, &format!("{BASE}/ehr/{ehr}/composition/{vo}"), None).await;
    assert_eq!(status, StatusCode::OK);
    let tag = headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .to_owned();
    let version = tag.trim_start_matches("W/").trim_matches('"').to_owned();
    let by_version = format!("{BASE}/ehr/{ehr}/composition/{version}");

    let (status, headers, body) = get(&app, &by_version, Some(&tag)).await;
    assert_eq!(status, StatusCode::NOT_MODIFIED, "{body}");
    assert!(body.is_empty(), "a 304 carries no body");
    assert_eq!(
        headers.get(header::ETAG).and_then(|v| v.to_str().ok()),
        Some(tag.as_str())
    );

    let strong = format!("\"{version}\"");
    let (status, _, _) = get(&app, &by_version, Some(&format!("W/\"other\", {strong}"))).await;
    assert_eq!(
        status,
        StatusCode::NOT_MODIFIED,
        "the weak comparison matches the strong form in a list"
    );

    let (status, _, body) = get(&app, &by_version, Some("W/\"other\"")).await;
    assert_eq!(status, StatusCode::OK, "another tag reads the version");
    assert!(body.contains(&version), "{body}");

    let recorded = reads(&pool, 4).await;
    assert_eq!(
        recorded.len(),
        4,
        "every read is recorded, a 304 among them"
    );
}
