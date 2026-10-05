// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The admin storage-parity sweep against a real `PostgreSQL` 18 (shared
//! testkit harness).
//!
//! NOTE: no openEHR spec governs storage mechanics — our own design/extension;
//! the sweep re-derives every stored version from its `node` rows and compares
//! the result with the materialized `version.body`.
//!
//! Every tamper case reaches PAST the service into the stored rows with raw
//! SQL, which is the point: an attacker or a corrupt page does not go through
//! the write path, so neither does the fixture. A case that tampers must prove
//! the `UPDATE`/`DELETE` matched something, or it would pass while testing
//! nothing.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              fixture helpers; a failing fixture must panic at the fixture (the \
              Rust Book ch11)"
)]

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::integrity::StorageDomain;
use ferroehr::service::admin::integrity::rebuild::NodeRebuildOutcome;
use ferroehr::service::admin::integrity::{
    StorageParityDefect, StorageParityEvent, StorageParityScope,
};
use ferroehr::storage::codec::reassemble;
use ferroehr::storage::row::ReadRow;

use crate::fixtures::{composition, folder, uv};

/// An EHR carrying one committed COMPOSITION — three content versions in all
/// (`EHR_STATUS`, `EHR_ACCESS`, the COMPOSITION), every one of which the sweep
/// reads.
async fn seed_ehr_with_composition(svc: &FerroEhrService) -> EhrId {
    let ehr_id = svc.create_ehr(None).await.expect("ehr create");
    svc.create_composition(ehr_id, uv(&composition("Encounter"), "249", None))
        .await
        .expect("composition commit");
    ehr_id
}

/// The `(vo_id, sys_version)` of the one stored version of `kind` in `ehr_id`.
async fn one_version(pool: &PgPool, ehr_id: EhrId, kind: &str) -> (Uuid, i32) {
    sqlx::query_as(
        "SELECT vo_id, sys_version FROM version WHERE ehr_id = $1 AND kind = $2 \
         ORDER BY sys_version DESC LIMIT 1",
    )
    .bind(ehr_id.0)
    .bind(kind)
    .fetch_one(pool)
    .await
    .expect("a stored version of the requested kind")
}

/// Run a statement that must touch at least one row — a tamper fixture that
/// matched nothing would let its test pass while proving nothing.
async fn tamper(pool: &PgPool, sql: &'static str, vo_id: Uuid, sys_version: i32) {
    let affected = sqlx::query(sql)
        .bind(vo_id)
        .bind(sys_version)
        .execute(pool)
        .await
        .expect("the tamper statement runs")
        .rows_affected();
    assert!(
        affected > 0,
        "the tamper statement matched no rows, so nothing was actually tampered"
    );
}

#[tokio::test]
async fn a_committed_composition_sweeps_clean() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    seed_ehr_with_composition(&svc).await;

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");

    assert!(
        report.is_clean(),
        "a repository written through the commit path must be parity-clean: {report:?}"
    );
    assert!(report.mismatches.is_empty());
    assert!(
        report.versions_checked >= 3,
        "the EHR create plus one composition commit stores at least three versions, saw {}",
        report.versions_checked
    );
    assert_eq!(
        report.versions_checked,
        report.versions_with_body + report.versions_without_body
    );
}

#[tokio::test]
async fn a_tampered_node_row_is_reported_as_content_differs() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;

    // The AQL index copy, which read-time signature verification never sees.
    tamper(
        &pool,
        "UPDATE node SET data = jsonb_set(data, '{archetype_node_id}', '\"tampered\"') \
         WHERE vo_id = $1 AND sys_version = $2 AND num = 0",
        vo_id,
        sys_version,
    )
    .await;

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");

    assert_eq!(
        report.mismatch_count, 1,
        "exactly the tampered version must be reported: {report:?}"
    );
    let found = &report.mismatches[0];
    assert_eq!(found.vo_id, vo_id);
    assert_eq!(found.sys_version, sys_version);
    assert_eq!(found.kind, "COMPOSITION");
    assert_eq!(found.defect, StorageParityDefect::ContentDiffers);
    assert!(!report.truncated);
}

#[tokio::test]
async fn a_tampered_version_body_is_reported_as_content_differs() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;

    // The other copy: the materialized projection every point read serves.
    tamper(
        &pool,
        "UPDATE version SET body = (jsonb_set((body)::jsonb, '{archetype_node_id}', '\"tampered\"'))::text \
         WHERE vo_id = $1 AND sys_version = $2",
        vo_id,
        sys_version,
    )
    .await;

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");

    assert_eq!(report.mismatch_count, 1, "{report:?}");
    let found = &report.mismatches[0];
    assert_eq!(found.vo_id, vo_id);
    assert_eq!(found.sys_version, sys_version);
    assert_eq!(found.defect, StorageParityDefect::ContentDiffers);
}

