// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data permit on the access record, end to end over the assembled
//! router, the audit sender and the local Audit Record Repository.
//!
//! A secondary-use reader acts under a data permit a health data access body
//! issued (Regulation (EU) 2025/327 Art. 68, `docs/law/eu/ehds/text.html`).
//! A population query and an export declared under one are recorded with the
//! declared purpose and the permit's identifier; a value outside the
//! identifier form is recorded as absent. The stored FHIR `AuditEvent` the
//! subject-scoped ITI-81 retrieval answers carries the permit at
//! `agent.policy` (FHIR R4 `AuditEvent.agent.policy`). No openEHR spec
//! governs access-log content: our own design/extension.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;

use crate::common;
use crate::common::BASE;
use crate::common::EXT;
use crate::common::FHIR;

/// The headers the default configuration reads the purpose and the permit from.
const PURPOSE: &str = "x-purpose-of-use";
const PERMIT: &str = "x-data-permit";
/// A synthetic permit identifier.
const PERMIT_ID: &str = "hdab-nl/2026/0042";
/// The synthetic patient subject every EHR of the test resolves to.
const SUBJECT: &str = "patient-permit-1";

/// An audited app with the local store on and one EHR; returns the EHR id.
async fn app() -> (testkit::TestDb, PgPool, Router, String) {
    common::audited_app(SUBJECT, "root").await
}

/// The `(purpose, permit)` of every stored record of `operation`, polled until
/// `expected` records have landed.
async fn recorded(
    pool: &PgPool,
    operation: &str,
    expected: usize,
) -> Vec<(Option<String>, Option<String>)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let rows: Vec<(Option<String>, Option<String>)> = sqlx::query_as(
            "SELECT purpose, permit FROM audit.audit_event \
             WHERE operation = $1 ORDER BY chain_seq",
        )
        .bind(operation)
        .fetch_all(pool)
        .await
        .expect("read the trail");
        if rows.len() >= expected {
            return rows;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {} of {expected} {operation} records landed",
            rows.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A population query (no `ehr_id`) declared under a permit, one without a
/// permit and one with a value that is not an identifier are recorded with
/// the permit, without one, and without one.
#[tokio::test]
async fn a_population_query_is_recorded_with_its_purpose_and_permit() {
    let (_db, pool, app, _ehr) = app().await;
    let query = |permit: Option<&str>| {
        let mut req = Request::post(format!("{BASE}/query/aql"))
            .header(header::AUTHORIZATION, common::basic_credential("root"))
            .header(header::CONTENT_TYPE, "application/json")
            .header(PURPOSE, "RESEARCH");
        if let Some(permit) = permit {
            req = req.header(PERMIT, permit);
        }
        req.body(Body::from(r#"{"q":"SELECT e/ehr_id/value FROM EHR e"}"#))
            .unwrap()
    };
    for permit in [Some(PERMIT_ID), None, Some("permit of J. Jansen")] {
        let (status, body) = common::send_body(&app, query(permit)).await;
        assert_eq!(status, StatusCode::OK, "population query: {body}");
    }
    assert_eq!(
        recorded(&pool, "query_execute_adhoc_query_body", 3).await,
        vec![
            (Some("RESEARCH".to_owned()), Some(PERMIT_ID.to_owned())),
            (Some("RESEARCH".to_owned()), None),
            (Some("RESEARCH".to_owned()), None),
        ]
    );
}

/// An export declared under a permit is recorded with the purpose and the
/// permit, and one declared under none without it; the subject-scoped ITI-81
/// retrieval answers the access record of the first with the permit at the
/// requestor's `agent.policy` and that of the second with no policy.
#[tokio::test]
async fn an_export_is_recorded_with_its_purpose_and_permit() {
    let (_db, pool, app, ehr) = app().await;
    for permit in [Some(PERMIT_ID), None] {
        let mut req = Request::get(format!("{EXT}/message/export/{ehr}"))
            .header(header::AUTHORIZATION, common::basic_credential("root"))
            .header(PURPOSE, "RESEARCH");
        if let Some(permit) = permit {
            req = req.header(PERMIT, permit);
        }
        let (status, body) = common::send_body(&app, req.body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "export: {body}");
    }
    assert_eq!(
        recorded(&pool, "message_export_ehrs", 2).await,
        vec![
            (Some("RESEARCH".to_owned()), Some(PERMIT_ID.to_owned())),
            (Some("RESEARCH".to_owned()), None),
        ]
    );

    let mut policies = exported_policies(&app).await;
    policies.sort_by_key(Value::to_string);
    assert_eq!(policies, vec![serde_json::json!([PERMIT_ID]), Value::Null]);
}

/// The requestor's `agent.policy` of every Export access record the
/// subject-scoped ITI-81 retrieval answers, polled until both exports' records
/// have landed.
async fn exported_policies(app: &Router) -> Vec<Value> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let (status, body) = common::send_body(
            app,
            Request::get(format!(
                "{FHIR}/AuditEvent?date=ge2026-01-01&patient={SUBJECT}"
            ))
            .header(header::AUTHORIZATION, common::basic_credential("root"))
            .body(Body::empty())
            .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "subject-scoped retrieval: {body}");
        let bundle: Value = serde_json::from_str(&body).expect("bundle json");
        let policies: Vec<Value> = bundle["entry"]
            .as_array()
            .expect("entries")
            .iter()
            .map(|entry| &entry["resource"])
            .filter(|resource| {
                resource["entity"]
                    .as_array()
                    .is_some_and(|e| e.iter().any(|entity| entity["name"] == "Export"))
            })
            .map(|resource| {
                let requestor = resource["agent"]
                    .as_array()
                    .expect("agents")
                    .iter()
                    .find(|agent| agent["requestor"] == true)
                    .expect("an access record names its requestor");
                requestor["policy"].clone()
            })
            .collect();
        if policies.len() >= 2 {
            return policies;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {} of 2 Export records answered: {bundle}",
            policies.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
