// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A bound of the wrong JSON type against a temporal AQL path, executed
//! against a real PostgreSQL 18 (shared testkit harness, #5064).
//!
//! QUERY master03 §Built-in Types (Dates and Times): a date/time operand is an
//! ISO 8601 string, so a number, boolean or `null` bound against a temporal
//! path is a defective query, refused with the caller's `400` and never
//! compared as `NULL` into an empty result.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use ferroehr::error::ServiceError;
use ferroehr::error::refusal::Malformed;
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;

use crate::fixtures::create_ehr;

/// A `WHERE` comparison of a `DV_DATE_TIME` value against `$t`.
const IN_WHERE: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
     WHERE c/context/start_time/value >= $t";

/// A `VERSION` standard predicate on the commit time against `$t`.
const IN_VERSION: &str = "SELECT v/uid/value FROM EHR e \
     CONTAINS VERSION v[commit_audit/time_committed/value > $t] \
     CONTAINS COMPOSITION c";

/// Each bound that is no ISO 8601 string.
fn non_temporal_bounds() -> [(&'static str, Value); 5] {
    [
        ("an integer", json!(5)),
        ("a real", json!(1.5)),
        ("a boolean", json!(true)),
        ("null", Value::Null),
        ("a non-ISO string", json!("yesterday")),
    ]
}

/// Runs `aql` with `params` and asserts the typed `400` precondition refusal.
async fn assert_refused(
    svc: &FerroEhrService,
    aql: &str,
    kind: &str,
    params: BTreeMap<String, Value>,
) {
    let ehr = create_ehr(svc).await;
    let request = AqlQueryRequest {
        ehr_ids: vec![ehr],
        parameters: params,
        ..AqlQueryRequest::default()
    };
    let err = svc
        .execute_aql(aql, None, &request)
        .await
        .expect_err("a non-temporal bound is refused");
    assert!(
        matches!(&err, ServiceError::BadRequest(r) if r.kind == Malformed::Precondition),
        "{aql:?} with {kind}: the caller's defect classifies 400, got {err:?}"
    );
}

/// A parameter of each non-temporal type in `WHERE` is the caller's `400`.
#[tokio::test]
async fn a_non_temporal_parameter_in_where_is_a_bad_request() {
    let (_db, svc) = crate::fixtures::service().await;
    for (kind, bound) in non_temporal_bounds() {
        let params = BTreeMap::from([("t".to_owned(), bound)]);
        assert_refused(&svc, IN_WHERE, kind, params).await;
    }
}

/// A parameter of each non-temporal type in a `VERSION` predicate is the
/// caller's `400`.
#[tokio::test]
async fn a_non_temporal_parameter_in_a_version_predicate_is_a_bad_request() {
    let (_db, svc) = crate::fixtures::service().await;
    for (kind, bound) in non_temporal_bounds() {
        let params = BTreeMap::from([("t".to_owned(), bound)]);
        assert_refused(&svc, IN_VERSION, kind, params).await;
    }
}

/// A number, boolean or `NULL` literal against a temporal path in `WHERE` is
/// the caller's `400`: QUERY master03 §Literals and §Comparison operators give
/// `NULL` no comparison meaning, presence being §EXISTS.
#[tokio::test]
async fn a_non_temporal_literal_against_a_temporal_path_is_a_bad_request() {
    let (_db, svc) = crate::fixtures::service().await;
    for literal in ["5", "1.5", "true", "NULL"] {
        let aql = format!(
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
             WHERE c/context/start_time/value >= {literal}"
        );
        assert_refused(&svc, &aql, literal, BTreeMap::new()).await;
    }
}
