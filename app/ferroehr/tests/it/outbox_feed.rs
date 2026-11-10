// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outbox feed against a real `PostgreSQL` 18: the reader registry the
//! boot reconciles from `[record.change_feed] readers`, the cursor a reader moves by
//! acknowledging, and the retention prune that every active cursor holds back
//! (#4786, #4766, #3330).
//!
//! No openEHR spec governs eventing: our own design/extension.

use std::time::Duration;

use ferroehr::extensions::outbox::{
    Acknowledged, OutboxDomain, Reconciled, acknowledge, prune, read, reconcile,
};
use ferroehr::service::FerroEhrService;
use sqlx::{Connection as _, PgPool};

/// The retention window the prune cases use.
const SEVEN_DAYS: Duration = Duration::from_hours(7 * 24);

/// The reader most of these tests register.
const READER: &str = "omop";

fn names(readers: &[&str]) -> Vec<String> {
    readers.iter().map(|&reader| reader.to_owned()).collect()
}

async fn seqs(pool: &PgPool) -> Vec<i64> {
    sqlx::query_scalar("SELECT seq FROM clinical.event_outbox ORDER BY seq")
        .fetch_all(pool)
        .await
        .expect("read outbox seqs")
}

async fn registry(pool: &PgPool) -> Vec<(String, i64, bool)> {
    sqlx::query_as(
        "SELECT reader, last_seq, active FROM clinical.event_outbox_reader ORDER BY reader",
    )
    .fetch_all(pool)
    .await
    .expect("read the registry")
}

/// Five EHR creations, one outbox row each, aged well past the window.
async fn seed_aged_rows(pool: &PgPool) -> Vec<i64> {
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool));
    for _ in 0..5 {
        crate::fixtures::create_ehr(&svc).await;
    }
    sqlx::query("UPDATE clinical.event_outbox SET committed_at = now() - interval '30 days'")
        .execute(pool)
        .await
        .expect("back-date the commit instants");
    let seqs = seqs(pool).await;
    assert_eq!(seqs.len(), 5, "one outbox row per EHR creation");
    seqs
}

#[tokio::test]
async fn boot_registers_the_named_readers_and_deactivates_the_rest() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let first = reconcile(&pool, &names(&["ferrobridge", READER]))
        .await
        .expect("first boot");
    assert_eq!(
        first,
        Reconciled {
            registered: names(&["ferrobridge", READER]),
            deactivated: Vec::new(),
        }
    );
    let seeded = seed_aged_rows(&pool).await;
    assert_eq!(
        acknowledge(&pool, READER, seeded[1]).await.expect("ack"),
        Acknowledged::Moved
    );

    // The configuration drops `omop`: the next boot marks it inactive and
    // keeps its cursor.
    let second = reconcile(&pool, &names(&["ferrobridge"]))
        .await
        .expect("second boot");
    assert_eq!(
        second,
        Reconciled {
            registered: Vec::new(),
            deactivated: names(&[READER]),
        }
    );
    assert_eq!(
        registry(&pool).await,
        [
            ("ferrobridge".to_owned(), 0, true),
            (READER.to_owned(), seeded[1], false),
        ]
    );
    assert_eq!(
        read(&pool, OutboxDomain::Clinical, READER, 100)
            .await
            .expect("read"),
        None,
        "an inactive reader has no feed"
    );

    // Named again, it resumes where it left off.
    let third = reconcile(&pool, &names(&["ferrobridge", READER]))
        .await
        .expect("third boot");
    assert_eq!(third.registered, names(&[READER]));
    let resumed = read(&pool, OutboxDomain::Clinical, READER, 100)
        .await
        .expect("read")
        .expect("an active reader");
    assert_eq!(
        resumed.iter().map(|event| event.seq).collect::<Vec<_>>(),
        seeded[2..],
        "the feed starts after the kept cursor"
    );

    // A boot with no reader deactivates both.
    let off = reconcile(&pool, &[]).await.expect("boot with no reader");
    assert_eq!(off.deactivated, names(&["ferrobridge", READER]));
}

#[tokio::test]
async fn prune_stops_at_the_lowest_active_reader_cursor() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    reconcile(&pool, &names(&[READER])).await.expect("register");
    let seeded = seed_aged_rows(&pool).await;

    // The reader has acknowledged the first two rows only.
    acknowledge(&pool, READER, seeded[1])
        .await
        .expect("acknowledge");
    let pruned = prune(&pool, SEVEN_DAYS).await.expect("prune");
    assert_eq!(pruned, 2, "only the rows at or below the cursor are pruned");
    assert_eq!(
        seqs(&pool).await,
        seeded[2..],
        "every row the reader has not acknowledged survives the window"
    );

    // The reader catches up: the rest goes on the next pass.
    acknowledge(&pool, READER, seeded[4])
        .await
        .expect("acknowledge the tail");
    assert_eq!(prune(&pool, SEVEN_DAYS).await.expect("prune"), 3);
    assert_eq!(seqs(&pool).await, Vec::<i64>::new());
}