#[tokio::test]
async fn a_version_whose_node_rows_are_gone_is_reported_as_nodes_missing() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;

    tamper(
        &pool,
        "DELETE FROM node WHERE vo_id = $1 AND sys_version = $2",
        vo_id,
        sys_version,
    )
    .await;

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");

    assert_eq!(report.mismatch_count, 1, "{report:?}");
    let found = &report.mismatches[0];
    assert_eq!(found.vo_id, vo_id);
    assert_eq!(found.sys_version, sys_version);
    assert_eq!(found.defect, StorageParityDefect::NodesMissing);
}

#[tokio::test]
async fn a_logically_deleted_version_sweeps_clean() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = svc.create_ehr(None).await.expect("ehr create");
    let dir = svc
        .create_directory(ehr_id, uv(&folder("root"), "249", None))
        .await
        .expect("directory create");
    svc.delete_directory(ehr_id, Some(dir.uid.parse().expect("ovid")), None)
        .await
        .expect("directory delete");

    // The delete committed a version with data Void (RM common master06
    // §Logical Deletion): no body, and no node rows to disagree with it.
    let bodiless: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM version WHERE ehr_id = $1 AND kind = 'FOLDER' AND body IS NULL",
    )
    .bind(ehr_id.0)
    .fetch_one(&pool)
    .await
    .expect("count");
    assert_eq!(bodiless, 1, "the delete must store one bodiless version");

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");

    assert!(report.is_clean(), "{report:?}");
    assert_eq!(report.versions_without_body, 1);
}

#[tokio::test]
async fn node_rows_under_a_bodiless_version_are_reported_as_unexpected_nodes() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = svc.create_ehr(None).await.expect("ehr create");
    let dir = svc
        .create_directory(ehr_id, uv(&folder("root"), "249", None))
        .await
        .expect("directory create");
    svc.delete_directory(ehr_id, Some(dir.uid.parse().expect("ovid")), None)
        .await
        .expect("directory delete");
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "FOLDER").await;

    // Give the bodiless version the previous version's rows: content in the
    // AQL index that no served version accounts for.
    let inserted = sqlx::query(
        "INSERT INTO node (vo_id, sys_version, ehr_id, num, num_cap, parent_num, \
         rm_type, archetype, arch_entity, arch_concept, arch_major, name, path, data) \
         SELECT vo_id, $2, ehr_id, num, num_cap, parent_num, rm_type, archetype, \
         arch_entity, arch_concept, arch_major, name, path, data \
         FROM node WHERE vo_id = $1 AND sys_version = $2 - 1",
    )
    .bind(vo_id)
    .bind(sys_version)
    .execute(&pool)
    .await
    .expect("insert the stray rows")
    .rows_affected();
    assert!(inserted > 0, "the fixture must actually insert stray rows");

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");

    assert_eq!(report.mismatch_count, 1, "{report:?}");
    let found = &report.mismatches[0];
    assert_eq!(found.vo_id, vo_id);
    assert_eq!(found.sys_version, sys_version);
    assert_eq!(found.defect, StorageParityDefect::UnexpectedNodes);
}

/// A scoped sweep reads only what its bounds cover, which is how an operator
/// verifies one record without reading the whole repository (#3110).
///
/// The tamper lives in one EHR; a sweep scoped to the other reports clean and
/// never even counts the damaged version, while the unscoped sweep finds it.
#[tokio::test]
async fn a_scoped_sweep_covers_only_its_own_ehr() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    let damaged = seed_ehr_with_composition(&svc).await;
    let intact = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, damaged, "COMPOSITION").await;
    tamper(
        &pool,
        "UPDATE version SET body = '{\"_type\":\"COMPOSITION\"}' \
         WHERE vo_id = $1 AND sys_version = $2",
        vo_id,
        sys_version,
    )
    .await;

    let clean = svc
        .verify_storage_parity(StorageParityScope {
            ehr_id: Some(intact.0),
            ..StorageParityScope::default()
        })
        .await
        .expect("scoped sweep");
    assert!(clean.is_clean(), "the intact EHR sweeps clean: {clean:?}");
    assert!(
        clean.versions_checked > 0,
        "the scoped sweep still read that EHR's own versions: {clean:?}"
    );

    let scoped = svc
        .verify_storage_parity(StorageParityScope {
            ehr_id: Some(damaged.0),
            ..StorageParityScope::default()
        })
        .await
        .expect("scoped sweep");
    assert_eq!(scoped.mismatch_count, 1, "the tamper is in this EHR");

    let whole = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("unscoped sweep");
    assert_eq!(whole.mismatch_count, 1);
    assert!(
        whole.versions_checked > scoped.versions_checked,
        "the unscoped sweep reads more than one EHR's versions: {whole:?} vs {scoped:?}"
    );
}

