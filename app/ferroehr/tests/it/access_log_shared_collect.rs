// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The collection of shared parts (#5968): a shared part carries the latest
//! `retain_until` of the entries naming it, and the retention sweep removes
//! the parts past it that no entry names, in bounded batches. A part an
//! entry still names is never removed, and an orphan goes within a few
//! passes. No openEHR spec governs audit storage: our own design.

use jiff::Timestamp;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;
use serde_json::json;
use sqlx::{Connection, PgPool};

use ferroehr::access_log::store::AccessLogStore;

/// The example entry with the uid and EHR of number `n`, kept until
/// `retain_until`.
fn entry(n: u32, retain_until: &str) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["ehr_id"]["value"] = json!(format!("7d44b88c-4199-4bad-97dc-{n:012x}"));
    value["retention"]["retain_until"]["value"] = json!(retain_until);
    serde_json::from_value(value).expect("the example entry reads")
}

/// The grounds of a tombstone.
fn expiry() -> NonEmptyVec<DvCodedText> {
    let ground: DvCodedText = serde_json::from_value(json!({
        "_type": "DV_CODED_TEXT", "value": "retention period ended",
        "defining_code": { "terminology_id": { "value": "local" }, "code_string": "expired" }
    }))
    .expect("the ground reads");
    NonEmptyVec::new(vec![ground]).expect("one ground")
}

/// A date and time as the RM writes it.
fn at(value: &str) -> DvDateTime {
    serde_json::from_value(json!({ "_type": "DV_DATE_TIME", "value": value }))
        .expect("the time reads")
}

/// The shared parts held.
async fn parts(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit.access_log_shared")
        .fetch_one(pool)
        .await
        .expect("the shared parts")
}

/// Moves every shared part's `retain_until` to `date`, as the passing of
/// time would: the table refuses to lower it, so only its owner can.
#[expect(
    clippy::disallowed_methods,
    reason = "a test changes the stored parts on a raw pool connection, outside any request"
)]
async fn age_parts(pool: &PgPool, date: &str) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    for statement in [
        "ALTER TABLE audit.access_log_shared DISABLE TRIGGER USER".to_owned(),
        format!("UPDATE audit.access_log_shared SET retain_until = '{date}'"),
        "ALTER TABLE audit.access_log_shared ENABLE TRIGGER USER".to_owned(),
    ] {
        sqlx::query(sqlx::AssertSqlSafe(statement))
            .execute(&mut *tx)
            .await
            .expect("the parts age");
    }
    tx.commit().await.expect("commit");
}

/// The date the sweep runs on: today.
fn today() -> jiff::civil::Date {
    Timestamp::now().to_zoned(jiff::tz::TimeZone::UTC).date()
}

/// A part keeps the `retain_until` of the entries that name it, which its
/// retention member states, and never one before the day the intake named
/// it; nothing shortens it.
#[tokio::test]
async fn a_shared_part_is_kept_as_long_as_its_entries() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    store
        .append(vec![
            entry(1, "2031-11-07"),
            entry(2, "2033-01-01"),
            entry(3, "2033-01-01"),
        ])
        .await
        .expect("the entries append");
    let kept: Vec<String> = sqlx::query_scalar(
        "SELECT retain_until::text FROM audit.access_log_shared ORDER BY retain_until",
    )
    .fetch_all(&pool)
    .await
    .expect("the parts");
    assert_eq!(kept, ["2031-11-07", "2033-01-01"], "one part per retention");
    store
        .append(vec![entry(4, "2020-01-01")])
        .await
        .expect("an entry kept to a past date");
    let raised: bool = sqlx::query_scalar(
        "SELECT bool_and(retain_until >= current_date) FROM audit.access_log_shared",
    )
    .fetch_one(&pool)
    .await
    .expect("the parts");
    assert!(
        raised,
        "a part the intake names today is kept today at least"
    );
    let lowered = sqlx::query("UPDATE audit.access_log_shared SET retain_until = retain_until - 1")
        .execute(&pool)
        .await
        .expect_err("a part's retention is never shortened");
    assert!(lowered.to_string().contains("never rewritten"), "{lowered}");
}

