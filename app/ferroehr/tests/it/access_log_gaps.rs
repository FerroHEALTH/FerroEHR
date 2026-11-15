// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The fold of expired access-log tombstones into gaps of the chain (#5619)
//! against a real `PostgreSQL` 18: a tombstone is kept as long again as its
//! entry was, then each run of tombstones past their end folds into one gap,
//! and the chain, its incremental verification, the checkpoints, an export
//! and an import all verify across it, while a deletion outside the fold is
//! still reported.
//!
//! No openEHR spec governs audit retention: our own design.

use jiff::Timestamp;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::{Value, json};
use sqlx::PgPool;

use ferroehr::access_log::chain::{self, ChainFinding};
use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log::expiry::{Log, expire, fold};
use ferroehr::access_log::gap::Folded;
use ferroehr::access_log::store::AccessLogStore;
use ferroehr::access_log::transfer::LogExport;
use ferroehr::versioning::signature::trust::TrustedCertificates;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

use crate::access_log_checkpoint::{generate_key, import_record, keyring, trusting};

/// The system the log belongs to.
const SYSTEM: &str = "cdr-a.example.org";
/// The system an export moves to.
const RECEIVER: &str = "cdr-b.example.org";
/// The EHR whose log moves.
const MOVED: &str = "7d44b88c-4199-4bad-97dc-000000000001";
/// An EHR whose log stays.
const STAYS: &str = "7d44b88c-4199-4bad-97dc-000000000002";

/// The instant `text` names.
fn at(text: &str) -> Timestamp {
    text.parse().expect("an instant")
}

/// The day the entries' retention ends: four and a half years after they
/// were recorded, on 1 January 2026.
fn expiry_run() -> Timestamp {
    at("2030-06-01T12:00:00Z")
}

/// A day past every tombstone's own end that [`expiry_run`] made: as long
/// again after its removal.
fn fold_run() -> Timestamp {
    at("2035-01-01T00:00:00Z")
}

/// Entry `n` of [`SYSTEM`] about `ehr_id`, recorded on 1 January 2026 and
/// kept to the end of `retain_until`.
fn entry(n: u32, ehr_id: &str, retain_until: &str) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(uid(n));
    value["system_id"] = json!(SYSTEM);
    value["ehr_id"]["value"] = json!(ehr_id);
    value["time_recorded"]["value"] = json!("2026-01-01T00:00:00Z");
    value["retention"]["retain_until"]["value"] = json!(retain_until);
    serde_json::from_value(value).expect("the example entry reads")
}

/// The uid of entry `n`.
fn uid(n: u32) -> String {
    format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}")
}

/// Entries numbered from 1, one per `retain_until`, about [`STAYS`].
fn entries(retain_until: &[&str]) -> Vec<AccessLogEntry> {
    (1..)
        .zip(retain_until)
        .map(|(n, until)| entry(n, STAYS, until))
        .collect()
}

/// The findings of a full verification, as (position, text).
async fn findings(pool: &PgPool) -> Vec<(Option<i64>, String)> {
    crate::fixtures::verify_chain(pool)
        .await
        .expect("verify")
        .into_iter()
        .map(|finding| (finding.chain_seq, finding.finding))
        .collect()
}

/// The findings of an incremental verification of the access log from its
/// cursor, which a clean run moves to the newest position.
async fn incremental(pool: &PgPool) -> Vec<(Option<i64>, String)> {
    sqlx::query_as(
        "SELECT chain_seq, finding FROM audit.verify_access_log_chain_incremental(false)",
    )
    .fetch_all(pool)
    .await
    .expect("the incremental verification runs")
}

/// The stored gaps, as (first, last).
async fn gaps(pool: &PgPool) -> Vec<(i64, i64)> {
    sqlx::query_as("SELECT first_seq, last_seq FROM audit.access_log_chain_gap ORDER BY first_seq")
        .fetch_all(pool)
        .await
        .expect("read the gaps")
}