/// A `committed_since` bound covers only versions written at or after it, so
/// an operator can verify what changed since an incident (#3110).
#[tokio::test]
async fn a_committed_since_bound_excludes_earlier_versions() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    seed_ehr_with_composition(&svc).await;
    let before = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("unscoped sweep");
    assert!(before.versions_checked > 0);

    // The cutoff is read from the database, not the process: `committed_at` is
    // stamped by PostgreSQL's own clock, and a container's clock need not agree
    // with this process's to the millisecond.
    let cutoff: jiff_sqlx::Timestamp = sqlx::query_scalar("SELECT now()")
        .fetch_one(&pool)
        .await
        .expect("the database's own clock");
    let cutoff = cutoff.to_jiff();

    // Everything seeded so far is older than the cutoff, so it covers nothing;
    // the sweep succeeds and reports an empty count rather than failing or
    // silently falling back to the whole store.
    let after = svc
        .verify_storage_parity(StorageParityScope {
            committed_since: Some(cutoff),
            ..StorageParityScope::default()
        })
        .await
        .expect("bounded sweep");
    assert_eq!(after.versions_checked, 0, "{after:?}");

    // A version committed after the cutoff is covered.
    seed_ehr_with_composition(&svc).await;
    let later = svc
        .verify_storage_parity(StorageParityScope {
            committed_since: Some(cutoff),
            ..StorageParityScope::default()
        })
        .await
        .expect("bounded sweep");
    assert!(later.versions_checked > 0, "{later:?}");

    let whole = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("unscoped sweep");
    assert!(
        later.versions_checked < whole.versions_checked,
        "the bound still excludes the versions written before it: {later:?} vs {whole:?}"
    );
}

#[tokio::test]
async fn the_streamed_sweep_reports_the_same_findings_as_the_aggregated_one() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;
    tamper(
        &pool,
        "UPDATE node SET data = jsonb_set(data, '{archetype_node_id}', '\"tampered\"') \
         WHERE vo_id = $1 AND sys_version = $2 AND num = 0",
        vo_id,
        sys_version,
    )
    .await;

    let mut sweep = svc.storage_parity_sweep(StorageParityScope::default());
    let mut mismatches = Vec::new();
    let mut summary = None;
    let mut progress_ticks = 0_u32;
    while let Some(batch) = sweep.next_batch().await.expect("advance the sweep") {
        for event in batch {
            match event {
                StorageParityEvent::Mismatch(found) => mismatches.push(found),
                StorageParityEvent::Progress { .. } => progress_ticks += 1,
                StorageParityEvent::Summary { counts, .. } => {
                    assert!(summary.is_none(), "the summary is emitted exactly once");
                    summary = Some(counts);
                }
            }
        }
    }

    let counts = summary.expect("the sweep ends with a summary");
    assert!(
        progress_ticks > 0,
        "a sweep that read a page reports that it is advancing"
    );
    assert_eq!(mismatches.len(), 1, "{mismatches:?}");
    assert_eq!(mismatches[0].vo_id, vo_id);
    assert_eq!(mismatches[0].sys_version, sys_version);
    assert_eq!(mismatches[0].defect, StorageParityDefect::ContentDiffers);

    // The two shapes are one sweep, so their numbers cannot drift apart.
    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("aggregated sweep");
    assert_eq!(counts.versions_checked, report.versions_checked);
    assert_eq!(counts.versions_with_body, report.versions_with_body);
    assert_eq!(counts.versions_without_body, report.versions_without_body);
    assert_eq!(counts.mismatch_count, report.mismatch_count);
    assert_eq!(mismatches, report.mismatches);
}

#[tokio::test]
async fn a_dropped_stream_stops_the_sweep_where_it_stood() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    seed_ehr_with_composition(&svc).await;

    // One batch, then drop. Nothing may panic and nothing may run on: the
    // sweep holds no transaction and no lock, so abandoning it is the whole
    // cancellation story a disconnected client gets.
    let mut sweep = svc.storage_parity_sweep(StorageParityScope::default());
    let first = sweep.next_batch().await.expect("the first batch");
    assert!(
        first.is_some(),
        "a seeded repository yields at least one batch"
    );
    drop(sweep);

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("a later sweep is unaffected");
    assert!(report.versions_checked >= 3, "{report:?}");
}

// ── the rebuild: the repair the sweep is the diagnosis for (#3143) ───────────
//
// Every case here damages the derived copy the same way the sweep cases do,
// with raw SQL past the service, then repairs it through the admin operation
// and proves the repair by two independent readers: the sweep itself, and an
// AQL query, which reads the `node` rows and nothing else.

/// The `name/value` strings an AQL query over every COMPOSITION returns — a
/// reader that touches the `node` rows and never the materialized body, so it
/// sees the rebuild's actual output.
async fn aql_composition_names(svc: &FerroEhrService) -> Vec<String> {
    let result = svc
        .execute_ad_hoc_query(
            "SELECT c/name/value FROM COMPOSITION c".to_owned(),
            ferroehr::service::query::request::AqlQueryRequest::default(),
        )
        .await
        .expect("ad-hoc AQL")
        .result_set;
    result
        .get("rows")
        .and_then(Value::as_array)
        .expect("rows array")
        .iter()
        .filter_map(|row| row.as_array()?.first()?.as_str().map(str::to_owned))
        .collect()
}