/// A part an entry still names is never collected, past its retention or
/// not; once the last entry naming it is a tombstone, the sweep removes it,
/// and orphaned parts go within a pass per batch.
#[tokio::test]
async fn the_sweep_collects_a_shared_part_once_no_entry_names_it() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let written = store
        .append(vec![entry(1, "2025-01-01"), entry(2, "2025-01-01")])
        .await
        .expect("the entries append");
    store
        .settle(ferroehr::access_log::chainer::Owed::default())
        .await
        .expect("the intake chains");
    age_parts(&pool, "2025-01-02").await;
    for _ in 0..3 {
        assert_eq!(
            store.collect_shared(today(), 1).await.expect("a pass"),
            0,
            "both entries name the part"
        );
    }
    let (first, second) = (&written[0], &written[1]);
    store
        .tombstone(first.uid.value(), at("2026-01-01T00:00:00Z"), expiry())
        .await
        .expect("the tombstone")
        .expect("the entry");
    assert_eq!(store.collect_shared(today(), 1).await.expect("a pass"), 0);
    assert_eq!(
        parts(&pool).await,
        1,
        "the other entry still names the part"
    );
    assert_eq!(
        store.read(second.uid.value()).await.expect("read"),
        Some(AccessLogRecord::AccessLogEntry(second.clone())),
        "the other entry reads back with its shared part"
    );
    store
        .tombstone(second.uid.value(), at("2026-01-01T00:00:00Z"), expiry())
        .await
        .expect("the tombstone")
        .expect("the entry");
    assert_eq!(store.collect_shared(today(), 1).await.expect("a pass"), 1);
    assert_eq!(parts(&pool).await, 0, "no entry names the part any more");

    sqlx::query(
        "INSERT INTO audit.access_log_shared (digest, content, retain_until) \
         SELECT sha256(convert_to(c, 'UTF8')), c, DATE '2025-01-01' \
           FROM (SELECT format('{\"n\":%s}', n) AS c FROM generate_series(1, 5) AS n) AS t",
    )
    .execute(&pool)
    .await
    .expect("five orphaned parts");
    let mut passes = 0;
    while store.collect_shared(today(), 2).await.expect("a pass") > 0 {
        passes += 1;
    }
    assert_eq!(passes, 3, "two parts a pass, five parts");
    assert_eq!(parts(&pool).await, 0, "every orphan is collected");
    let findings: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.verify_access_log_chain()")
        .fetch_one(&pool)
        .await
        .expect("the verifier");
    assert_eq!(findings, 0, "the chain verifies");
}

/// A part an intake write in flight names is skipped by the sweep, and once
/// the write commits the part is kept through the day the intake named it
/// (#5887).
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test holds an intake write open on a raw pool connection, outside any request"
)]
async fn the_sweep_skips_a_shared_part_an_intake_write_names() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let store = AccessLogStore::new(pool.clone());
    let written = store
        .append(vec![entry(1, "2025-01-01")])
        .await
        .expect("the entry appends");
    store
        .settle(ferroehr::access_log::chainer::Owed::default())
        .await
        .expect("the intake chains");
    store
        .tombstone(written[0].uid.value(), at("2026-01-01T00:00:00Z"), expiry())
        .await
        .expect("the tombstone")
        .expect("the entry");
    age_parts(&pool, "2025-01-02").await;

    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    let mut second = entry(2, "2025-01-01");
    let intake =
        ferroehr::access_log::intake::IntakeEntry::of(&mut second, None).expect("the intake form");
    ferroehr::access_log::intake::take_in(&mut tx, &[intake], &[Vec::new()], None)
        .await
        .expect("the intake write");
    assert_eq!(
        store.collect_shared(today(), 10).await.expect("a pass"),
        0,
        "the part the write in flight names is skipped"
    );
    tx.commit().await.expect("the intake write commits");
    assert_eq!(
        store.collect_shared(today(), 10).await.expect("a pass"),
        0,
        "the part is kept through today"
    );
    store
        .settle(ferroehr::access_log::chainer::Owed::default())
        .await
        .expect("the intake chains");
    assert!(
        matches!(
            store.read(second.uid.value()).await.expect("read"),
            Some(AccessLogRecord::AccessLogEntry(_))
        ),
        "the entry written beside the sweep reads back with its shared part"
    );
}