/// The positions that still hold a record.
async fn positions(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT chain_seq FROM audit.access_log_record ORDER BY chain_seq")
        .fetch_all(pool)
        .await
        .expect("read the records")
}

/// A ground of `code` in the `local` terminology.
fn ground(code: &str) -> DvCodedText {
    serde_json::from_value(json!({ "_type": "DV_CODED_TEXT", "value": code,
        "defining_code": { "terminology_id": { "value": "local" }, "code_string": code } }))
    .expect("the ground reads")
}

/// The checkpoints of `pool` for [`SYSTEM`], signing with a fresh key.
fn signed_log(pool: &PgPool) -> (Checkpoints, pgp::composed::SignedSecretKey) {
    let key = generate_key();
    let log = Checkpoints::new(
        pool.clone(),
        SYSTEM.to_owned(),
        keyring(&key),
        std::sync::Arc::new(TrustedCertificates::default()),
    );
    (log, key)
}

/// A tombstone is not folded before its own end, an entry never is, and once
/// past it each run of consecutive tombstones folds into one gap that keeps
/// the link of the run's last record and the grounds of its tombstones. The
/// chain and its incremental verification hold across the gaps, the head
/// included, a record appended after a gap links to it, and a record deleted
/// outside the fold is still reported.
#[tokio::test]
async fn a_run_of_expired_tombstones_folds_into_one_gap() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let written = store
        .append(entries(&[
            "2029-01-01",
            "2029-01-01",
            "2029-01-01",
            "2029-01-01",
            "2040-01-01",
            "2029-01-01",
        ]))
        .await
        .expect("the entries append");
    assert_eq!(incremental(&pool).await, [], "the log verifies before");

    assert_eq!(
        expire(&store, Log::Local, expiry_run())
            .await
            .expect("expire"),
        5
    );
    assert_eq!(
        fold(&store, Log::Local, expiry_run()).await.expect("fold"),
        Folded::default(),
        "a tombstone is kept past its removal"
    );
    assert_eq!(
        fold(&store, Log::Local, at("2034-10-30T00:00:00Z"))
            .await
            .expect("fold"),
        Folded::default(),
        "a tombstone is kept as long again as its entry was"
    );

    let folded = fold(&store, Log::Local, fold_run()).await.expect("fold");
    assert_eq!(
        folded,
        Folded {
            tombstones: 5,
            gaps: 2
        }
    );
    assert_eq!(gaps(&pool).await, [(1, 4), (6, 6)]);
    assert_eq!(positions(&pool).await, [5], "the entry kept longer stays");
    let stored = store.gaps(1, 6).await.expect("read the gaps");
    let links: Vec<&str> = stored.iter().map(|gap| gap.link.as_str()).collect();
    let expected: Vec<&str> = [3, 5]
        .iter()
        .map(|&index| written[index].link.digest.as_str())
        .collect();
    assert_eq!(links, expected, "each gap keeps its last record's link");
    for gap in &stored {
        assert_eq!(
            gap.grounds.as_ref(),
            written[0].retention.grounds.as_ref(),
            "the fold names the grounds of its tombstones"
        );
        assert_eq!(gap.time_folded.value, "2035-01-01T00:00:00Z");
    }

    assert_eq!(
        findings(&pool).await,
        [],
        "the log verifies across the gaps"
    );
    assert_eq!(incremental(&pool).await, [], "and incrementally");
    assert_eq!(
        fold(&store, Log::Local, fold_run())
            .await
            .expect("fold again"),
        Folded::default(),
        "a second run finds nothing"
    );

    store
        .append(vec![entry(7, STAYS, "2040-01-01")])
        .await
        .expect("an entry follows the folded head");
    assert_eq!(findings(&pool).await, [], "the new entry links to the gap");
    assert_eq!(incremental(&pool).await, []);

    crate::access_log_checkpoint::tamper(
        &pool,
        "audit.access_log_record",
        &["DELETE FROM audit.access_log_record WHERE chain_seq = 5"],
    )
    .await;
    let found = findings(&pool).await;
    assert!(
        found
            .iter()
            .any(|(at, text)| *at == Some(6) && text.contains("deleted between chain position 4")),
        "a deletion outside the fold is tampering: {found:?}"
    );
}