#[tokio::test]
async fn a_rebuild_over_an_undamaged_repository_writes_nothing() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;

    let before: i64 = sqlx::query_scalar("SELECT count(*) FROM node WHERE ehr_id = $1")
        .bind(ehr_id.0)
        .fetch_one(&pool)
        .await
        .expect("node count");

    let report = svc
        .rebuild_version_nodes(StorageParityScope {
            ehr_id: Some(ehr_id.0),
            ..StorageParityScope::default()
        })
        .await
        .expect("rebuild");

    assert!(report.is_complete(), "nothing to refuse: {report:?}");
    assert_eq!(report.versions_damaged, 0, "nothing was damaged");
    assert_eq!(report.versions_rebuilt, 0, "so nothing was written");
    assert!(report.records.is_empty());
    assert!(
        report.versions_checked >= 3,
        "the sweep behind it still read the scope: {report:?}"
    );

    let after: i64 = sqlx::query_scalar("SELECT count(*) FROM node WHERE ehr_id = $1")
        .bind(ehr_id.0)
        .fetch_one(&pool)
        .await
        .expect("node count");
    assert_eq!(before, after, "a clean scope is left byte for byte alone");
}

#[tokio::test]
async fn a_tampered_node_row_is_rebuilt_and_the_sweep_goes_clean() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;

    // The AQL index copy: the composition's name, which an AQL projection of
    // `c/name/value` reads straight out of the damaged row.
    tamper(
        &pool,
        "UPDATE node SET data = jsonb_set(data, '{name,value}', '\"tampered\"') \
         WHERE vo_id = $1 AND sys_version = $2 AND num = 0",
        vo_id,
        sys_version,
    )
    .await;
    assert!(
        aql_composition_names(&svc)
            .await
            .contains(&"tampered".to_owned()),
        "the fixture really did corrupt what AQL reads"
    );

    let report = svc
        .rebuild_version_nodes(StorageParityScope::default())
        .await
        .expect("rebuild");

    assert_eq!(
        report.versions_damaged, 1,
        "exactly one damaged: {report:?}"
    );
    assert_eq!(report.versions_rebuilt, 1);
    assert_eq!(report.versions_refused, 0);
    let record = &report.records[0];
    assert_eq!(record.vo_id, vo_id);
    assert_eq!(record.sys_version, sys_version);
    assert_eq!(record.kind, "COMPOSITION");
    assert_eq!(record.defect, StorageParityDefect::ContentDiffers);
    let NodeRebuildOutcome::Rebuilt { node_rows } = &record.outcome else {
        panic!("expected a rebuild, got {:?}", record.outcome);
    };
    let stored: i64 =
        sqlx::query_scalar("SELECT count(*) FROM node WHERE vo_id = $1 AND sys_version = $2")
            .bind(vo_id)
            .bind(sys_version)
            .fetch_one(&pool)
            .await
            .expect("node count");
    assert_eq!(
        i64::from(*node_rows),
        stored,
        "the reported row count is what the version actually has"
    );
    assert!(stored > 0, "and the version has rows again");

    let after = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(after.is_clean(), "the sweep is clean afterwards: {after:?}");

    let names = aql_composition_names(&svc).await;
    assert!(
        !names.contains(&"tampered".to_owned()),
        "AQL no longer reads the corrupted value: {names:?}"
    );
    assert!(
        names.contains(&"Encounter".to_owned()),
        "AQL reads the restored value: {names:?}"
    );
}

#[tokio::test]
async fn a_version_whose_node_rows_are_gone_is_rebuilt_from_its_body() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;

    tamper(
        &pool,
        "DELETE FROM node WHERE vo_id = $1 AND sys_version = $2",
        vo_id,
        sys_version,
    )
    .await;

    let report = svc
        .rebuild_version_nodes(StorageParityScope {
            vo_id: Some(vo_id),
            sys_version: Some(sys_version),
            ..StorageParityScope::default()
        })
        .await
        .expect("rebuild");

    assert_eq!(
        report.versions_checked, 1,
        "the version-scoped rebuild read exactly that version: {report:?}"
    );
    assert_eq!(report.versions_rebuilt, 1);
    assert_eq!(
        report.records[0].defect,
        StorageParityDefect::NodesMissing,
        "the sweep verdict that selected it is carried through"
    );

    let after = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(
        after.is_clean(),
        "the whole store is clean again: {after:?}"
    );
    assert!(
        aql_composition_names(&svc)
            .await
            .contains(&"Encounter".to_owned()),
        "and AQL reaches the composition again"
    );
}

