// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusal records of the login binding at the assembled router (#5011),
//! over a real `FerroEhrService` on a real `PostgreSQL` 18 with the local
//! party source and the audit store.
//!
//! A caller's first ten refusals in a window are recorded one by one, and the
//! rest become one summary record carrying their count and the EHR ids they
//! named when the window closes. A token that fails verification is charged
//! to its source network, never to a verified caller, so a flood of invalid
//! tokens cannot hide a verified caller's refusal. No openEHR spec governs
//! access logging: our own design.

use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use http::StatusCode;
use serde_json::{Value, json};

use crate::common::{self, BASE};
use crate::party_binding_http::{audited_service, config, refusal_rows, router_with, token};

/// The refusals of one caller recorded one by one in a window.
const PER_CALLER: usize = 10;

/// The router over `svc`, binding through the local source.
async fn router(svc: Arc<FerroEhrService>) -> Router {
    let source = Arc::new(svc.party_source("demographic"));
    router_with(svc, config(None), source).await
}

/// The status of a read of the `EHR_STATUS` of `ehr` as `credential`.
async fn read(app: &Router, ehr: &str, credential: &str) -> StatusCode {
    common::send_body(
        app,
        common::req("GET", format!("{BASE}/ehr/{ehr}/ehr_status"))
            .auth(credential)
            .accept("application/json")
            .build(),
    )
    .await
    .0
}

/// The summary records of `binding` once one is stored, waiting past the end
/// of the 60-second window the ledger keeps.
async fn summaries(pool: &sqlx::PgPool, binding: &str) -> Vec<Value> {
    for _ in 0..90 {
        let rows: Vec<Value> = sqlx::query_scalar(
            "SELECT summary FROM audit.access_trail \
             WHERE binding = $1 AND summary IS NOT NULL ORDER BY recorded_at",
        )
        .bind(binding)
        .fetch_all(pool)
        .await
        .expect("query");
        if !rows.is_empty() {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
    }
    panic!("no {binding} summary record was stored");
}

/// Thirteen refused reads of one unbound caller, each naming another EHR:
/// ten are recorded one by one, and the last three become one summary record
/// with their count and the three EHR ids.
#[tokio::test]
async fn a_callers_refusals_past_its_limit_become_one_summary_naming_the_ehr_ids() {
    let (db, _audit, svc) = audited_service().await;
    let app = router(svc).await;
    let credential = token("3c4d5e6f-7a8b-4c9d-8e0f-1a2b3c4d5e6f", &json!({}));
    let ehrs: Vec<String> = (0..PER_CALLER + 3)
        .map(|_| uuid::Uuid::now_v7().to_string())
        .collect();
    for ehr in &ehrs {
        assert_eq!(read(&app, ehr, &credential).await, StatusCode::FORBIDDEN);
    }
    let pool = db.pool();
    let one_by_one = refusal_rows(&pool, "refused-unbound", PER_CALLER).await;
    let summary = summaries(&pool, "refused-unbound").await;
    assert_eq!(summary.len(), 1, "one summary record: {summary:?}");
    let mut named: Vec<String> = summary[0]["ehr_ids"]
        .as_array()
        .expect("the EHR ids")
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    named.sort();
    let mut expected = ehrs[PER_CALLER..].to_vec();
    expected.sort();
    assert_eq!(summary[0]["count"], 3, "{summary:?}");
    assert_eq!(named, expected);
    let recorded = refusal_rows(&pool, "refused-unbound", PER_CALLER).await;
    assert_eq!(
        recorded.len(),
        one_by_one.len(),
        "no refusal past the limit is recorded on its own"
    );
    assert_eq!(recorded.len(), PER_CALLER);
}

/// A flood of tokens that fail verification exhausts its source network's
/// share of the window, and the verified unbound caller's refusal after it is
/// still recorded on its own.
#[tokio::test]
async fn a_flood_of_invalid_tokens_never_stops_a_verified_callers_record() {
    let (db, _audit, svc) = audited_service().await;
    let app = router(svc).await;
    let ehr = uuid::Uuid::now_v7().to_string();
    for _ in 0..PER_CALLER * 2 {
        assert_eq!(
            read(&app, &ehr, "Bearer not-a-token").await,
            StatusCode::UNAUTHORIZED
        );
    }
    let credential = token("4d5e6f7a-8b9c-4d0e-9f1a-2b3c4d5e6f7a", &json!({}));
    assert_eq!(read(&app, &ehr, &credential).await, StatusCode::FORBIDDEN);
    let rows = refusal_rows(&db.pool(), "refused-unbound", 1).await;
    assert_eq!(rows.len(), 1, "the verified caller's refusal is recorded");
}
