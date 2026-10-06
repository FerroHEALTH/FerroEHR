// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Message component's EHR-Extract audit emission
//! (`ferroehr::service::message`) against a real `PostgreSQL` 18 (shared testkit
//! harness) with the local Audit Record Repository as the observation point.
//!
//! EHR-Extract communication carries patient-identifiable clinical data across
//! systems and is audited for **non-repudiation**: the security chapter requires
//! that "logging of communication of Extracts … can be used to guarantee
//! non-repudiation of information passed between systems" (BASE
//! `architecture_overview/master07-security.adoc` §Non-repudiation). These tests
//! pin that a completed export and a completed import each record exactly one
//! `extract`-class event, in the direction the operation ran, naming the EHR —
//! and that the emission is gated on the audit configuration.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::time::Duration;

use sqlx::{PgPool, Row};

use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::sender::{AuditHandle, start};

use crate::admin_fixture::{repository, seed_full_ehr};

/// An audit configuration that only writes to the local repository — no syslog
/// transport, so the observation point is the `audit` schema alone.
fn store_only_config() -> AuditConfig {
    AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    }
}

/// The recorded `extract`-class events as `(action, resource_id)` pairs,
/// polled until the sender's background drain has written them.
async fn extract_events(pool: &PgPool) -> Vec<(String, Option<String>)> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut rows = Vec::new();
    while std::time::Instant::now() < deadline {
        rows = sqlx::query(
            "SELECT action, resource_id FROM audit.audit_event \
             WHERE resource_class = 'extract' ORDER BY recorded_at, action",
        )
        .fetch_all(pool)
        .await
        .expect("audit query")
        .iter()
        .map(|row| {
            (
                row.get::<String, _>("action"),
                row.get::<Option<String>, _>("resource_id"),
            )
        })
        .collect();
        if !rows.is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    rows
}

/// A completed export records one `Read`-direction extract event naming the
/// exported EHR; a completed import into a second repository records the
/// `Create`-direction twin. Nothing else in the operation is audited as an
/// extract, so the count is exact.
#[tokio::test]
async fn export_and_import_each_record_one_directed_extract_event() {
    let (_src_db, src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;

    let (out_sender, _out_handle): (_, AuditHandle) =
        start(store_only_config(), None, Some(src_pool.clone()))
            .await
            .expect("source audit sender");
    let (in_sender, _in_handle): (_, AuditHandle) =
        start(store_only_config(), None, Some(dst_pool.clone()))
            .await
            .expect("target audit sender");
    let source: FerroEhrService = source.with_audit(out_sender);
    let target: FerroEhrService = target.with_audit(in_sender);

    let ehr = seed_full_ehr(&source).await;
    let extract = {
        let mut extracts = source.extract_ehrs(ehr).await.expect("extract_ehrs");
        openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("EXTRACT")
    };
    target.import_ehr(None, extract).await.expect("import_ehr");

    assert_eq!(
        extract_events(&src_pool).await,
        vec![("R".to_owned(), Some(ehr.to_string()))],
        "the export records one Read-direction extract event naming the EHR"
    );
    assert_eq!(
        extract_events(&dst_pool).await,
        vec![("C".to_owned(), Some(ehr.to_string()))],
        "the import records one Create-direction extract event naming the EHR"
    );
}

/// With no audit sender wired, the export still succeeds and writes nothing:
/// the emission is gated, and an unaudited deployment is not a failing one.
#[tokio::test]
async fn an_export_without_audit_configured_records_nothing_and_still_succeeds() {
    let (_db, pool, svc) = repository().await;

    let ehr = seed_full_ehr(&svc).await;
    let extracts = svc.extract_ehrs(ehr).await.expect("extract_ehrs");
    assert_eq!(extracts.len(), 1, "the export itself is unaffected");

    let recorded: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.audit_event")
        .fetch_one(&pool)
        .await
        .expect("audit count");
    assert_eq!(recorded, 0, "no sender means no audit record at all");
}

/// The extract record of each direction carries the categories of the data it
/// served or landed (EHDS Annex II 3.2(c), `docs/law/eu/ehds/text.html`): the
/// seeded EHR holds only an `EHR_STATUS`, its `EHR_ACCESS` and a directory, so
/// both records read `none` by resource kind, with the kinds as evidence.
#[tokio::test]
async fn the_extract_records_carry_the_categories_of_the_data() {
    let (_src_db, src_pool, source) = repository().await;
    let (_dst_db, dst_pool, target) = repository().await;
    let (out_sender, _out_handle): (_, AuditHandle) =
        start(store_only_config(), None, Some(src_pool.clone()))
            .await
            .expect("source audit sender");
    let (in_sender, _in_handle): (_, AuditHandle) =
        start(store_only_config(), None, Some(dst_pool.clone()))
            .await
            .expect("target audit sender");
    let source: FerroEhrService = source.with_audit(out_sender);
    let target: FerroEhrService = target.with_audit(in_sender);

    let ehr = seed_full_ehr(&source).await;
    let extract = {
        let mut extracts = source.extract_ehrs(ehr).await.expect("extract_ehrs");
        openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("EXTRACT")
    };
    target.import_ehr(None, extract).await.expect("import_ehr");

    for (pool, direction) in [(&src_pool, "export"), (&dst_pool, "import")] {
        assert_eq!(extract_events(pool).await.len(), 1, "{direction}");
        let row = sqlx::query(
            "SELECT categories, category_basis, category_evidence FROM audit.audit_event \
             WHERE resource_class = 'extract'",
        )
        .fetch_one(pool)
        .await
        .expect("the extract record");
        let categories: Option<serde_json::Value> = row.get("categories");
        let basis: Option<String> = row.get("category_basis");
        let evidence: Option<serde_json::Value> = row.get("category_evidence");
        assert_eq!(
            categories,
            Some(serde_json::json!(["none"])),
            "{direction}: the record is classified"
        );
        assert_eq!(basis.as_deref(), Some("resource-kind"), "{direction}");
        let evidence = evidence.expect("evidence");
        for kind in ["EHR_STATUS", "FOLDER"] {
            assert!(
                evidence
                    .as_array()
                    .is_some_and(|ids| ids.iter().any(|id| id == kind)),
                "{direction}: {kind} is evidence, got {evidence}"
            );
        }
    }
}
