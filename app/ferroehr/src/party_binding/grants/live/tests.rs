// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A grant holds within its dates and instants under the ceiling, a change
//! is tracked inside its request only, queued stale refreshes run one load
//! (#6343), and the facts copy finds every stamp its snapshot did not see,
//! whatever order the stamping transactions commit in, with no notification
//! sent (#6074).

use super::*;
use crate::party_binding::grants::store::{Dates, Term};
use sqlx::PgPool;

fn date(text: &str) -> jiff::civil::Date {
    text.parse().expect("a date")
}

fn row(code: &str) -> CapabilityRow {
    CapabilityRow::new(
        Uuid::nil(),
        Uuid::nil(),
        code.to_owned(),
        Term {
            lifecycle_state: "532".to_owned(),
            role_dates: Some(Dates {
                lower: None,
                upper: None,
            }),
            dates: Some(Dates {
                lower: Some(date("2026-12-01")),
                upper: Some(date("2026-12-03")),
            }),
            not_before: None,
            not_after: None,
        },
        None,
    )
}

#[test]
fn a_grant_holds_within_its_dates_and_instants_under_the_ceiling() {
    let ceiling = GrantCeiling::new(Operation::ALL);
    let now: jiff::Timestamp = "2026-12-02T12:00:00Z".parse().expect("an instant");
    let today = date("2026-12-02");
    let relay = row("commit_composition");
    assert_eq!(
        in_force(&relay, &ceiling, today, now),
        Some(Operation::CommitComposition)
    );
    assert_eq!(
        in_force(&relay, &ceiling, date("2026-12-03"), now),
        None,
        "the upper bound is exclusive"
    );
    assert_eq!(
        in_force(&relay, &GrantCeiling::new([]), today, now),
        None,
        "an operation the ceiling does not admit is never honoured"
    );
    let mut timed = relay.clone();
    timed.term.not_before = Some("2026-12-02T13:00:00Z".parse().expect("an instant"));
    assert_eq!(in_force(&timed, &ceiling, today, now), None);
    timed.term.not_before = None;
    timed.term.not_after = Some("2026-12-02T11:00:00Z".parse().expect("an instant"));
    assert_eq!(in_force(&timed, &ceiling, today, now), None);
    let mut empty = relay.clone();
    empty.term.role_dates = None;
    assert_eq!(in_force(&empty, &ceiling, today, now), None);
    let mut inactive = relay;
    inactive.term.lifecycle_state = "800".to_owned();
    assert_eq!(in_force(&inactive, &ceiling, today, now), None);
    assert_eq!(
        in_force(&row("import_everything"), &ceiling, today, now),
        None
    );
}

/// Refreshes that all found the copy stale and queued on `installing` run
/// one load between them: each after the first finds the revision it
/// read installed (#6343).
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_stale_refreshes_make_one_load() {
    let db = testkit::db().await.expect("testkit database");
    let pool = crate::db::domain_pool_from(&db.pool(), Domain::Party);
    let live = Arc::new(LiveGrants::new(pool));
    let gate = live.installing.lock().await;
    let refreshes: Vec<_> = (0..6)
        .map(|_| {
            let live = Arc::clone(&live);
            tokio::spawn(async move { live.refresh().await })
        })
        .collect();
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    drop(gate);
    for refresh in refreshes {
        refresh
            .await
            .expect("the refresh ends")
            .expect("the store answers");
    }
    assert_eq!(live.loads.load(Ordering::Acquire), 1, "one load");
    assert!(live.revision().is_some(), "the revision is installed");
}

#[tokio::test]
async fn a_change_is_tracked_inside_the_request_only() {
    let ((), changed) = tracking(async { note_change() }).await;
    assert!(changed);
    let ((), changed) = tracking(async {}).await;
    assert!(!changed);
    note_change();
}

/// A membership of `party` in a team of its own, stamped as a commit stamps
/// it, on the open transaction `tx`.
async fn stamp_membership(tx: &mut sqlx::PgConnection, party: Uuid) {
    sqlx::query(
        "INSERT INTO binding_caller_fact (holder_id, ordinal, kind, party_id, target_id, \
         lifecycle_state, role_validity, validity) \
         VALUES ($1, 0, 'MBR', $1, $2, '532', '(,)', '(,)')",
    )
    .bind(party)
    .bind(Uuid::now_v7())
    .execute(&mut *tx)
    .await
    .expect("the fact");
    caller::store::finish(tx, vec![party])
        .await
        .expect("the stamp");
}

