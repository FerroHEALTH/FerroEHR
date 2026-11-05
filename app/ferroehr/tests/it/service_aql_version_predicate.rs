// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `VERSION` standard predicate on `commit_audit/time_committed`, executed
//! against a real PostgreSQL 18 (shared testkit harness, #5062).
//!
//! QUERY master03 §Standard predicate: a standard predicate has a left-hand
//! operand, an operator from `>, >=, =, <, <=, !=`, and a right-hand operand,
//! and §Class expressions lets it be part three of a class expression. So
//! `VERSION v[commit_audit/time_committed/value > $t]` selects the versions
//! whose commit time is after `$t`, each operator with its §Comparison
//! operators meaning, never "the version current at `$t`".

use serde_json::{Value, json};

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;

use crate::fixtures::{composition, create_ehr, uv, vo_of};

/// Every version of the one composition, with its commit time, oldest first.
const VERSIONS: &str = "SELECT v/uid/value, v/commit_audit/time_committed/value \
     FROM EHR e CONTAINS VERSION v[ALL_VERSIONS] CONTAINS COMPOSITION c \
     ORDER BY v/commit_audit/time_committed/value";

async fn run(svc: &FerroEhrService, aql: &str, ehr: EhrId, params: Value) -> Vec<Value> {
    let request = AqlQueryRequest {
        ehr_ids: vec![ehr],
        parameters: serde_json::from_value(params).expect("query parameters"),
        ..AqlQueryRequest::default()
    };
    svc.execute_aql(aql, None, &request)
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"))
        .result_set["rows"]
        .as_array()
        .expect("rows array")
        .clone()
}

/// The first cell of every row, sorted.
fn uids(rows: &[Value]) -> Vec<String> {
    let mut cells: Vec<String> = rows
        .iter()
        .map(|r| r[0].as_str().expect("a version uid").to_owned())
        .collect();
    cells.sort();
    cells
}

/// One composition committed three times; returns the EHR and its versions
/// as `(uid, time_committed)`, oldest first.
async fn three_versions(svc: &FerroEhrService) -> (EhrId, Vec<(String, String)>) {
    let ehr = create_ehr(svc).await;
    let mut current = svc
        .create_composition(ehr, uv(&composition("version predicate"), "249", None))
        .await
        .expect("create the composition")
        .version_uid();
    let vo: VoId = vo_of(&current).parse().expect("a versioned object uuid");
    for _ in 0..2 {
        current = svc
            .update_composition(
                ehr,
                vo,
                uv(&composition("version predicate"), "251", Some(&current)),
            )
            .await
            .expect("update the composition")
            .version_uid();
    }
    let rows = run(svc, VERSIONS, ehr, json!({})).await;
    let versions: Vec<(String, String)> = rows
        .iter()
        .map(|r| {
            (
                r[0].as_str().expect("a version uid").to_owned(),
                r[1].as_str().expect("a commit time").to_owned(),
            )
        })
        .collect();
    assert_eq!(versions.len(), 3, "three versions: {rows:?}");
    (ehr, versions)
}

/// Each admitted operator, compared against the middle version's commit time,
/// selects exactly the versions whose commit time satisfies it.
#[tokio::test]
async fn each_operator_selects_the_versions_whose_commit_time_satisfies_it() {
    let (_db, svc) = crate::fixtures::service().await;
    let (ehr, versions) = three_versions(&svc).await;
    let [(first, _), (second, at), (third, _)] = versions.as_slice() else {
        panic!("three versions: {versions:?}");
    };
    let sorted = |picked: &[&String]| {
        let mut all: Vec<String> = picked.iter().map(|s| (*s).clone()).collect();
        all.sort();
        all
    };
    for (op, expected) in [
        (">", sorted(&[third])),
        (">=", sorted(&[second, third])),
        ("=", sorted(&[second])),
        ("<", sorted(&[first])),
        ("<=", sorted(&[first, second])),
        ("!=", sorted(&[first, third])),
    ] {
        let aql = format!(
            "SELECT v/uid/value FROM EHR e \
             CONTAINS VERSION v[commit_audit/time_committed/value {op} $t] \
             CONTAINS COMPOSITION c"
        );
        let rows = run(&svc, &aql, ehr, json!({ "t": at })).await;
        assert_eq!(uids(&rows), expected, "time_committed {op} {at}");
    }
}

/// The whole `DV_DATE_TIME` compares like its `value` (QUERY master03 §Dates
/// and Times compares `composition/context/start_time` with a date-time
/// literal), here as a literal after every commit: no version is later.
#[tokio::test]
async fn a_bound_after_every_commit_selects_no_later_version() {
    let (_db, svc) = crate::fixtures::service().await;
    let (ehr, versions) = three_versions(&svc).await;
    let rows = run(
        &svc,
        "SELECT v/uid/value FROM EHR e \
         CONTAINS VERSION v[commit_audit/time_committed > '9999-12-31T23:59:59Z'] \
         CONTAINS COMPOSITION c",
        ehr,
        json!({}),
    )
    .await;
    assert!(rows.is_empty(), "no version after the bound: {rows:?}");
    let rows = run(
        &svc,
        "SELECT v/uid/value FROM EHR e \
         CONTAINS VERSION v[commit_audit/time_committed < '9999-12-31T23:59:59Z'] \
         CONTAINS COMPOSITION c",
        ehr,
        json!({}),
    )
    .await;
    let mut all: Vec<String> = versions.into_iter().map(|(uid, _)| uid).collect();
    all.sort();
    assert_eq!(uids(&rows), all, "every version before the bound");
}

/// A bound that is no ISO-8601 date/time is refused with `400`, never
/// compared as nothing (QUERY master03 §Built-in Types, Dates and Times).
#[tokio::test]
async fn a_bound_that_is_no_date_time_is_a_bad_request() {
    let (_db, svc) = crate::fixtures::service().await;
    let ehr = create_ehr(&svc).await;
    let request = AqlQueryRequest {
        ehr_ids: vec![ehr],
        parameters: serde_json::from_value(json!({ "t": "yesterday" })).expect("parameters"),
        ..AqlQueryRequest::default()
    };
    let err = svc
        .execute_aql(
            "SELECT v/uid/value FROM EHR e \
             CONTAINS VERSION v[commit_audit/time_committed/value > $t] \
             CONTAINS COMPOSITION c",
            None,
            &request,
        )
        .await
        .expect_err("a non-temporal bound is refused");
    assert!(
        matches!(err, ferroehr::error::ServiceError::BadRequest(_)),
        "{err:?}"
    );
}