/// Entries expire in any order: a longer-kept entry pins the runs on both
/// sides apart, each run folds on its own, and the pinning entry folds into a
/// gap of its own once its tombstone's end passes, leaving adjacent gaps that
/// verify.
#[tokio::test]
async fn out_of_order_expiry_folds_into_adjacent_gaps() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    store
        .append(entries(&[
            "2029-01-01",
            "2033-01-01",
            "2029-01-01",
            "2029-01-01",
            "2040-01-01",
        ]))
        .await
        .expect("the entries append");

    assert_eq!(
        expire(&store, Log::Local, expiry_run())
            .await
            .expect("expire"),
        3
    );
    let folded = fold(&store, Log::Local, fold_run()).await.expect("fold");
    assert_eq!(folded.gaps, 2, "the pinning entry splits the run");
    assert_eq!(gaps(&pool).await, [(1, 1), (3, 4)]);
    assert_eq!(findings(&pool).await, []);
    assert_eq!(incremental(&pool).await, []);

    let later = at("2034-01-01T00:00:00Z");
    assert_eq!(expire(&store, Log::Local, later).await.expect("expire"), 1);
    assert_eq!(
        fold(&store, Log::Local, fold_run()).await.expect("fold"),
        Folded::default(),
        "its tombstone is kept as long as it was an entry"
    );
    let folded = fold(&store, Log::Local, at("2043-01-01T00:00:00Z"))
        .await
        .expect("fold");
    assert_eq!(
        folded,
        Folded {
            tombstones: 1,
            gaps: 1
        }
    );
    assert_eq!(gaps(&pool).await, [(1, 1), (2, 2), (3, 4)]);
    assert_eq!(positions(&pool).await, [5]);
    assert_eq!(findings(&pool).await, []);
    assert_eq!(incremental(&pool).await, []);
}

/// A run of tombstones folds whole across the checkpoints of the appends that
/// wrote it: a checkpoint inside the run leaves with it, and the one at its
/// last position stays and verifies against the gap's link. A gap of this
/// system's log carries no signature, since it signs checkpoints and no link
/// (#5704).
#[tokio::test]
async fn a_run_folds_across_the_checkpoints_inside_it() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let (log, _key) = signed_log(&pool);
    let all = entries(&["2029-01-01"; 6]);
    log.store()
        .append(all[..3].to_vec())
        .await
        .expect("the first entries append");
    let early = log
        .at(3)
        .await
        .expect("read")
        .expect("the first append's checkpoint");
    log.store()
        .append(all[3..].to_vec())
        .await
        .expect("the rest append");
    let late = log
        .at(6)
        .await
        .expect("read")
        .expect("the second append's checkpoint");

    expire(log.store(), Log::Local, expiry_run())
        .await
        .expect("expire");
    let folded = fold(log.store(), Log::Local, fold_run())
        .await
        .expect("fold");
    assert_eq!(folded.tombstones, 6);
    assert_eq!(gaps(&pool).await, [(1, 6)], "one gap for the whole run");
    assert!(
        log.store()
            .gaps(1, 6)
            .await
            .expect("read the gaps")
            .iter()
            .all(|gap| gap.signature.is_none()),
        "a gap of this system's log carries no signature"
    );
    assert_eq!(
        log.at(early.sequence).await.expect("read"),
        None,
        "the checkpoint inside the run leaves with it"
    );
    let verification = log.verify(Some(late.clone())).await.expect("verify");
    assert!(verification.intact(), "{verification:?}");
    assert_eq!(
        log.seal().await.expect("seal"),
        Some(late),
        "the folded head keeps its checkpoint"
    );
    let refused = sqlx::query("DELETE FROM audit.access_log_checkpoint")
        .execute(&pool)
        .await
        .expect_err("a checkpoint leaves only through the fold");
    assert!(refused.to_string().contains("append-only"), "{refused}");
}