#[tokio::test]
async fn node_rows_under_a_bodiless_version_rebuild_to_none() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;

    // A logical delete (RM common master06 §Logical Deletion) stores no body,
    // so its node rows are unexpected — and rebuild to zero of them.
    tamper(
        &pool,
        "UPDATE version SET body = NULL WHERE vo_id = $1 AND sys_version = $2",
        vo_id,
        sys_version,
    )
    .await;

    let report = svc
        .rebuild_version_nodes(StorageParityScope::default())
        .await
        .expect("rebuild");

    assert_eq!(report.versions_rebuilt, 1, "{report:?}");
    assert_eq!(
        report.records[0].defect,
        StorageParityDefect::UnexpectedNodes
    );
    assert_eq!(
        report.records[0].outcome,
        NodeRebuildOutcome::Rebuilt { node_rows: 0 },
        "a bodiless version rebuilds to no node rows"
    );

    let remaining: i64 =
        sqlx::query_scalar("SELECT count(*) FROM node WHERE vo_id = $1 AND sys_version = $2")
            .bind(vo_id)
            .bind(sys_version)
            .fetch_one(&pool)
            .await
            .expect("node count");
    assert_eq!(remaining, 0, "the rows are gone");

    let after = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(after.is_clean(), "the sweep is clean afterwards: {after:?}");
}

#[tokio::test]
async fn a_body_that_does_not_decompose_is_refused_and_its_rows_are_untouched() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;

    // The body is the damaged copy here: `NOT_A_ROOT` is no versioned-object
    // root type, so `decompose` refuses it (`StorageError::NotAStructureRoot`).
    tamper(
        &pool,
        "UPDATE version SET body = '{\"_type\":\"NOT_A_ROOT\"}' \
         WHERE vo_id = $1 AND sys_version = $2",
        vo_id,
        sys_version,
    )
    .await;
    let before: Vec<(i32, String)> = sqlx::query_as(
        "SELECT num, path FROM node WHERE vo_id = $1 AND sys_version = $2 ORDER BY num",
    )
    .bind(vo_id)
    .bind(sys_version)
    .fetch_all(&pool)
    .await
    .expect("node rows");
    assert!(!before.is_empty(), "the version has rows to preserve");

    let report = svc
        .rebuild_version_nodes(StorageParityScope::default())
        .await
        .expect("rebuild");

    assert_eq!(report.versions_refused, 1, "{report:?}");
    assert_eq!(report.versions_rebuilt, 0);
    assert!(
        !report.is_complete(),
        "a refusal makes the run incomplete: {report:?}"
    );
    let NodeRebuildOutcome::Refused { reason } = &report.records[0].outcome else {
        panic!("expected a refusal, got {:?}", report.records[0].outcome);
    };
    assert!(
        reason.contains("does not decompose"),
        "the refusal names what is wrong with the body: {reason}"
    );

    let after: Vec<(i32, String)> = sqlx::query_as(
        "SELECT num, path FROM node WHERE vo_id = $1 AND sys_version = $2 ORDER BY num",
    )
    .bind(vo_id)
    .bind(sys_version)
    .fetch_all(&pool)
    .await
    .expect("node rows");
    assert_eq!(
        before, after,
        "a refused version's rows are left exactly as they were, never half-written"
    );
}

#[tokio::test]
async fn an_archived_version_is_rebuilt_and_stays_archived() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr_id = seed_ehr_with_composition(&svc).await;
    let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;

    tamper(
        &pool,
        "UPDATE node SET data = jsonb_set(data, '{name,value}', '\"tampered\"') \
         WHERE vo_id = $1 AND sys_version = $2 AND num = 0",
        vo_id,
        sys_version,
    )
    .await;
    svc.archive_ehrs(vec![ehr_id.0.to_string()])
        .await
        .expect("archive");
    let cold_before: i64 =
        sqlx::query_scalar("SELECT count(*) FROM node_cold WHERE vo_id = $1 AND sys_version = $2")
            .bind(vo_id)
            .bind(sys_version)
            .fetch_one(&pool)
            .await
            .expect("cold node count");
    assert!(cold_before > 0, "the object really moved to the cold tier");

    let report = svc
        .rebuild_version_nodes(StorageParityScope::default())
        .await
        .expect("rebuild");
    assert_eq!(report.versions_rebuilt, 1, "{report:?}");

    // The tier's rule is that a write thaws first, so the repair happens in
    // the primary tier — and the object must come back exactly as archived as
    // it went in.
    let still_marked: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM vo_head \
         WHERE vo_id = $1 AND tier = 'cold' AND archived_at IS NOT NULL)",
    )
    .bind(vo_id)
    .fetch_one(&pool)
    .await
    .expect("archive marker");
    assert!(still_marked, "the archive marker survives the repair");
    let primary: i64 =
        sqlx::query_scalar("SELECT count(*) FROM node_hot WHERE vo_id = $1 AND sys_version = $2")
            .bind(vo_id)
            .bind(sys_version)
            .fetch_one(&pool)
            .await
            .expect("primary node count");
    assert_eq!(primary, 0, "nothing was left behind in the primary tier");

    let after = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(
        after.is_clean(),
        "the sweep, which reads both tiers, is clean: {after:?}"
    );
}

