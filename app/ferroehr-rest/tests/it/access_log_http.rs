// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR access-log entry per access (#5126), end to end over the
//! assembled router, the audit sender and the local store.
//!
//! Every route kind that touches EHR data writes its entries through the one
//! recording path: an ITS-REST write and read, an AQL population query with
//! one entry per EHR it served (an EHR-only statement included), a refusal, a
//! read of the access log through the FHIR ITI-81 search, and the export
//! extension; the management routes are `crate::access_log_management_http`. No entry carries a data
//! value of the EHR or the query text. No openEHR spec governs the access log:
//! our own design on the draft `org.openehr.rm.access_log` 0.1.0.

use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::common;
use crate::common::{BASE, EXT, FHIR};

/// The synthetic patient subject every EHR of the test resolves to.
pub(crate) const SUBJECT: &str = "patient-access-log-1";
/// A value written into a composition, which no entry may carry.
const SENTINEL: &str = "access-log-sentinel-6b1f";

/// One stored entry: its EHR, action, result, request id and canonical form.
#[derive(Debug, Clone)]
pub(crate) struct Stored {
    pub(crate) ehr_id: Option<String>,
    pub(crate) action: Option<String>,
    pub(crate) result: Option<String>,
    pub(crate) request_id: Option<String>,
    pub(crate) content: String,
}

/// The columns of one stored entry, in [`Stored`]'s order.
type Columns = (
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
    String,
);

/// The entries stored so far that `keep` selects, polled until there are at
/// least `expected`.
pub(crate) async fn entries(
    pool: &PgPool,
    expected: usize,
    keep: impl Fn(&Stored) -> bool,
) -> Vec<Stored> {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let rows: Vec<Columns> = sqlx::query_as(
            "SELECT ehr_id, action, result, request_id, content \
             FROM audit.access_log_record ORDER BY chain_seq",
        )
        .fetch_all(pool)
        .await
        .expect("read the access log");
        let all = rows.len();
        let debug_rows: Vec<String> = rows.iter().map(|row| row.4.clone()).collect();
        let kept: Vec<Stored> = rows
            .into_iter()
            .map(|(ehr_id, action, result, request_id, content)| Stored {
                ehr_id,
                action,
                result,
                request_id,
                content,
            })
            .filter(|stored| keep(stored))
            .collect();
        if kept.len() >= expected {
            return kept;
        }
        assert!(
            Instant::now() < deadline,
            "only {} of {expected} entries landed, of {all} stored: {:#?}",
            kept.len(),
            debug_rows
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A minimal templateless COMPOSITION whose name carries [`SENTINEL`].
fn composition() -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": SENTINEL },
        "language": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "en"
        },
        "territory": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
            "code_string": "NL"
        },
        "category": {
            "_type": "DV_CODED_TEXT",
            "value": "event",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "433"
            }
        },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "access log tester" }
    })
}

/// `req` sent as the ADMIN `root`.
pub(crate) fn as_root(req: http::request::Builder) -> http::request::Builder {
    req.header(header::AUTHORIZATION, common::basic_credential("root"))
}