/// An export of one EHR whose records have another EHR's folded tombstones
/// between them carries the gap, verifies across it and imports into another
/// system, which keeps the gap; an export missing the gap, or with its link
/// changed, does not verify.
#[tokio::test]
async fn an_export_spans_a_gap() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let pool = source_db.pool();
    let (log, key) = signed_log(&pool);
    log.store()
        .append(vec![
            entry(1, MOVED, "2040-01-01"),
            entry(2, STAYS, "2029-01-01"),
            entry(3, STAYS, "2029-01-01"),
            entry(4, MOVED, "2040-01-01"),
        ])
        .await
        .expect("the entries append");
    expire(log.store(), Log::Local, expiry_run())
        .await
        .expect("expire");
    fold(log.store(), Log::Local, fold_run())
        .await
        .expect("fold");
    assert_eq!(gaps(&pool).await, [(2, 3)]);

    let export = log.export(MOVED).await.expect("the export verifies");
    assert_eq!(export.records.len(), 2);
    let linked: Vec<i64> = export.links.iter().map(|link| link.sequence).collect();
    assert_eq!(linked, [1, 4]);
    let spans: Vec<(i64, i64)> = export
        .gaps
        .iter()
        .map(|gap| (gap.first_sequence, gap.last_sequence))
        .collect();
    assert_eq!(spans, [(2, 3)]);
    let trusted = |data: &[u8], signature: &str| log.signed(data, signature);
    assert_eq!(chain::verify(export.span(), &trusted).expect("verify"), []);

    let without = LogExport {
        gaps: Vec::new(),
        ..export.clone()
    };
    assert_eq!(
        chain::verify(without.span(), &trusted).expect("verify"),
        [ChainFinding::NotAdjacent {
            sequence: 4,
            previous: 1
        }],
        "a gap left out is a hole in the span"
    );
    let mut changed: Value = serde_json::to_value(&export).expect("the export writes");
    changed["gaps"][0]["link"] = json!("0".repeat(64));
    let changed: LogExport = serde_json::from_value(changed).expect("the export reads");
    let found = chain::verify(changed.span(), &trusted).expect("verify");
    assert_eq!(
        found,
        [ChainFinding::PreviousDigest { sequence: 4 }],
        "a changed gap breaks the link after it"
    );

    let wire = serde_json::to_string(&export).expect("the export writes");
    let received: LogExport = serde_json::from_str(&wire).expect("the export reads");
    assert_eq!(received, export);
    let target = Checkpoints::new(
        target_db.pool(),
        RECEIVER.to_owned(),
        keyring(&generate_key()),
        trusting(&key),
    );
    target
        .import(&received, import_record())
        .await
        .expect("the import verifies across the gap");
    let kept: Vec<(String, i64, i64)> =
        sqlx::query_as("SELECT system_id, first_seq, last_seq FROM audit.access_log_imported_gap")
            .fetch_all(&target_db.pool())
            .await
            .expect("read the imported gaps");
    assert_eq!(kept, [(SYSTEM.to_owned(), 2, 3)]);
    target
        .import(&received, import_record())
        .await
        .expect("a repeated import keeps the gap once");
}

