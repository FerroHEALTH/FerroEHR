// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The retention of the access logs imported from another system (#5664,
//! #6242) against a real `PostgreSQL` 19: an imported entry is replaced by
//! its tombstone at the `retain_until` it states, its tombstone is kept as
//! long again and then folds, leaving its uid, bundle and content digest. The
//! stored bundle verifies after every step, across a later import from the
//! same system, and reports an imported record changed afterwards.
//!
//! No openEHR spec governs audit retention: our own design.

use jiff::Timestamp;
use serde_json::json;
use sqlx::PgPool;

use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log::expiry::{Log, expire, fold};
use ferroehr::access_log::imported::ImportedFailure;
use ferroehr::versioning::signature::trust::TrustedCertificates;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

use crate::access_log_checkpoint::{generate_key, import_record, keyring, tamper, trusting};

/// The system the log is exported from.
const SYSTEM: &str = "cdr-a.example.org";
/// The system that imports it.
const RECEIVER: &str = "cdr-b.example.org";
/// The EHR whose log moves first.
const MOVED: &str = "7d44b88c-4199-4bad-97dc-000000000001";
/// An EHR whose log moves later, or stays.
const OTHER: &str = "7d44b88c-4199-4bad-97dc-000000000002";

/// The instant `text` names.
fn at(text: &str) -> Timestamp {
    text.parse().expect("an instant")
}

/// A run past the retention of every entry kept to 2029.
fn expiry_run() -> Timestamp {
    at("2030-06-01T12:00:00Z")
}

/// A run past the end of every tombstone [`expiry_run`] made.
fn fold_run() -> Timestamp {
    at("2035-01-01T00:00:00Z")
}

/// Entry `n` of [`SYSTEM`] about `ehr_id`, recorded on 1 January 2026 and
/// kept to the end of `retain_until`.
fn entry(n: u32, ehr_id: &str, retain_until: &str) -> AccessLogEntry {
    entry_recorded(n, ehr_id, "2026-01-01T00:00:00Z", retain_until)
}

/// Entry `n` of [`SYSTEM`] about `ehr_id`, recorded at `recorded` and kept
/// to the end of `retain_until`.
fn entry_recorded(n: u32, ehr_id: &str, recorded: &str, retain_until: &str) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["system_id"] = json!(SYSTEM);
    value["ehr_id"]["value"] = json!(ehr_id);
    value["time_recorded"]["value"] = json!(recorded);
    value["retention"]["retain_until"]["value"] = json!(retain_until);
    serde_json::from_value(value).expect("the example entry reads")
}

/// The exporting system's log in `pool`, signing with `key`.
fn exporter(pool: &PgPool, key: &ferrohealth_openpgp_pq::key::Key) -> Checkpoints {
    Checkpoints::new(
        pool.clone(),
        SYSTEM.to_owned(),
        keyring(key),
        std::sync::Arc::new(TrustedCertificates::default()),
    )
}

/// The importing system's log in `pool`, trusting `key`.
fn importer(pool: &PgPool, key: &ferrohealth_openpgp_pq::key::Key) -> Checkpoints {
    Checkpoints::new(
        pool.clone(),
        RECEIVER.to_owned(),
        keyring(&generate_key()),
        trusting(key),
    )
}

/// The imported records, as (uid number, record type).
async fn records(pool: &PgPool) -> Vec<(String, String)> {
    sqlx::query_as(
        "SELECT right(uid, 2), record_type FROM audit.access_log_imported_record ORDER BY uid",
    )
    .fetch_all(pool)
    .await
    .expect("read the imported records")
}

/// The record type of an entry.
const ENTRY: &str = "ACCESS_LOG_ENTRY";
/// The record type of a tombstone.
const TOMBSTONE: &str = "ACCESS_LOG_TOMBSTONE";

