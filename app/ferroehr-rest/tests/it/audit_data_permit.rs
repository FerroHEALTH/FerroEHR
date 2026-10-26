// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The data permit on the access record, end to end over the assembled
//! router, the audit sender and the local Audit Record Repository.
//!
//! A secondary-use reader acts under a data permit a health data access body
//! issued (Regulation (EU) 2025/327 Art. 68, `docs/law/eu/ehds/text.html`).
//! A population query and an export declared under one are recorded with the
//! declared purpose and the permit's identifier; a value outside the
//! identifier form is recorded as absent. No openEHR spec governs access-log
//! content: our own design/extension.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;

use crate::common;
use crate::common::BASE;

/// The headers the default configuration reads the purpose and the permit from.
const PURPOSE: &str = "x-purpose-of-use";
const PERMIT: &str = "x-data-permit";
/// A synthetic permit identifier.
const PERMIT_ID: &str = "hdab-nl/2026/0042";

fn basic(name: &str) -> String {
    use base64::Engine;
    format!(
        "Basic {}",
        base64::engine::general_purpose::STANDARD.encode(format!("{name}:pw").as_bytes())
    )
}

/// An audited app with the local store on and one EHR; returns the EHR id.
async fn app() -> (testkit::TestDb, PgPool, Router, String) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender)
            .with_audit_store(AuditStore::new(pool.clone())),
    );
    let mut config = common::api_config(true);
    common::with_basic_users(&mut config, &[("root", &["ADMIN"])]);
    let app = ferroehr_rest::build_full(config, svc, None, Observability::default())
        .await
        .expect("build app");
    let (status, body) = common::send_body(
        &app,
        Request::post(format!("{BASE}/ehr"))
            .header(header::AUTHORIZATION, basic("root"))
            .header("Prefer", "return=representation")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "EHR create: {body}");
    let value: Value = serde_json::from_str(&body).expect("EHR json");
    let ehr = value["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned();
    (db, pool, app, ehr)
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
            .header(header::AUTHORIZATION, basic("root"))
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
/// permit.
#[tokio::test]
async fn an_export_is_recorded_with_its_purpose_and_permit() {
    let (_db, pool, app, ehr) = app().await;
    let (status, body) = common::send_body(
        &app,
        Request::get(format!("{BASE}/message/export/{ehr}"))
            .header(header::AUTHORIZATION, basic("root"))
            .header(PURPOSE, "RESEARCH")
            .header(PERMIT, PERMIT_ID)
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "export: {body}");
    assert_eq!(
        recorded(&pool, "message_export_ehrs", 1).await,
        vec![(Some("RESEARCH".to_owned()), Some(PERMIT_ID.to_owned()))]
    );
}