/// Create one more EHR; returns its id.
pub(crate) async fn create_ehr(app: &Router) -> String {
    let (status, body) = common::send_body(
        app,
        as_root(Request::post(format!("{BASE}/ehr")))
            .header("Prefer", "return=representation")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: Value = serde_json::from_str(&body).expect("EHR json");
    value["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned()
}

/// Commit [`composition`] into `ehr`; returns its version uid.
pub(crate) async fn commit(app: &Router, ehr: &str) -> String {
    let (status, body) = common::send_body(
        app,
        as_root(Request::post(format!("{BASE}/ehr/{ehr}/composition")))
            .header(header::CONTENT_TYPE, "application/json")
            .header("Prefer", "return=representation")
            .body(Body::from(composition().to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {body}");
    let value: Value = serde_json::from_str(&body).expect("composition json");
    value["uid"]["value"].as_str().expect("uid").to_owned()
}

/// An ITS-REST write and read each write one granted entry for the EHR, the
/// read naming the version it delivered, and neither entry carries the
/// composition's content.
#[tokio::test]
async fn an_its_rest_write_and_read_each_write_an_entry_without_content() {
    let (_db, pool, app, ehr) = common::audited_app(SUBJECT, "root").await;
    let uid = commit(&app, &ehr).await;
    let (status, body) = common::send_body(
        &app,
        as_root(Request::get(format!("{BASE}/ehr/{ehr}/composition/{uid}")))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "read: {body}");
    assert!(body.contains(SENTINEL), "the read served the composition");

    let mine = |s: &Stored| s.ehr_id.as_deref() == Some(ehr.as_str());
    let stored = entries(&pool, 3, |s| {
        mine(s) && matches!(s.action.as_deref(), Some("create" | "read"))
    })
    .await;
    let created = stored
        .iter()
        .filter(|s| s.action.as_deref() == Some("create"))
        .count();
    assert!(
        created >= 2,
        "the EHR and the composition create: {stored:?}"
    );
    let read: Vec<&Stored> = stored
        .iter()
        .filter(|s| s.action.as_deref() == Some("read"))
        .collect();
    assert_eq!(read.len(), 1, "{stored:?}");
    assert_eq!(read[0].result.as_deref(), Some("granted"));
    assert!(read[0].content.contains(&uid), "the read names its version");
    for entry in &stored {
        assert!(
            !entry.content.contains(SENTINEL),
            "an entry carries a data value of the EHR: {}",
            entry.content
        );
    }
}

/// A population query writes one entry for each EHR it served, every one
/// under the request id of the query, and none of them carries the query
/// text.
#[tokio::test]
async fn a_population_query_writes_one_entry_per_ehr_it_served() {
    let (_db, pool, app, first) = common::audited_app(SUBJECT, "root").await;
    let others = [create_ehr(&app).await, create_ehr(&app).await];
    for ehr in [&first, &others[0], &others[1]] {
        commit(&app, ehr).await;
    }
    let text = format!(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE c/name/value = '{SENTINEL}'"
    );
    let (status, body) = common::send_body(
        &app,
        as_root(Request::post(format!("{BASE}/query/aql")))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "q": text }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "population query: {body}");

    let served = entries(&pool, 3, |s| {
        s.action.as_deref() == Some("query") && s.ehr_id.is_some()
    })
    .await;
    let mut ehrs: Vec<&str> = served.iter().filter_map(|s| s.ehr_id.as_deref()).collect();
    ehrs.sort_unstable();
    let mut expected = vec![first.as_str(), others[0].as_str(), others[1].as_str()];
    expected.sort_unstable();
    assert_eq!(ehrs, expected, "one entry per contributing EHR");
    let request = served[0].request_id.clone();
    assert!(request.is_some(), "the entries name their request");
    assert!(
        served.iter().all(|s| s.request_id == request),
        "the entries share the request id: {served:?}"
    );
    for entry in &served {
        assert!(!entry.content.contains(SENTINEL), "{}", entry.content);
        assert!(entry.content.contains("text_digest"), "{}", entry.content);
    }
}

/// A statement that projects only the EHR, with no `CONTAINS COMPOSITION`,
/// writes one entry for each EHR it served under the request id of the query
/// (#5564), so the patient's view of who queried the record finds it.
#[tokio::test]
async fn an_ehr_only_query_writes_one_entry_per_ehr_it_served() {
    let (_db, pool, app, first) = common::audited_app(SUBJECT, "root").await;
    let others = [create_ehr(&app).await, create_ehr(&app).await];
    let text = "SELECT e/ehr_id/value FROM EHR e";
    let (status, body) = common::send_body(
        &app,
        as_root(Request::post(format!("{BASE}/query/aql")))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "q": text }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "EHR-only query: {body}");
    let answered: Value = serde_json::from_str(&body).expect("result set json");
    let rows = answered["rows"].as_array().expect("rows").len();

    let served = entries(&pool, rows, |s| {
        s.action.as_deref() == Some("query") && s.ehr_id.is_some()
    })
    .await;
    assert_eq!(served.len(), rows, "one entry per row's EHR: {served:?}");
    for ehr in [&first, &others[0], &others[1]] {
        assert!(
            served
                .iter()
                .any(|s| s.ehr_id.as_deref() == Some(ehr.as_str())),
            "the EHR {ehr} the query served has its entry: {served:?}"
        );
    }
    let request = served[0].request_id.clone();
    assert!(request.is_some(), "the entries name their request");
    assert!(
        served.iter().all(|s| s.request_id == request),
        "the entries share the request id: {served:?}"
    );
}

/// A refused access to an EHR writes one refused entry for that EHR.
#[tokio::test]
async fn a_refused_access_writes_a_refused_entry() {
    let (_db, pool, app, ehr) = common::audited_app(SUBJECT, "root").await;
    let (status, _) = common::send_body(
        &app,
        Request::get(format!("{BASE}/ehr/{ehr}"))
            .header(header::AUTHORIZATION, "Basic cm9vdDp3cm9uZw==")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::UNAUTHORIZED);
    let refused = entries(&pool, 1, |s| s.result.as_deref() == Some("refused")).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    let refused_again = entries(&pool, 1, |s| s.result.as_deref() == Some("refused")).await;
    assert_eq!(
        refused_again.len(),
        1,
        "one entry per refused access: {refused_again:?}"
    );
    assert_eq!(refused[0].ehr_id.as_deref(), Some(ehr.as_str()));
}

/// A read of the access log through the FHIR ITI-81 search writes an entry
/// that names the read of the log by its search.
#[tokio::test]
async fn a_read_of_the_log_writes_an_entry_naming_the_log() {
    let (_db, pool, app, _ehr) = common::audited_app(SUBJECT, "root").await;
    let (status, body) = common::send_body(
        &app,
        as_root(Request::get(format!(
            "{FHIR}/AuditEvent?date=ge2026-01-01&patient={SUBJECT}"
        )))
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "ITI-81: {body}");
    let read = entries(&pool, 1, |s| s.content.contains("audit_event_search")).await;
    assert_eq!(read.len(), 1, "{read:?}");
    assert_eq!(read[0].action.as_deref(), Some("read"));
    assert!(
        read[0].content.contains("FHIR search"),
        "{}",
        read[0].content
    );
    assert!(
        !read[0].content.contains(SUBJECT),
        "the search is named by its digest: {}",
        read[0].content
    );
}

/// The export extension writes an `export` entry for the EHR it exported,
/// which the service records by itself.
#[tokio::test]
async fn an_export_writes_an_entry_for_the_ehr() {
    let (_db, pool, app, ehr) = common::audited_app(SUBJECT, "root").await;
    let (status, body) = common::send_body(
        &app,
        as_root(Request::get(format!("{EXT}/message/export/{ehr}")))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "export: {body}");
    let exported = entries(&pool, 1, |s| {
        s.ehr_id.as_deref() == Some(ehr.as_str()) && s.action.as_deref() == Some("export")
    })
    .await;
    assert_eq!(exported[0].result.as_deref(), Some("granted"));
}