/// An imported bundle ages out: its entries are replaced by their tombstones
/// at the `retain_until` they state, the tombstones are kept as long again
/// and then fold, and the bundle verifies after each step and reports an
/// imported record changed afterwards.
#[tokio::test]
async fn an_imported_bundle_ages_out_and_still_verifies() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    source
        .store()
        .append(vec![
            entry(1, MOVED, "2029-01-01"),
            entry(2, OTHER, "2029-01-01"),
            entry(3, MOVED, "2029-01-01"),
            entry(5, MOVED, "2040-01-01"),
        ])
        .await
        .expect("the entries append");
    let export = source.export(MOVED).await.expect("the export");
    assert_eq!(export.records.len(), 3);

    let pool = target_db.pool();
    let target = importer(&pool, &key);
    target
        .import(&export, import_record())
        .await
        .expect("the import verifies");
    assert_eq!(target.verify_imported().await.expect("verify"), []);

    let store = target.store();
    assert_eq!(
        expire(store, Log::Imported, at("2028-06-01T00:00:00Z"))
            .await
            .expect("expire"),
        0,
        "an imported entry is kept to its own retain_until"
    );
    assert_eq!(
        expire(store, Log::Imported, expiry_run())
            .await
            .expect("expire"),
        2
    );
    assert_eq!(
        records(&pool).await,
        [
            ("01".to_owned(), TOMBSTONE.to_owned()),
            ("03".to_owned(), TOMBSTONE.to_owned()),
            ("05".to_owned(), ENTRY.to_owned()),
        ]
    );
    let content: String = sqlx::query_scalar(
        "SELECT content FROM audit.access_log_imported_record WHERE right(uid, 2) = '01'",
    )
    .fetch_one(&pool)
    .await
    .expect("read the tombstone");
    assert!(
        !content.contains(MOVED),
        "the tombstone keeps no EHR: {content}"
    );
    assert_eq!(target.verify_imported().await.expect("verify"), []);

    let folded = fold(store, Log::Imported, fold_run()).await.expect("fold");
    assert_eq!(folded.tombstones, 2);
    assert_eq!(records(&pool).await, [("05".to_owned(), ENTRY.to_owned())]);
    let kept: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_imported_folded")
        .fetch_one(&pool)
        .await
        .expect("the folded");
    assert_eq!(
        kept, 2,
        "each folded tombstone keeps its uid, bundle and digest"
    );
    assert_eq!(target.verify_imported().await.expect("verify"), []);
    assert_eq!(
        target.verify(None).await.expect("verify").imported,
        [],
        "every bundle verifies"
    );

    tamper(
        &pool,
        "audit.access_log_imported_record",
        &[
            "UPDATE audit.access_log_imported_record SET content = replace(content, \
           '\"was_delivered\":true', '\"was_delivered\":false')",
        ],
    )
    .await;
    let findings = target.verify_imported().await.expect("verify");
    assert!(
        findings
            .iter()
            .any(|finding| matches!(finding.finding, ImportedFailure::Record(_))),
        "{findings:?}"
    );
}

/// A record removed from the imported table outside the fold breaks its
/// bundle's records digest.
#[tokio::test]
async fn an_imported_record_removed_outside_the_fold_is_found() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    source
        .store()
        .append(vec![
            entry(1, MOVED, "2040-01-01"),
            entry(3, MOVED, "2040-01-01"),
        ])
        .await
        .expect("the entries append");
    let export = source.export(MOVED).await.expect("the export");
    let pool = target_db.pool();
    let target = importer(&pool, &key);
    target
        .import(&export, import_record())
        .await
        .expect("the import verifies");
    tamper(
        &pool,
        "audit.access_log_imported_record",
        &["DELETE FROM audit.access_log_imported_record WHERE right(uid, 2) = '01'"],
    )
    .await;
    let findings = target.verify_imported().await.expect("verify");
    assert!(
        findings
            .iter()
            .any(|finding| finding.finding == ImportedFailure::Records),
        "{findings:?}"
    );
}

/// A later import of another EHR from the same system after a fold is kept
/// and verifies, and a repeated import of the first EHR does not bring back
/// the records whose tombstones folded here.
#[tokio::test]
async fn a_later_import_after_a_fold_keeps_what_ended_here_ended() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    source
        .store()
        .append(vec![
            entry(1, MOVED, "2029-01-01"),
            entry(2, OTHER, "2040-01-01"),
            entry(3, MOVED, "2029-01-01"),
        ])
        .await
        .expect("the entries append");
    let first = source.export(MOVED).await.expect("the first export");
    let pool = target_db.pool();
    let target = importer(&pool, &key);
    target
        .import(&first, import_record())
        .await
        .expect("the first import");
    let store = target.store();
    expire(store, Log::Imported, expiry_run())
        .await
        .expect("expire");
    fold(store, Log::Imported, fold_run()).await.expect("fold");
    assert_eq!(records(&pool).await, Vec::<(String, String)>::new());

    let later = source.export(OTHER).await.expect("the later export");
    target
        .import(&later, import_record())
        .await
        .expect("the later import");
    assert_eq!(records(&pool).await, [("02".to_owned(), ENTRY.to_owned())]);
    target
        .import(&first, import_record())
        .await
        .expect("a repeated import is recorded again");
    assert_eq!(
        records(&pool).await,
        [("02".to_owned(), ENTRY.to_owned())],
        "the folded records stay folded"
    );
    assert_eq!(target.verify_imported().await.expect("verify"), []);
}

