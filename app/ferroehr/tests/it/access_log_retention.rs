// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Retention per access-log entry (#5128) against a real `PostgreSQL` 18: an
//! entry whose `retain_until` has passed is replaced by its tombstone in
//! place, in any order of expiry, the chain verifies after it, and a
//! tombstone never becomes an entry again (`docs/plans/access-log.md`
//! §Retention, §Integrity).
//!
//! No openEHR spec governs audit retention: our own design.

use jiff::Timestamp;
use serde_json::json;
use sqlx::PgPool;

use ferroehr::access_log::expiry::expire;
use ferroehr::access_log::store::{AccessLogStore, canonical_form};
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;

/// The proposal's example entry with the uid and EHR of number `n`, kept to
/// the end of `retain_until`.
fn entry(n: u32, retain_until: &str) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(uid(n));
    value["ehr_id"]["value"] = json!(format!("7d44b88c-4199-4bad-97dc-{n:012x}"));
    value["retention"]["retain_until"]["value"] = json!(retain_until);
    serde_json::from_value(value).expect("the example entry reads")
}

/// The uid of entry `n`.
fn uid(n: u32) -> String {
    format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}")
}

/// Noon on 1 June 2030, UTC: the day the retention runs in these tests.
fn now() -> Timestamp {
    "2030-06-01T12:00:00Z".parse().expect("an instant")
}

/// Five entries: the middle one kept until 2029, the second to the day the
/// tests run, the first and fourth longer, and the last without end.
async fn log(pool: &PgPool) -> (AccessLogStore, Vec<AccessLogEntry>) {
    let store = AccessLogStore::new(pool.clone());
    let mut entries = vec![
        entry(1, "2031-11-07"),
        entry(2, "2030-06-01"),
        entry(3, "2029-11-07"),
        entry(4, "2036-11-07"),
        entry(5, "2031-11-07"),
    ];
    let mut kept = serde_json::to_value(&entries[4]).expect("the entry writes");
    kept["retention"]
        .as_object_mut()
        .expect("an object")
        .remove("retain_until");
    entries[4] = serde_json::from_value(kept).expect("an entry kept without end reads");
    let entries = store.append(entries).await.expect("the entries append");
    (store, entries)
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

/// An entry whose last day has passed is replaced by its tombstone, which
/// keeps its uid, `system_id`, `time_recorded` and link and names the grounds
/// of the period that ended; an entry in its last day, one kept longer and
/// one kept without end stay entries.
#[tokio::test]
async fn an_expired_entry_becomes_a_tombstone() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let (store, entries) = log(&pool).await;

    assert_eq!(expire(&store, now()).await.expect("the retention runs"), 1);

    let expired = &entries[2];
    let Some(AccessLogRecord::AccessLogTombstone(tombstone)) =
        store.read(&uid(3)).await.expect("read")
    else {
        panic!("the expired entry is a tombstone");
    };
    assert_eq!(tombstone.uid, expired.uid);
    assert_eq!(tombstone.system_id, expired.system_id);
    assert_eq!(tombstone.time_recorded, expired.time_recorded);
    assert_eq!(tombstone.link, expired.link, "the link is kept whole");
    assert_eq!(tombstone.time_removed.value, "2030-06-01T12:00:00Z");
    assert_eq!(
        tombstone.grounds, expired.retention.grounds,
        "the tombstone names the grounds of the period that ended"
    );
    let content: String =
        sqlx::query_scalar("SELECT content FROM audit.access_log_record WHERE uid = $1")
            .bind(uid(3))
            .fetch_one(&pool)
            .await
            .expect("the stored record");
    assert!(
        !content.contains("ehr_id") && !content.contains("accessor"),
        "nothing of the entry's content is left: {content}"
    );

    for n in [1, 2, 4, 5] {
        assert!(
            matches!(
                store.read(&uid(n)).await.expect("read"),
                Some(AccessLogRecord::AccessLogEntry(_))
            ),
            "entry {n} is within its retention"
        );
    }
}

/// The chain verifies after an entry in the middle of the log is replaced,
/// and again after the entries before it, in an order other than the chain's.
#[tokio::test]
async fn the_chain_still_verifies_after_tombstoning() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let (store, _) = log(&pool).await;
    assert_eq!(findings(&pool).await, [], "the log verifies before");

    assert_eq!(expire(&store, now()).await.expect("the middle expires"), 1);
    assert_eq!(
        findings(&pool).await,
        [],
        "the log verifies with a tombstone in its middle"
    );

    let later = "2032-01-01T00:00:00Z".parse().expect("an instant");
    assert_eq!(
        expire(&store, later)
            .await
            .expect("the first and second expire"),
        2
    );
    assert_eq!(
        findings(&pool).await,
        [],
        "the log verifies with entries expired out of order"
    );
}

/// A tombstone is never replaced again and never becomes an entry: a later
/// run leaves it as it is, the store refuses to replace it, the table refuses
/// the entry's content back, and its uid cannot be written again.
#[tokio::test]
async fn a_tombstone_never_comes_back() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let (store, entries) = log(&pool).await;
    assert_eq!(expire(&store, now()).await.expect("the retention runs"), 1);
    let before = store.read(&uid(3)).await.expect("read");

    assert_eq!(
        expire(&store, now())
            .await
            .expect("the retention runs again"),
        0,
        "a second run finds nothing to replace"
    );
    let grounds = entries[2].retention.grounds.clone();
    let removed = serde_json::from_value(json!({ "_type": "DV_DATE_TIME",
        "value": "2030-06-02T00:00:00Z" }))
    .expect("the time reads");
    assert!(
        store
            .tombstone(&uid(3), removed, grounds)
            .await
            .expect("the store answers")
            .is_none(),
        "a tombstone is not replaced again"
    );

    let restored = sqlx::query("UPDATE audit.access_log_record SET content = $2 WHERE uid = $1")
        .bind(uid(3))
        .bind(canonical_form(&entries[2]).expect("the entry's canonical form"))
        .execute(&pool)
        .await;
    assert!(restored.is_err(), "the table refuses the entry back");
    assert!(
        store.append(vec![entries[2].clone()]).await.is_err(),
        "the uid is not written again"
    );

    assert_eq!(store.read(&uid(3)).await.expect("read"), before);
    assert_eq!(findings(&pool).await, []);
}