#[tokio::test]
async fn a_reader_the_configuration_dropped_holds_no_floor() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    reconcile(&pool, &names(&[READER])).await.expect("register");
    let seeded = seed_aged_rows(&pool).await;
    acknowledge(&pool, READER, seeded[0])
        .await
        .expect("acknowledge");

    reconcile(&pool, &[]).await.expect("the reader is dropped");
    assert_eq!(prune(&pool, SEVEN_DAYS).await.expect("prune"), 5);
    assert_eq!(seqs(&pool).await, Vec::<i64>::new());
}

#[tokio::test]
async fn a_row_inside_the_window_stays_whatever_the_cursor() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    reconcile(&pool, &names(&[READER])).await.expect("register");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    crate::fixtures::create_ehr(&svc).await;
    let fresh = seqs(&pool).await;
    acknowledge(&pool, READER, fresh[0])
        .await
        .expect("acknowledge");
    assert_eq!(prune(&pool, SEVEN_DAYS).await.expect("prune"), 0);
    assert_eq!(seqs(&pool).await, fresh);
}

#[tokio::test]
async fn an_acknowledgement_moves_forward_and_never_past_the_tail() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    reconcile(&pool, &names(&[READER])).await.expect("register");
    let seeded = seed_aged_rows(&pool).await;

    assert_eq!(
        acknowledge(&pool, READER, seeded[2]).await.expect("ack"),
        Acknowledged::Moved
    );
    assert_eq!(
        acknowledge(&pool, READER, seeded[2]).await.expect("again"),
        Acknowledged::Moved,
        "acknowledging the cursor again changes nothing"
    );
    assert_eq!(
        acknowledge(&pool, READER, seeded[1]).await.expect("back"),
        Acknowledged::Behind { cursor: seeded[2] }
    );
    assert_eq!(
        acknowledge(&pool, READER, seeded[4] + 1)
            .await
            .expect("past"),
        Acknowledged::PastTail { tail: seeded[4] }
    );
    assert_eq!(
        acknowledge(&pool, "nobody", seeded[0])
            .await
            .expect("unknown"),
        Acknowledged::Unknown
    );
    assert_eq!(
        registry(&pool).await,
        [(READER.to_owned(), seeded[2], true)]
    );
}

#[tokio::test]
async fn a_read_waits_for_a_commit_that_drew_a_lower_seq() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    reconcile(&pool, &names(&[READER])).await.expect("register");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let contribution: uuid::Uuid =
        sqlx::query_scalar("SELECT contribution_id FROM clinical.event_outbox")
            .fetch_one(&pool)
            .await
            .expect("the seeded contribution");
    let tail = seqs(&pool).await[0];
    acknowledge(&pool, READER, tail)
        .await
        .expect("start at the tail");

    // A slow commit draws the next seq and stays open; a quick one draws the
    // seq after it and commits first.
    let mut slow = pool.acquire().await.expect("slow connection");
    let mut slow_tx = slow.begin().await.expect("begin the slow commit");
    ferroehr::storage::version_repo::commit::write_outbox(
        &mut slow_tx,
        contribution,
        Some(ehr),
        jiff::Timestamp::now(),
        Vec::new(),
    )
    .await
    .expect("the slow insert");
    let mut quick = pool.acquire().await.expect("quick connection");
    let mut quick_tx = quick.begin().await.expect("begin the quick commit");
    ferroehr::storage::version_repo::commit::write_outbox(
        &mut quick_tx,
        contribution,
        Some(ehr),
        jiff::Timestamp::now(),
        Vec::new(),
    )
    .await
    .expect("the quick insert");
    quick_tx.commit().await.expect("the quick commit");

    // The read cannot serve the quick row before the slow one ends.
    let reader_pool = pool.clone();
    let pending = tokio::spawn(async move {
        read(&reader_pool, OutboxDomain::Clinical, READER, 100)
            .await
            .expect("read")
            .expect("an active reader")
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !pending.is_finished(),
        "a read waits while a commit holds a lower seq"
    );
    slow_tx.commit().await.expect("the slow commit");
    let served = pending.await.expect("the read task");
    let served_seqs: Vec<i64> = served.iter().map(|event| event.seq).collect();
    assert_eq!(
        served_seqs.len(),
        2,
        "both rows are served: {served_seqs:?}"
    );
    assert!(
        served_seqs[0] < served_seqs[1],
        "in seq order: {served_seqs:?}"
    );
}