/// A kept imported record rewritten together with its content digest, so
/// that its own digest still matches, breaks its bundle's records digest
/// (#6242).
#[tokio::test]
async fn a_kept_record_rewritten_with_its_digest_breaks_the_bundle() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    source
        .store()
        .append(vec![
            entry(1, MOVED, "2040-01-01"),
            entry(3, MOVED, "2040-01-01"),
        ])
        .await
        .expect("the entries append");
    let export = source.export(MOVED).await.expect("the export");
    let pool = target_db.pool();
    let target = importer(&pool, &key);
    target
        .import(&export, import_record())
        .await
        .expect("the import verifies");
    tamper(
        &pool,
        "audit.access_log_imported_record",
        &[
            "UPDATE audit.access_log_imported_record \
           SET content = replace(content, '\"was_delivered\":true', '\"was_delivered\":false') \
           WHERE right(uid, 2) = '01'",
            "UPDATE audit.access_log_imported_record \
           SET content_digest = encode(sha256(convert_to(content, 'UTF8')), 'hex') \
           WHERE right(uid, 2) = '01'",
        ],
    )
    .await;
    let findings = target.verify_imported().await.expect("verify");
    assert!(
        findings
            .iter()
            .any(|finding| finding.finding == ImportedFailure::Records),
        "{findings:?}"
    );
    assert!(
        !findings
            .iter()
            .any(|finding| matches!(finding.finding, ImportedFailure::Record(_))),
        "the record's own digest matches its rewritten content: {findings:?}"
    );
}

/// Two replicas folding the imported tombstones at once fold each one once,
/// and every bundle verifies after them (#6242).
#[tokio::test]
async fn two_replicas_fold_imported_bundles_at_once() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let key = generate_key();
    let source = exporter(&source_db.pool(), &key);
    let ehrs: Vec<String> = (1..=6_u32)
        .map(|n| format!("7d44b88c-4199-4bad-97dc-{n:012x}"))
        .collect();
    let mut n = 0_u32;
    for ehr in &ehrs {
        let batch: Vec<AccessLogEntry> = (0..10)
            .map(|_| {
                n += 1;
                entry(n, ehr, "2029-01-01")
            })
            .collect();
        source
            .store()
            .append(batch)
            .await
            .expect("the entries append");
    }
    let pool = target_db.pool();
    let target = importer(&pool, &key);
    for ehr in &ehrs {
        let export = source.export(ehr).await.expect("the export");
        target
            .import(&export, import_record())
            .await
            .expect("the import verifies");
    }
    expire(target.store(), Log::Imported, expiry_run())
        .await
        .expect("expire");

    let replica = |store: ferroehr::access_log::store::AccessLogStore| async move {
        let mut total = 0_u64;
        loop {
            let batch = store
                .fold_expired(Log::Imported, fold_run(), 5)
                .await
                .expect("a replica folds");
            total += batch.tombstones;
            if batch.tombstones == 0 {
                return total;
            }
        }
    };
    let (one, two) = tokio::join!(
        tokio::spawn(replica(ferroehr::access_log::store::AccessLogStore::new(
            pool.clone()
        ))),
        tokio::spawn(replica(ferroehr::access_log::store::AccessLogStore::new(
            pool.clone()
        )))
    );
    let one = one.expect("the first replica runs");
    let two = two.expect("the second replica runs");
    assert_eq!(one + two, 60, "every imported tombstone folds once");
    let folded: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_imported_folded")
        .fetch_one(&pool)
        .await
        .expect("the folded");
    assert_eq!(folded, 60);
    assert_eq!(target.verify_imported().await.expect("verify"), []);
}
