// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every EHR a query touches is accessed (EHDS Art. 9(1)): an EHR the query
//! names by its scope gets an access-log entry whatever its rows hold, a
//! `COUNT` and a past-the-end page included, and a population aggregate
//! writes one entry per EHR whose data it read (#5639). Over the assembled
//! router, the audit drain and the local store on a real `PostgreSQL` 18.
//! Which EHRs a query touched is our own design.

use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::json;

use crate::access_log_http::{SENTINEL, SUBJECT, as_root, commit, create_ehr, entries};
use crate::common::{self, BASE};

/// Runs `text` as the administrator, scoped to `ehr` when one is given.
async fn query(app: &axum::Router, text: &str, ehr: Option<&str>) {
    let mut req = as_root(Request::post(format!("{BASE}/query/aql")))
        .header(header::CONTENT_TYPE, "application/json");
    if let Some(ehr) = ehr {
        req = req.header("openehr-ehr-id", ehr);
    }
    let (status, body) = common::send_body(
        app,
        req.body(Body::from(json!({ "q": text }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}: {body}");
}

/// A `COUNT` scoped to an EHR, and a page past the end of its rows, each
/// write an entry for that EHR under the query's request.
#[tokio::test]
async fn a_scoped_count_and_a_past_the_end_page_write_an_entry_for_the_ehr() {
    let (_db, pool, app, ehr) = common::audited_app(SUBJECT, "root").await;
    commit(&app, &ehr).await;
    query(
        &app,
        "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c",
        Some(&ehr),
    )
    .await;
    query(
        &app,
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c LIMIT 10 OFFSET 50",
        Some(&ehr),
    )
    .await;
    let named = entries(&pool, 2, |s| {
        s.action.as_deref() == Some("query") && s.ehr_id.as_deref() == Some(ehr.as_str())
    })
    .await;
    assert_eq!(named.len(), 2, "one entry per query: {named:?}");
    assert_ne!(
        named[0].request_id, named[1].request_id,
        "each under its own request: {named:?}"
    );
}

/// A population `COUNT` writes one entry for each EHR whose compositions it
/// counted, and none for an EHR it did not read.
#[tokio::test]
async fn a_population_aggregate_writes_one_entry_per_contributing_ehr() {
    let (_db, pool, app, first) = common::audited_app(SUBJECT, "root").await;
    let second = create_ehr(&app).await;
    let untouched = create_ehr(&app).await;
    commit(&app, &first).await;
    commit(&app, &second).await;
    query(
        &app,
        &format!(
            "SELECT COUNT(c/uid/value) FROM EHR e CONTAINS COMPOSITION c \
             WHERE c/name/value = '{SENTINEL}'"
        ),
        None,
    )
    .await;
    let counted = entries(&pool, 2, |s| {
        s.action.as_deref() == Some("query") && s.ehr_id.is_some()
    })
    .await;
    let mut ehrs: Vec<&str> = counted.iter().filter_map(|s| s.ehr_id.as_deref()).collect();
    ehrs.sort_unstable();
    let mut expected = vec![first.as_str(), second.as_str()];
    expected.sort_unstable();
    assert_eq!(ehrs, expected, "one entry per contributing EHR");
    assert!(
        !counted
            .iter()
            .any(|s| s.ehr_id.as_deref() == Some(untouched.as_str())),
        "{counted:?}"
    );
}

/// A query the evaluator refuses names the EHR the caller asked for: the
/// child's portal token scoping a query to another person's EHR is refused
/// `refused-not-subject`, and the refused entry names that EHR.
#[tokio::test]
async fn a_refused_query_names_the_ehr_it_asked_for() {
    let fx = crate::access_log_reads_http::fixture().await;
    let credential = crate::portal_representatives_http::token(
        crate::portal_representatives_http::CHILD_LOGIN,
        &fx.ehr,
        &json!({ "scope": "patient/composition-*.cruds patient/aql-*.s" }),
    );
    let (status, body) = common::send_body(
        &fx.app,
        common::req("POST", format!("{BASE}/query/aql"))
            .auth(&credential)
            .header("openehr-ehr-id", fx.elsewhere.as_str())
            .json(json!({ "q": "SELECT e/ehr_id/value FROM EHR e" }).to_string())
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    let refused = entries(&fx.pool, 1, |s| {
        s.result.as_deref() == Some("refused") && s.action.as_deref() == Some("query")
    })
    .await;
    assert_eq!(
        refused[0].ehr_id.as_deref(),
        Some(fx.elsewhere.as_str()),
        "{refused:?}"
    );
}