#[tokio::test]
async fn a_scoped_rebuild_repairs_only_its_own_ehr() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let repaired = seed_ehr_with_composition(&svc).await;
    let left_alone = seed_ehr_with_composition(&svc).await;

    for ehr_id in [repaired, left_alone] {
        let (vo_id, sys_version) = one_version(&pool, ehr_id, "COMPOSITION").await;
        tamper(
            &pool,
            "UPDATE node SET data = jsonb_set(data, '{name,value}', '\"tampered\"') \
             WHERE vo_id = $1 AND sys_version = $2 AND num = 0",
            vo_id,
            sys_version,
        )
        .await;
    }

    let report = svc
        .rebuild_version_nodes(StorageParityScope {
            ehr_id: Some(repaired.0),
            ..StorageParityScope::default()
        })
        .await
        .expect("rebuild");
    assert_eq!(report.versions_rebuilt, 1, "one EHR's damage: {report:?}");

    let remaining = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert_eq!(
        remaining.mismatch_count, 1,
        "the other EHR's damage is untouched: {remaining:?}"
    );
    assert_eq!(
        remaining.mismatches[0].vo_id,
        one_version(&pool, left_alone, "COMPOSITION").await.0
    );
}

// ── both pseudonymisation domains (#3178) ────────────────────────────────────
//
// Parties left the clinical schema in #3153, and the sweep kept reading only
// the schema it always had. Damage to the copy nobody recomputes went
// undetected on the domain holding the identifying data, which is the one
// place a silent integrity gap is least acceptable. These cases pin the
// coverage so a future change cannot narrow it back without failing.

/// A committed party, and the `(vo_id, sys_version)` of the version holding it.
async fn seed_party(svc: &FerroEhrService, pool: &PgPool) -> (Uuid, i32) {
    svc.party_create(
        ferroehr::service::demographic::types::PartyKind::Person,
        crate::typed_body::typed(&crate::pseudonymisation_boundary::a_person()),
        None,
    )
    .await
    .expect("create a person through the service seam");
    sqlx::query_as(
        "SELECT vo_id, sys_version FROM party.version \
         ORDER BY sys_version DESC LIMIT 1",
    )
    .fetch_one(pool)
    .await
    .expect("the stored party version")
}

#[tokio::test]
async fn a_damaged_party_is_reported_by_a_sweep_with_no_scope() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let (vo_id, sys_version) = seed_party(&svc, &pool).await;

    tamper(
        &pool,
        "UPDATE party.node SET data = jsonb_set(data, '{name,value}', '\"tampered\"') \
         WHERE vo_id = $1 AND sys_version = $2 AND num = 0",
        vo_id,
        sys_version,
    )
    .await;

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");

    assert_eq!(
        report.mismatch_count, 1,
        "an unscoped sweep reads the demographic domain too: {report:?}"
    );
    let found = &report.mismatches[0];
    assert_eq!(found.vo_id, vo_id);
    assert_eq!(
        found.domain,
        StorageDomain::Demographic,
        "and says which domain the damage is in"
    );
    assert_eq!(found.defect, StorageParityDefect::ContentDiffers);
}

#[tokio::test]
async fn a_sweep_with_no_scope_counts_versions_from_both_domains() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    seed_ehr_with_composition(&svc).await;

    let clinical_only = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep")
        .versions_checked;

    seed_party(&svc, &pool).await;
    let both = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep")
        .versions_checked;

    assert!(
        both > clinical_only,
        "committing a party raises the unscoped count, so the demographic \
         domain is actually read rather than skipped: {clinical_only} then {both}"
    );
}

#[tokio::test]
async fn a_damaged_party_is_rebuilt_and_reads_correctly_afterwards() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let (vo_id, sys_version) = seed_party(&svc, &pool).await;

    tamper(
        &pool,
        "UPDATE party.node SET data = jsonb_set(data, '{name,value}', '\"tampered\"') \
         WHERE vo_id = $1 AND sys_version = $2 AND num = 0",
        vo_id,
        sys_version,
    )
    .await;

    let report = svc
        .rebuild_version_nodes(StorageParityScope::default())
        .await
        .expect("rebuild");

    assert_eq!(report.versions_rebuilt, 1, "{report:?}");
    assert_eq!(
        report.records[0].domain,
        StorageDomain::Demographic,
        "the record names the domain it repaired"
    );

    let after = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(after.is_clean(), "the sweep is clean afterwards: {after:?}");

    // The node rows are what a decomposed read serves, so this is the reader
    // that would still have seen the damage.
    let name: String = sqlx::query_scalar(
        "SELECT data #>> '{name,value}' FROM party.node \
         WHERE vo_id = $1 AND sys_version = $2 AND num = 0",
    )
    .bind(vo_id)
    .bind(sys_version)
    .fetch_one(&pool)
    .await
    .expect("read the rebuilt name");
    assert_ne!(name, "tampered", "the corrupted value is gone");
}