/// An erasure a law orders is a tombstone like any other: it folds once it
/// has been a tombstone as long as it was an entry, alone between entries
/// still kept, and the gap names the erasure's grounds.
#[tokio::test]
async fn an_erased_entry_folds_like_any_other() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    store
        .append(entries(&["2040-01-01"; 3]))
        .await
        .expect("the entries append");
    let removed = serde_json::from_value(json!({ "_type": "DV_DATE_TIME",
        "value": "2027-01-01T00:00:00Z" }))
    .expect("the time reads");
    store
        .tombstone(
            &uid(2),
            removed,
            NonEmptyVec::new(vec![ground("court-order")]).expect("one ground"),
        )
        .await
        .expect("erase")
        .expect("the entry is replaced");

    assert_eq!(
        fold(&store, Log::Local, at("2027-12-31T00:00:00Z"))
            .await
            .expect("fold"),
        Folded::default()
    );
    let folded = fold(&store, Log::Local, at("2028-01-02T00:00:00Z"))
        .await
        .expect("fold");
    assert_eq!(
        folded,
        Folded {
            tombstones: 1,
            gaps: 1
        }
    );
    let stored = store.gaps(1, 3).await.expect("read the gaps");
    let codes: Vec<(i64, i64, &str)> = stored
        .iter()
        .flat_map(|gap| {
            gap.grounds.iter().map(|ground| {
                (
                    gap.first_sequence,
                    gap.last_sequence,
                    ground.defining_code.code_string.as_str(),
                )
            })
        })
        .collect();
    assert_eq!(codes, [(2, 2, "court-order")]);
    assert_eq!(positions(&pool).await, [1, 3]);
    assert_eq!(findings(&pool).await, []);
}

/// Two replicas folding at once never fold a tombstone twice: their gaps do
/// not overlap, together they hold every expired position, and the chain
/// verifies.
#[tokio::test]
async fn two_replicas_fold_at_once() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let count = 600_u32;
    store
        .append((1..=count).map(|n| entry(n, STAYS, "2029-01-01")).collect())
        .await
        .expect("the entries append");
    assert_eq!(
        expire(&store, Log::Local, expiry_run())
            .await
            .expect("expire"),
        u64::from(count)
    );

    let replica = |store: AccessLogStore| async move {
        let mut total = 0_u64;
        loop {
            let batch = store
                .fold_expired(Log::Local, fold_run(), 25)
                .await
                .expect("a replica folds");
            total += batch.tombstones;
            if batch.tombstones == 0 {
                return total;
            }
        }
    };
    let (one, two) = tokio::join!(
        tokio::spawn(replica(AccessLogStore::new(pool.clone()))),
        tokio::spawn(replica(AccessLogStore::new(pool.clone())))
    );
    let one = one.expect("the first replica runs");
    let two = two.expect("the second replica runs");
    assert_eq!(one + two, u64::from(count), "every tombstone folds once");

    let held = gaps(&pool).await;
    let mut next = 1_i64;
    for (first, last) in &held {
        assert_eq!(
            *first, next,
            "the gaps do not overlap or leave a hole: {held:?}"
        );
        next = last + 1;
    }
    assert_eq!(next, i64::from(count) + 1);
    assert_eq!(positions(&pool).await, Vec::<i64>::new());
    assert_eq!(findings(&pool).await, []);
}

/// The fold is the one way out: a tombstone deleted directly and a gap
/// changed or removed are refused.
#[tokio::test]
async fn a_gap_and_a_tombstone_leave_only_through_the_fold() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    store
        .append(entries(&["2029-01-01"; 3]))
        .await
        .expect("the entries append");
    expire(&store, Log::Local, expiry_run())
        .await
        .expect("expire");
    assert!(
        sqlx::query("DELETE FROM audit.access_log_record WHERE chain_seq = 1")
            .execute(&pool)
            .await
            .is_err(),
        "a tombstone is not deleted outside the fold"
    );
    fold(&store, Log::Local, fold_run()).await.expect("fold");
    for statement in [
        "UPDATE audit.access_log_chain_gap SET link = repeat('0', 64)",
        "DELETE FROM audit.access_log_chain_gap",
        "TRUNCATE audit.access_log_chain_gap",
    ] {
        assert!(
            sqlx::query(statement).execute(&pool).await.is_err(),
            "{statement} is refused"
        );
    }
    assert_eq!(gaps(&pool).await, [(1, 3)]);
    assert_eq!(findings(&pool).await, []);
}
