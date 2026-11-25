// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The fold of expired access-log tombstones (#5619, #6242) against a real
//! `PostgreSQL` 19: a tombstone is kept as long again as its entry was, then
//! the entries of a sealed window whose records are all tombstones past their
//! end leave together, the window recorded as folded with its seal kept, and
//! the verification, the checkpoints, an export and an import all hold after
//! it, while a deletion outside the fold is still reported.
//!
//! No openEHR spec governs audit retention: our own design.

use jiff::Timestamp;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::json;
use sqlx::PgPool;

use ferroehr::access_log::chain::ChainFinding;
use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::access_log::expiry::{Log, expire, fold};
use ferroehr::access_log::gap::Folded;
use ferroehr::access_log::publish::Publication;
use ferroehr::access_log::store::AccessLogStore;
use ferroehr::versioning::signature::trust::TrustedCertificates;
use openehr_rm::draft::access_log::access_log::access_log_checkpoint::AccessLogCheckpoint;
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
    ferroehr::system_log::store::AuditStore::new(pool.clone())
        .verify_chain_advancing(
            ferroehr::system_log::store::chains::AuditChain::AccessLog,
            ferroehr::system_log::store::chains::VerifyScope::SinceCheckpoint,
        )
        .await
        .expect("the incremental verification runs")
        .into_iter()
        .map(|finding| (finding.chain_seq, finding.finding))
        .collect()
}

/// The folded windows, as (first, last).
async fn folds(pool: &PgPool) -> Vec<(i64, i64)> {
    sqlx::query_as(
        "SELECT s.first_id, s.last_id FROM audit.access_log_seal_fold f \
         JOIN audit.access_log_seal s ON s.window_no = f.window_no ORDER BY s.window_no",
    )
    .fetch_all(pool)
    .await
    .expect("read the folds")
}

/// Append `entries` and seal them in a window of their own.
async fn sealed(store: &AccessLogStore, entries: Vec<AccessLogEntry>) {
    store.append(entries).await.expect("the entries append");
    store
        .seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
        .await
        .expect("sealed");
}

/// The entry ids that still hold a record.
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
fn signed_log(pool: &PgPool) -> (Checkpoints, ferrohealth_openpgp_pq::key::Key) {
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
/// every record of a sealed window is a tombstone past its end the window's
/// entries leave together and the window is recorded as folded; the
/// verification and its incremental run hold after it, a later window
/// follows the folded one, and a record deleted outside the fold is still
/// reported.
#[tokio::test]
async fn a_window_of_expired_tombstones_folds() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    sealed(&store, entries(&["2029-01-01"; 4])).await;
    assert_eq!(
        expire(&store, Log::Local, expiry_run())
            .await
            .expect("expire"),
        4
    );
    assert_eq!(
        fold(&store, Log::Local, expiry_run()).await.expect("fold"),
        Folded::default(),
        "no tombstone is past its own end yet"
    );
    let folded = fold(&store, Log::Local, fold_run()).await.expect("fold");
    assert_eq!(
        folded,
        Folded {
            tombstones: 4,
            gaps: 1
        }
    );
    assert_eq!(positions(&pool).await, Vec::<i64>::new());
    assert_eq!(folds(&pool).await, [(1, 4)]);
    assert_eq!(findings(&pool).await, []);
    assert_eq!(incremental(&pool).await, []);

    sealed(
        &store,
        vec![entry(5, STAYS, "2040-01-01"), entry(6, STAYS, "2040-01-01")],
    )
    .await;
    assert_eq!(
        findings(&pool).await,
        [],
        "a later window follows the folded one"
    );
    sqlx::query("ALTER TABLE audit.access_log_entry DISABLE TRIGGER USER")
        .execute(&pool)
        .await
        .expect("as the owner");
    sqlx::query("DELETE FROM audit.access_log_entry WHERE id = 5")
        .execute(&pool)
        .await
        .expect("the deletion");
    sqlx::query("ALTER TABLE audit.access_log_entry ENABLE TRIGGER USER")
        .execute(&pool)
        .await
        .expect("as the owner");
    assert!(
        !findings(&pool).await.is_empty(),
        "a record deleted outside the fold is reported"
    );
}