// ── an older decomposition (#3273) ───────────────────────────────────────────
//
// Rows an older decomposition wrote are not damaged: they reassemble into
// exactly their body. Only the second comparison — against the rows this build
// would write — tells them apart from healthy ones, and the rebuild is the
// upgrade step that brings them over.

/// A PERSON whose contact address carries the leaf a row-level predicate binds
/// to, plus a top-level `details` `ITEM_TREE` that was a row under the older
/// decomposition too.
///
/// The contact subtree is what distinguishes the two shapes: `CONTACT`,
/// `ADDRESS` and `PARTY_IDENTITY` earn their own rows now
/// (`ferroehr::storage::structure`) and were inline on the party root before.
///
/// RM shapes: `PERSON` (`identities` 1..*, `contacts`, `details`), `CONTACT`
/// (`addresses` 1..*), `ADDRESS` (`details`), `ITEM_TREE` (`items`), `ELEMENT`
/// (`value`) — `openehr_rm::v1_2`.
fn a_person_with_a_contact_address() -> Value {
    serde_json::json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0"
        },
        "name": { "_type": "DV_TEXT", "value": "Ada Lovelace" },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0003",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": "Lovelace" }
                }]
            }
        }],
        "contacts": [{
            "_type": "CONTACT",
            "archetype_node_id": "at0005",
            "name": { "_type": "DV_TEXT", "value": "home" },
            "addresses": [{
                "_type": "ADDRESS",
                "archetype_node_id": "openEHR-DEMOGRAPHIC-ADDRESS.address.v1",
                "archetype_details": {
                    "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-DEMOGRAPHIC-ADDRESS.address.v1" },
                    "rm_version": "1.1.0"
                },
                "name": { "_type": "DV_TEXT", "value": "postal" },
                "details": {
                    "_type": "ITEM_TREE",
                    "archetype_node_id": "at0006",
                    "name": { "_type": "DV_TEXT", "value": "address" },
                    "items": [{
                        "_type": "ELEMENT",
                        "archetype_node_id": "at0012",
                        "name": { "_type": "DV_TEXT", "value": "city" },
                        "value": { "_type": "DV_TEXT", "value": "Groningen" }
                    }]
                }
            }]
        }],
        "details": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0010",
            "name": { "_type": "DV_TEXT", "value": "structure" },
            "items": [{
                "_type": "ELEMENT",
                "archetype_node_id": "at0011",
                "name": { "_type": "DV_TEXT", "value": "sex" },
                "value": { "_type": "DV_TEXT", "value": "F" }
            }]
        }
    })
}

/// Commit [`a_person_with_a_contact_address`] through the service seam, and
/// return the versioned object plus the storage ordinal of its one version.
async fn seed_contactable_party(svc: &FerroEhrService, pool: &PgPool) -> (VoId, i32) {
    let vo_id = Box::pin(svc.create_party(uv(&a_person_with_a_contact_address(), "249", None)))
        .await
        .expect("create a person through the service seam");
    let sys_version: i32 = sqlx::query_scalar(
        "SELECT sys_version FROM party.version WHERE vo_id = $1 \
         ORDER BY sys_version DESC LIMIT 1",
    )
    .bind(vo_id.0)
    .fetch_one(pool)
    .await
    .expect("the stored party version");
    (vo_id, sys_version)
}

/// The stored `version.body` of one demographic version, parsed.
async fn stored_demographic_body(pool: &PgPool, vo_id: VoId, sys_version: i32) -> Value {
    let text: String =
        sqlx::query_scalar("SELECT body FROM party.version WHERE vo_id = $1 AND sys_version = $2")
            .bind(vo_id.0)
            .bind(sys_version)
            .fetch_one(pool)
            .await
            .expect("the stored body");
    serde_json::from_str(&text).expect("the stored body parses")
}

/// The stored node rows of one demographic version, in the read shape the
/// codec reassembles from.
async fn stored_demographic_rows(pool: &PgPool, vo_id: VoId, sys_version: i32) -> Vec<ReadRow> {
    let rows: Vec<(i32, i32, i32, String, Value)> = sqlx::query_as(
        "SELECT num, num_cap, parent_num, path, data FROM party.node \
         WHERE vo_id = $1 AND sys_version = $2 ORDER BY num",
    )
    .bind(vo_id.0)
    .bind(sys_version)
    .fetch_all(pool)
    .await
    .expect("the stored node rows");
    rows.into_iter()
        .map(|(num, num_cap, parent_num, path, data)| ReadRow {
            num,
            num_cap,
            parent_num,
            path,
            data,
        })
        .collect()
}

