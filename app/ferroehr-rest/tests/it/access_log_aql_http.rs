// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL over the access log (#5131), end to end over the assembled router, the
//! audit drain and the local store, on a real `PostgreSQL` 18.
//!
//! `ACCESS_LOG_ENTRY` is queried through the ITS-REST Query API under the
//! rules of the log's own resources (#5130): the administrator reads across
//! EHRs in the full view, the subject of care reads their own entries in the
//! subject view, a professional the admin class does not admit is refused, a
//! query that puts the log inside an EHR containment is refused, and every
//! such query writes its own access-log entry. No openEHR spec governs AQL
//! over the access log: our own design.

use serde_json::{Value, json};

use http::StatusCode;

use crate::access_log_reads_http::{PATIENT_ROLE, fixture, get, root, stored};
use crate::common::{self, BASE};
use crate::portal_representatives_http::{CHILD_LOGIN, token};

/// The child's portal token, granted the `aql-` search a query needs.
fn patient(ehr: &str) -> String {
    let claims = json!({
        "roles": [PATIENT_ROLE],
        "scope": "patient/composition-*.cruds patient/aql-*.s",
    });
    token(CHILD_LOGIN, ehr, &claims)
}

/// The ad-hoc query URI of `aql`, scoped to `ehr` when given.
fn query(aql: &str, ehr: Option<&str>) -> String {
    let scope = ehr.map(|ehr| format!("ehr_id={ehr}&")).unwrap_or_default();
    format!("{BASE}/query/aql?{scope}q={}", urlencoding::encode(aql))
}

/// The rows of a `RESULT_SET`.
fn rows(result: &Value) -> &[Value] {
    result["rows"].as_array().map_or(&[], Vec::as_slice)
}

/// Whether `entry` records an AQL query of the access log by an accessor of
/// `kind`.
fn aql_read_by(entry: &Value, kind: &str) -> bool {
    entry["accessor"]["kind"]["value"] == json!(kind)
        && entry["action"]["value"] == json!("read")
        && entry["query"]["language"] == json!("AQL")
}

/// The administrator reads every EHR's entries in the full view.
#[tokio::test]
async fn the_administrator_queries_across_ehrs() {
    let fx = fixture().await;
    let (status, body) = get(&fx.app, &format!("{BASE}/ehr/{}", fx.elsewhere), &root()).await;
    assert_eq!(status, StatusCode::OK, "root reads the other EHR: {body}");
    stored(&fx.pool, 1, |entry| {
        entry["ehr_id"]["value"] == json!(fx.elsewhere)
    })
    .await;

    let aql = "SELECT l/ehr_id/value, l/channel/request_id FROM ACCESS_LOG_ENTRY l \
               WHERE l/accessor/kind/value = 'person' AND EXISTS l/ehr_id";
    let (status, result) = get(&fx.app, &query(aql, None), &root()).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    let ehrs: Vec<&Value> = rows(&result).iter().map(|row| &row[0]).collect();
    assert!(ehrs.contains(&&json!(fx.ehr)), "{result}");
    assert!(ehrs.contains(&&json!(fx.elsewhere)), "{result}");
    assert!(
        rows(&result).iter().all(|row| row[1].is_string()),
        "the full view: {result}"
    );
}

/// The subject queries only their own entries, in the subject view.
#[tokio::test]
async fn the_subject_queries_only_their_own_redacted_entries() {
    let fx = fixture().await;
    let credential = patient(&fx.ehr);
    let aql = "SELECT l, l/ehr_id/value, l/accessor/credential/subject FROM ACCESS_LOG_ENTRY l";
    let (status, result) = get(&fx.app, &query(aql, Some(&fx.ehr)), &credential).await;
    assert_eq!(status, StatusCode::OK, "{result}");
    assert!(
        !rows(&result).is_empty(),
        "the professional's read is shown: {result}"
    );
    for row in rows(&result) {
        assert_eq!(row[1], json!(fx.ehr), "only their own EHR: {row}");
        assert_eq!(row[2], Value::Null, "the credential is withheld: {row}");
        assert!(row[0].get("link").is_none(), "the subject view: {row}");
        assert!(row[0].get("channel").is_none(), "{row}");
    }

    let (status, body) = get(&fx.app, &query(aql, Some(&fx.elsewhere)), &credential).await;
    assert!(
        status == StatusCode::NOT_FOUND || status == StatusCode::FORBIDDEN,
        "another EHR's log is not read: {status} {body}"
    );
}

/// A professional the admin class does not admit is refused, and a query that
/// puts the log inside an EHR containment is refused for everyone.
#[tokio::test]
async fn a_refused_caller_and_a_containment_are_refused() {
    let fx = fixture().await;
    let dr = common::basic_credential("dr");
    let (status, body) = get(
        &fx.app,
        &query("SELECT l FROM ACCESS_LOG_ENTRY l", None),
        &dr,
    )
    .await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

    let (status, body) = get(
        &fx.app,
        &query("SELECT l FROM EHR e CONTAINS ACCESS_LOG_ENTRY l", None),
        &root(),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(
        body.to_string()
            .contains("never appears inside an EHR containment"),
        "the refusal names the rule: {body}"
    );
}

/// Each query of the log writes an entry: the subject's names the log of
/// their EHR, and the administrator's names the query by its language.
#[tokio::test]
async fn each_query_of_the_log_is_recorded() {
    let fx = fixture().await;
    let aql = "SELECT l/uid/value FROM ACCESS_LOG_ENTRY l";
    let (status, _) = get(&fx.app, &query(aql, Some(&fx.ehr)), &patient(&fx.ehr)).await;
    assert_eq!(status, StatusCode::OK);
    let own = stored(&fx.pool, 1, |entry| aql_read_by(entry, "subject of care")).await;
    assert_eq!(own[0]["ehr_id"]["value"], json!(fx.ehr), "{}", own[0]);
    assert!(
        own[0]["items"].as_array().is_some_and(|items| items
            .iter()
            .any(|item| item["rm_type"] == json!("ACCESS_LOG_RECORD"))),
        "the entry names the log it read: {}",
        own[0]
    );

    let (status, _) = get(&fx.app, &query(aql, None), &root()).await;
    assert_eq!(status, StatusCode::OK);
    stored(&fx.pool, 1, |entry| aql_read_by(entry, "person")).await;
}