/// Entries expire in any order: a longer-kept entry keeps its window's
/// tombstones until its own tombstone's end has passed, and then the window
/// folds whole.
#[tokio::test]
async fn a_longer_kept_entry_keeps_its_window() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    sealed(&store, entries(&["2029-01-01", "2030-01-01", "2029-01-01"])).await;
    expire(&store, Log::Local, at("2029-06-01T00:00:00Z"))
        .await
        .expect("expire");
    assert_eq!(
        fold(&store, Log::Local, at("2033-01-01T00:00:00Z"))
            .await
            .expect("fold"),
        Folded::default(),
        "the entry kept to 2030 keeps the window"
    );
    assert_eq!(positions(&pool).await, [1, 2, 3]);
    expire(&store, Log::Local, at("2030-06-01T00:00:00Z"))
        .await
        .expect("expire");
    let folded = fold(&store, Log::Local, at("2040-01-01T00:00:00Z"))
        .await
        .expect("fold");
    assert_eq!(folded.tombstones, 3);
    assert_eq!(folds(&pool).await, [(1, 3)]);
    assert_eq!(findings(&pool).await, []);
}

/// A folded window keeps its seal, so a checkpoint published before the fold
/// still verifies after it, and one whose proof was replaced does not.
#[tokio::test]
async fn a_published_checkpoint_verifies_after_its_window_folds() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let (log, _key) = signed_log(&pool);
    let directory = std::env::temp_dir().join(format!(
        "ferroehr-fold-publication-{}-{}",
        std::process::id(),
        uuid::Uuid::now_v7()
    ));
    std::fs::create_dir_all(&directory).expect("the publication directory");
    let publication = Publication::new(Some(directory.clone()));
    let all = entries(&["2029-01-01"; 6]);
    sealed(log.store(), all[..3].to_vec()).await;
    assert_eq!(publication.publish(&log).await.expect("publish"), Some(3));
    sealed(log.store(), all[3..].to_vec()).await;

    expire(log.store(), Log::Local, expiry_run())
        .await
        .expect("expire");
    fold(log.store(), Log::Local, fold_run())
        .await
        .expect("fold");
    assert_eq!(folds(&pool).await, [(1, 3), (4, 6)]);

    let file = std::fs::read_dir(&directory)
        .expect("the directory")
        .next()
        .expect("one published file")
        .expect("a directory entry")
        .path();
    let published: AccessLogCheckpoint =
        serde_json::from_str(&std::fs::read_to_string(file).expect("the published file"))
            .expect("the published checkpoint reads");
    assert_eq!(published.sequence, 3);
    let verification = log.verify(Some(published.clone())).await.expect("verify");
    assert!(verification.intact(), "{verification:?}");

    let mut forged = published;
    forged.proof = log
        .at(6)
        .await
        .expect("read")
        .expect("the newest checkpoint")
        .proof;
    let verification = log.verify(Some(forged)).await.expect("verify");
    assert!(
        verification
            .findings
            .contains(&ChainFinding::CheckpointProof { sequence: 3 }),
        "a proof over another checkpoint does not verify: {verification:?}"
    );
    std::fs::remove_dir_all(directory).expect("clean up");
}

/// An export of one EHR after the windows of another EHR's records folded
/// verifies and imports into another system.
#[tokio::test]
async fn an_export_after_a_fold_verifies_and_imports() {
    let source_db = testkit::db().await.expect("source database");
    let target_db = testkit::db().await.expect("target database");
    let pool = source_db.pool();
    let (log, key) = signed_log(&pool);
    sealed(log.store(), entries(&["2029-01-01"; 3])).await;
    sealed(
        log.store(),
        vec![entry(4, MOVED, "2040-01-01"), entry(5, MOVED, "2040-01-01")],
    )
    .await;
    expire(log.store(), Log::Local, expiry_run())
        .await
        .expect("expire");
    fold(log.store(), Log::Local, fold_run())
        .await
        .expect("fold");
    assert_eq!(folds(&pool).await, [(1, 3)]);

    let export = log.export(MOVED).await.expect("export");
    assert_eq!(export.records.len(), 2);
    assert_eq!(log.verify_bundle(&export).expect("verify"), Vec::new());
    let receiver = Checkpoints::new(
        target_db.pool(),
        RECEIVER.to_owned(),
        keyring(&generate_key()),
        trusting(&key),
    );
    let imported = receiver
        .import(&export, import_record())
        .await
        .expect("the import verifies");
    assert_eq!(imported.records, 2);
}