/// A copy over the party pool of a fresh store, read once.
async fn read_copy() -> (testkit::TestDb, PgPool, Arc<LiveGrants>) {
    let db = testkit::db().await.expect("testkit database");
    let pool = crate::db::domain_pool_from(&db.pool(), Domain::Party);
    let live = Arc::new(LiveGrants::new(pool.clone()));
    live.refresh().await.expect("the first read");
    (db, pool, live)
}

/// Whether the store's revisions, read against the snapshot the copy holds,
/// find a stamp it did not see.
async fn behind(pool: &PgPool, live: &LiveGrants) -> bool {
    let mut conn = acquire(pool).await.expect("a connection");
    let read = store::revisions(&mut conn, live.held_snapshot().as_ref())
        .await
        .expect("the revisions");
    !live.holds(&read)
}

/// A copy read before a stamp commits finds itself behind
/// (`party.facts_behind`) until it reads again, and then holds the stamped
/// party's facts; the stamp sent no notification on the grants channel.
#[tokio::test]
async fn an_older_snapshot_is_behind_until_the_copy_reads_again() {
    let (_db, pool, live) = read_copy().await;
    assert!(!behind(&pool, &live).await, "a copy just read is current");
    let mut listener = sqlx::postgres::PgListener::connect_with(&pool)
        .await
        .expect("a listener");
    listener.listen(CHANNEL).await.expect("LISTEN");
    let party = Uuid::now_v7();
    let mut conn = acquire(&pool).await.expect("a connection");
    let mut tx = sqlx::Connection::begin(&mut *conn).await.expect("a commit");
    stamp_membership(&mut tx, party).await;
    // NOTE: our own design (#5762), a test commit of party rows that no request makes.
    tx.commit().await.expect("the commit");
    drop(conn);
    let older = live.facts_snapshot().expect("a snapshot");
    for _ in 0..2 {
        assert!(behind(&pool, &live).await, "the older snapshot is behind");
        assert_eq!(live.facts_snapshot().as_ref(), Some(&older));
    }
    assert!(!live.held_facts().by_party.contains_key(&party));
    live.refresh().await.expect("the read again");
    assert!(
        !behind(&pool, &live).await,
        "the copy read again is current"
    );
    assert!(live.held_facts().by_party.contains_key(&party));
    let notified =
        tokio::time::timeout(std::time::Duration::from_millis(300), listener.try_recv()).await;
    assert!(
        notified.is_err(),
        "a facts stamp notifies no replica: {notified:?}"
    );
}

/// A stamp of a transaction still running when the copy was read is behind
/// that copy once it commits, though its xid is below the stamps the copy
/// already read: B started after A, stamped and committed first, and the copy
/// read between them reads A's stamp next.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_stamp_running_when_the_copy_was_read_is_read_once_it_commits() {
    let (_db, pool, live) = read_copy().await;
    let (first, second) = (Uuid::now_v7(), Uuid::now_v7());
    let mut a = acquire(&pool).await.expect("a connection");
    let mut a_tx = sqlx::Connection::begin(&mut *a).await.expect("A");
    stamp_membership(&mut a_tx, first).await;
    let mut b = acquire(&pool).await.expect("a connection");
    let mut b_tx = sqlx::Connection::begin(&mut *b).await.expect("B");
    stamp_membership(&mut b_tx, second).await;
    // NOTE: our own design (#5762), a test commit of party rows that no request makes.
    b_tx.commit().await.expect("B commits first");
    drop(b);
    live.refresh().await.expect("the read between them");
    assert!(live.held_facts().by_party.contains_key(&second));
    assert!(!live.held_facts().by_party.contains_key(&first));
    assert!(
        !behind(&pool, &live).await,
        "A's stamp is not committed, so the copy holds every one"
    );
    // NOTE: our own design (#5762), a test commit of party rows that no request makes.
    a_tx.commit().await.expect("A commits");
    drop(a);
    assert!(behind(&pool, &live).await, "A's stamp is unseen");
    live.refresh().await.expect("the read after A");
    assert!(
        live.held_facts().by_party.contains_key(&first),
        "the read after A takes its stamp"
    );
    assert!(!behind(&pool, &live).await);
}