/// Rewrite one party version's rows into the shape the decomposition wrote
/// before the demographic containers earned rows of their own: the
/// `identities` and `contacts` subtrees inline on the party root, and only the
/// top-level `details` `ITEM_STRUCTURE` split out.
///
/// The rewrite reaches past the service into the stored rows on purpose — an
/// installation that upgrades has exactly these rows and no way to have
/// written them through this build's commit path.
async fn write_the_old_party_shape(pool: &PgPool, vo_id: VoId, sys_version: i32) {
    let removed = sqlx::query(
        "DELETE FROM party.node WHERE vo_id = $1 AND sys_version = $2 \
         AND (path LIKE 'identities%' OR path LIKE 'contacts%')",
    )
    .bind(vo_id.0)
    .bind(sys_version)
    .execute(pool)
    .await
    .expect("drop the container rows")
    .rows_affected();
    assert!(
        removed > 0,
        "the fixture must actually remove the container rows, or it models nothing"
    );

    // The old root fragment is the body with its one structure child pruned,
    // which is exactly what the pre-#3273 codec kept inline.
    let inlined = sqlx::query(
        "UPDATE party.node n SET data = (\
           SELECT v.body::jsonb - 'details' FROM party.version v \
           WHERE v.vo_id = n.vo_id AND v.sys_version = n.sys_version) \
         WHERE n.vo_id = $1 AND n.sys_version = $2 AND n.num = 0",
    )
    .bind(vo_id.0)
    .bind(sys_version)
    .execute(pool)
    .await
    .expect("inline the containers on the party root")
    .rows_affected();
    assert_eq!(inlined, 1, "the fixture must rewrite exactly the root row");
}

#[tokio::test]
async fn a_stale_decomposition_is_reported_and_rebuilt() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let (vo_id, sys_version) = seed_contactable_party(&svc, &pool).await;
    let served = svc.get_party(vo_id).await.expect("read the fresh party");

    write_the_old_party_shape(&pool, vo_id, sys_version).await;

    // The tampering really does model the older shape rather than damage: the
    // rows still reassemble into exactly the body they were written from, so
    // the only thing left to disagree about is their shape.
    let body = stored_demographic_body(&pool, vo_id, sys_version).await;
    let old_rows = stored_demographic_rows(&pool, vo_id, sys_version).await;
    assert_eq!(
        reassemble(&old_rows).expect("the old rows form one tree"),
        body,
        "an older decomposition carries the same content, which is why the \
         reassembly comparison alone passes it as healthy"
    );
    assert!(
        !old_rows.iter().any(|row| row.path.starts_with("contacts")),
        "and it carries the contact subtree inline, out of reach of a row-level predicate"
    );

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert_eq!(
        report.mismatch_count, 1,
        "exactly the re-shaped version is reported: {report:?}"
    );
    let found = &report.mismatches[0];
    assert_eq!(found.vo_id, vo_id.0);
    assert_eq!(found.sys_version, sys_version);
    assert_eq!(found.domain, StorageDomain::Demographic);
    assert_eq!(found.defect, StorageParityDefect::StaleDecomposition);

    let rebuild = svc
        .rebuild_version_nodes(StorageParityScope::default())
        .await
        .expect("rebuild");
    assert_eq!(rebuild.versions_rebuilt, 1, "{rebuild:?}");
    assert_eq!(rebuild.versions_refused, 0);
    assert_eq!(
        rebuild.records[0].defect,
        StorageParityDefect::StaleDecomposition,
        "the sweep verdict that selected it is carried through"
    );

    let after = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(
        after.is_clean(),
        "one rebuild is the whole upgrade step: {after:?}"
    );
    let current_rows = stored_demographic_rows(&pool, vo_id, sys_version).await;
    assert!(
        current_rows
            .iter()
            .any(|row| row.path == "contacts0.addresses0."),
        "the contact address is a row of its own now, so a predicate bound to \
         its archetype reaches this version: {:?}",
        current_rows.iter().map(|row| &row.path).collect::<Vec<_>>()
    );

    assert_eq!(
        svc.get_party(vo_id).await.expect("read the repaired party"),
        served,
        "and the served body never moved: only the index over it did"
    );
}

#[tokio::test]
async fn a_freshly_committed_repository_reports_no_stale_decomposition() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    seed_ehr_with_composition(&svc).await;
    seed_contactable_party(&svc, &pool).await;

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");

    assert!(
        report.is_clean(),
        "a clinical composition and a party written by this build are both \
         already in the current shape, so the second comparison must find \
         nothing: {report:?}"
    );
    assert!(
        report.versions_checked >= 4,
        "and both domains were actually read: {report:?}"
    );
}