/// An erasure a law orders is a tombstone like any other: its window folds
/// once it has been a tombstone as long as it was an entry, and the fold
/// names the erasure's grounds.
#[tokio::test]
async fn an_erased_entry_folds_like_any_other() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    for n in 1..=3 {
        sealed(&store, vec![entry(n, STAYS, "2040-01-01")]).await;
    }
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
    let grounds: Vec<String> = sqlx::query_scalar(
        "SELECT g ->> 'value' FROM audit.access_log_seal_fold f, \
         jsonb_array_elements(f.grounds) AS g",
    )
    .fetch_all(&pool)
    .await
    .expect("the fold's grounds");
    assert_eq!(grounds, ["court-order"]);
    assert_eq!(positions(&pool).await, [1, 3]);
    assert_eq!(findings(&pool).await, []);
}

/// Two replicas folding at once never fold a window twice: together they
/// fold every expired window once, and the windows verify.
#[tokio::test]
async fn two_replicas_fold_at_once() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let windows = 60_u32;
    for window in 0..windows {
        sealed(
            &store,
            (1..=10)
                .map(|n| entry(window * 10 + n, STAYS, "2029-01-01"))
                .collect(),
        )
        .await;
    }
    expire(&store, Log::Local, expiry_run())
        .await
        .expect("expire");

    let replica = |store: AccessLogStore| async move {
        let mut total = 0_u64;
        loop {
            let batch = store
                .fold_expired(Log::Local, fold_run(), 5)
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
    assert_eq!(
        one + two,
        u64::from(windows * 10),
        "every tombstone folds once"
    );
    assert_eq!(
        folds(&pool).await.len(),
        usize::try_from(windows).expect("a count")
    );
    assert_eq!(positions(&pool).await, Vec::<i64>::new());
    assert_eq!(findings(&pool).await, []);
}

/// The fold is the one way out: a tombstone deleted directly and a fold
/// record changed or removed are refused.
#[tokio::test]
async fn a_fold_and_a_tombstone_leave_only_through_the_fold() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    sealed(&store, entries(&["2029-01-01"; 3])).await;
    expire(&store, Log::Local, expiry_run())
        .await
        .expect("expire");
    assert!(
        sqlx::query("DELETE FROM audit.access_log_entry WHERE id = 1")
            .execute(&pool)
            .await
            .is_err(),
        "a tombstone is not deleted outside the fold"
    );
    fold(&store, Log::Local, fold_run()).await.expect("fold");
    for statement in [
        "UPDATE audit.access_log_seal_fold SET grounds = '[]'",
        "DELETE FROM audit.access_log_seal_fold",
        "TRUNCATE audit.access_log_seal_fold",
    ] {
        assert!(
            sqlx::query(statement).execute(&pool).await.is_err(),
            "{statement} is refused"
        );
    }
    assert_eq!(folds(&pool).await, [(1, 3)]);
    assert_eq!(findings(&pool).await, []);
}

/// One fold run folds every sealed window whose records are all tombstones
/// past their end, across the seals between them, and the windows, their
/// seals and the newest checkpoint verify after it (#6242).
#[tokio::test]
async fn a_run_folds_across_the_windows_inside_it() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let (log, _key) = signed_log(&pool);
    let store = log.store();
    for window in 0..3_u32 {
        sealed(
            store,
            vec![
                entry(window * 2 + 1, STAYS, "2029-01-01"),
                entry(window * 2 + 2, STAYS, "2029-01-01"),
            ],
        )
        .await;
    }
    let checkpoint = log.seal().await.expect("seal").expect("a checkpoint");
    expire(store, Log::Local, expiry_run())
        .await
        .expect("expire");
    let folded = fold(store, Log::Local, fold_run()).await.expect("fold");
    assert_eq!(
        folded,
        Folded {
            tombstones: 6,
            gaps: 3
        },
        "three windows fold in one run"
    );
    assert_eq!(folds(&pool).await, [(1, 2), (3, 4), (5, 6)]);
    assert_eq!(findings(&pool).await, []);
    let verification = log.verify(Some(checkpoint)).await.expect("verify");
    assert!(verification.intact(), "{verification:?}");
}
