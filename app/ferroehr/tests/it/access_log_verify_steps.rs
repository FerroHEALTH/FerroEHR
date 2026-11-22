// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log's verification walks its sealed windows in bounded steps
//! (#5779, #6242) against a real `PostgreSQL` 19: each step verifies the
//! windows that fit its budget of records, at least one, and moves the
//! cursor, a walk reaches the last sealed window however long the backlog,
//! damage inside the backlog is reported with its position and stops the
//! cursor before its window, and a lag past `max_verify_lag` reads
//! `DEGRADED` on the health surface.
//!
//! No openEHR spec governs audit tamper detection: our own design.

use std::time::Duration;

use ferrohealth_health::document::State;
use ferrohealth_health::registry::HealthIndicator;
use serde_json::json;
use sqlx::{AssertSqlSafe, Connection, PgPool};

use ferroehr::access_log::store::AccessLogStore;
use ferroehr::system_log::chain_check::{ChainCheck, ChainCheckState};
use ferroehr::system_log::store::AuditStore;
use ferroehr::system_log::store::chain_walk::{STEP_FLOOR, StepPace};
use ferroehr::system_log::store::chains::{AuditChain, AuditChainFinding, VerifyScope};
use ferroehr::telemetry::indicators::AuditChainHealth;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;

/// The proposal's example entry with the uid and EHR of number `n`.
fn entry(n: u32) -> AccessLogEntry {
    let mut value = crate::access_log_store::example();
    value["uid"]["value"] = json!(format!("0193f1a2-7c1e-7b4a-9f00-{n:012x}"));
    value["ehr_id"]["value"] = json!(format!("7d44b88c-4199-4bad-97dc-{n:012x}"));
    serde_json::from_value(value).expect("the example entry reads")
}

/// Append entries `numbers` through the store, in their order.
async fn append(store: &AccessLogStore, numbers: std::ops::RangeInclusive<u32>) {
    store
        .append(numbers.map(entry).collect())
        .await
        .expect("the entries append");
}

/// Append entries `numbers` through the store, sealing a window of at most
/// `size` entries after each run of them.
async fn append_sealed(store: &AccessLogStore, numbers: std::ops::RangeInclusive<u32>, size: u32) {
    let (first, last) = (*numbers.start(), *numbers.end());
    let mut from = first;
    while from <= last {
        let to = from.saturating_add(size - 1).min(last);
        append(store, from..=to).await;
        store
            .seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
            .await
            .expect("the window seals");
        from = to + 1;
    }
}

/// The last entry id of the window the verification cursor names, 0 before
/// the first.
async fn cursor(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce((SELECT s.last_id FROM audit.access_log_seal s \
           JOIN audit.access_log_seal_verified v \
             ON v.singleton AND s.window_no = v.through_window), 0)",
    )
    .fetch_one(pool)
    .await
    .expect("the cursor")
}

/// Run `statement` with the table's triggers off, as only its owner can.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn tamper(pool: &PgPool, statement: &str) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    sqlx::query("ALTER TABLE audit.access_log_entry DISABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("disable the triggers");
    sqlx::query(AssertSqlSafe(statement.to_owned()))
        .execute(&mut *tx)
        .await
        .expect("the tampering statement itself must succeed");
    sqlx::query("ALTER TABLE audit.access_log_entry ENABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("enable the triggers");
    tx.commit().await.expect("commit");
}

/// A backlog of 25 entries sealed in windows of four, walked four records a
/// step, reaches the last window in seven steps, each moving the cursor by
/// one window; entries appended and not yet sealed during the walk wait for
/// the next one and read as its lag.
#[tokio::test]
async fn a_walk_reaches_the_head_in_bounded_steps_that_each_move_the_cursor() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let audit = AuditStore::new(pool.clone());
    let log = AccessLogStore::new(pool.clone());
    append_sealed(&log, 1..=25, 4).await;

    let mut walk = audit
        .chain_walk(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
        .await
        .expect("the walk starts");
    assert_eq!(walk.position(), 0);
    let mut reached = vec![cursor(&pool).await];
    while !walk.done() {
        let findings = audit.chain_step(&mut walk, 4).await.expect("the step runs");
        assert!(findings.is_empty(), "{findings:?}");
        reached.push(cursor(&pool).await);
        if reached.len() == 3 {
            append(&log, 26..=28).await;
        }
    }
    assert_eq!(
        reached,
        [0, 4, 8, 12, 16, 20, 24, 25],
        "each step moves the cursor by one window of four, to the last sealed window"
    );

    let lag = audit
        .chain_lag(AuditChain::AccessLog)
        .await
        .expect("the lag");
    assert_eq!(lag.entries, 3, "the entries appended during the walk");
    assert!(lag.oldest.is_some());

    log.seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
        .await
        .expect("the window seals");
    let mut next = audit
        .chain_walk(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
        .await
        .expect("the next walk starts");
    assert_eq!(
        next.position(),
        7,
        "the next walk starts after the verified window"
    );
    while !next.done() {
        assert!(
            audit
                .chain_step(&mut next, 4)
                .await
                .expect("the step runs")
                .is_empty()
        );
    }
    assert_eq!(cursor(&pool).await, 28);
    let lag = audit
        .chain_lag(AuditChain::AccessLog)
        .await
        .expect("the lag");
    assert_eq!((lag.entries, lag.oldest), (0, None));
}

/// Damage inside the backlog is reported at its position, every finding up
/// to the head is reported, and the cursor stops at the last intact step
/// before the first damage.
#[tokio::test]
async fn damage_inside_the_backlog_is_reported_and_holds_the_cursor() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let audit = AuditStore::new(pool.clone());
    let log = AccessLogStore::new(pool.clone());
    append_sealed(&log, 1..=8, 4).await;
    assert!(
        audit
            .verify_chain_advancing(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
            .await
            .expect("verify")
            .is_empty()
    );
    assert_eq!(cursor(&pool).await, 8);

    append_sealed(&log, 9..=30, 4).await;
    for id in [15, 27] {
        tamper(
            &pool,
            &format!(
                "UPDATE audit.access_log_entry \
                 SET content = replace(content, '\"was_delivered\":true', '\"was_delivered\":false') \
                 WHERE id = {id}"
            ),
        )
        .await;
    }

    let mut walk = audit
        .chain_walk(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
        .await
        .expect("the walk starts");
    let mut found = Vec::new();
    while !walk.done() {
        found.extend(audit.chain_step(&mut walk, 4).await.expect("the step runs"));
    }
    let positions: Vec<Option<i64>> = found.iter().map(|finding| finding.chain_seq).collect();
    assert_eq!(
        positions,
        [Some(15), Some(27)],
        "each modified entry is reported at its position: {found:?}"
    );
    assert!(
        found
            .iter()
            .all(|finding| finding.finding.contains("modified"))
    );
    assert_eq!(
        cursor(&pool).await,
        12,
        "the cursor stops at the end of the last intact window before the damage"
    );
}

/// The scheduled check walks a backlog to the head and reads intact, and a
/// lag whose oldest entry is older than `max_verify_lag` reads `DEGRADED`
/// until a run reaches the head.
#[tokio::test]
async fn a_lag_past_its_bound_reads_degraded_until_a_run_catches_up() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let audit = AuditStore::new(pool.clone());
    let log = AccessLogStore::new(pool.clone());
    append(&log, 1..=5).await;

    let check = ChainCheck::new(Some(Duration::from_secs(1)));
    let indicator = AuditChainHealth::new(check.clone());
    check
        .refresh_lag(&audit, AuditChain::AccessLog)
        .await
        .expect("the lag reads");
    assert_eq!(
        check.lag(AuditChain::AccessLog).map(|lag| lag.entries),
        Some(5)
    );
    let health = indicator.check().await;
    assert_eq!(
        health.state(),
        State::Degraded,
        "the example entries were recorded long before the bound"
    );
    assert!(
        health.detail().is_some_and(
            |detail| detail.contains("access_log chain's verification is 5 records behind")
        ),
        "{:?}",
        health.detail()
    );

    log.seal_committed(&ferroehr::access_log::seal::ACCESS_LOG)
        .await
        .expect("the window seals");
    assert!(matches!(
        check.run_scope(&audit, VerifyScope::SinceCheckpoint).await,
        ChainCheckState::Intact { .. }
    ));
    assert_eq!(
        check.lag(AuditChain::AccessLog).map(|lag| lag.entries),
        Some(0)
    );
    assert_eq!(indicator.check().await.state(), State::Up);
    assert_eq!(cursor(&pool).await, 5);
}

/// The system records' windows walk in the same bounded steps (one window
/// per record [`crate::audit_chain::seed`] writes): each moves its cursor,
/// and a modified record inside the backlog is reported at its position and
/// holds the cursor before it.
#[tokio::test]
async fn the_system_chain_walks_in_bounded_steps() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let audit = AuditStore::new(pool.clone());
    crate::audit_chain::seed(&audit, 7).await;

    let checkpoint = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT coalesce((SELECT through_window FROM audit.audit_event_seal_verified), 0)",
        )
        .fetch_one(&pool)
        .await
        .expect("the system cursor")
    };
    let mut walk = audit
        .chain_walk(AuditChain::System, VerifyScope::SinceCheckpoint)
        .await
        .expect("the walk starts");
    let mut reached = vec![checkpoint().await];
    while !walk.done() {
        assert!(
            audit
                .chain_step(&mut walk, 2)
                .await
                .expect("the step runs")
                .is_empty()
        );
        reached.push(checkpoint().await);
    }
    assert_eq!(reached, [0, 2, 4, 6, 7]);

    crate::audit_chain::seed(&audit, 6).await;
    crate::audit_chain::with_triggers_disabled(
        &pool,
        "UPDATE audit.audit_event SET principal = 'nobody' WHERE seq = 10",
    )
    .await;
    let mut walk = audit
        .chain_walk(AuditChain::System, VerifyScope::SinceCheckpoint)
        .await
        .expect("the walk starts");
    let mut found = Vec::new();
    while !walk.done() {
        found.extend(audit.chain_step(&mut walk, 2).await.expect("the step runs"));
    }
    let positions: Vec<Option<i64>> = found.iter().map(|finding| finding.chain_seq).collect();
    assert_eq!(positions, [Some(10)], "{found:?}");
    assert_eq!(
        checkpoint().await,
        9,
        "the cursor stops at the end of the last intact step"
    );
    let lag = audit.chain_lag(AuditChain::System).await.expect("the lag");
    assert_eq!(lag.entries, 4);
}

/// The scheduled check walks a backlog of many steps to the head, pausing
/// between them and adapting their size to their duration (#5804).
#[tokio::test]
async fn the_scheduled_check_paces_its_steps_to_the_head() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let audit = AuditStore::new(pool.clone());
    let log = AccessLogStore::new(pool.clone());
    append_sealed(&log, 1..=40, 4).await;

    let check = ChainCheck::new(None).with_first_step(StepPace::starting_at(3));
    assert!(matches!(
        check.run_scope(&audit, VerifyScope::SinceCheckpoint).await,
        ChainCheckState::Intact { .. }
    ));
    assert_eq!(cursor(&pool).await, 40);
    assert_eq!(
        check.lag(AuditChain::AccessLog).map(|lag| lag.entries),
        Some(0)
    );
}

/// Modify the access-log entry `id` with the table's triggers off, as only
/// its owner can.
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads the database on a raw pool connection, outside any request"
)]
async fn tamper_at(pool: &PgPool, id: i64) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    sqlx::query("ALTER TABLE audit.access_log_entry DISABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("disable the triggers");
    let changed = sqlx::query(
        "UPDATE audit.access_log_entry \
         SET content = replace(content, '\"was_delivered\":true', '\"was_delivered\":false') \
         WHERE id = $1",
    )
    .bind(id)
    .execute(&mut *tx)
    .await
    .expect("the tampering statement itself must succeed");
    assert_eq!(changed.rows_affected(), 1, "the entry {id} was changed");
    sqlx::query("ALTER TABLE audit.access_log_entry ENABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("enable the triggers");
    tx.commit().await.expect("commit");
}

/// A catch-up over a backlog of many steps verifies every entry to the head
/// in paced steps and keeps the pace it measured for the next walk; damage
/// deep in the next backlog is reported exactly as one window over the
/// whole backlog reports it, and holds the cursor before it (#5804).
#[tokio::test]
async fn a_large_catch_up_walks_paced_steps_and_reports_damage_exactly() {
    const BACKLOG: u32 = 6_000;
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let audit = AuditStore::new(pool.clone());
    let log = AccessLogStore::new(pool.clone());
    for first in (1..=BACKLOG).step_by(2_000) {
        append_sealed(&log, first..=first + 1_999, 2_000).await;
    }

    let first = StepPace::starting_at(STEP_FLOOR);
    let check = ChainCheck::new(None).with_first_step(first);
    assert!(matches!(
        check.run_scope(&audit, VerifyScope::SinceCheckpoint).await,
        ChainCheckState::Intact { .. }
    ));
    assert_eq!(
        cursor(&pool).await,
        i64::from(BACKLOG),
        "every entry verified"
    );
    assert_eq!(
        check.lag(AuditChain::AccessLog).map(|lag| lag.entries),
        Some(0)
    );
    let kept = check.pace(AuditChain::AccessLog);
    assert_ne!(kept, first, "the walk keeps the pace it measured");
    assert!(kept.pause() > Duration::ZERO);

    for first in (BACKLOG + 1..=2 * BACKLOG).step_by(2_000) {
        append_sealed(&log, first..=first + 1_999, 2_000).await;
    }
    let damaged = [i64::from(BACKLOG) + 1_501, i64::from(BACKLOG) + 4_750];
    for id in damaged {
        tamper_at(&pool, id).await;
    }
    let mut window = audit
        .chain_walk(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
        .await
        .expect("the walk starts");
    let whole = audit
        .chain_step(&mut window, i32::MAX)
        .await
        .expect("one window over the whole backlog");
    assert!(window.done());
    let ChainCheckState::Damaged {
        total, findings, ..
    } = check.run_scope(&audit, VerifyScope::SinceCheckpoint).await
    else {
        panic!("the damage is reported");
    };
    assert_eq!(total, 2);
    let reported = |found: &[AuditChainFinding]| {
        found
            .iter()
            .map(|finding| (finding.chain_seq, finding.finding.clone()))
            .collect::<Vec<_>>()
    };
    assert_eq!(reported(&findings), reported(&whole));
    assert_eq!(
        findings
            .iter()
            .map(|finding| finding.chain_seq)
            .collect::<Vec<_>>(),
        damaged.map(Some)
    );
    let held = cursor(&pool).await;
    assert!(
        (i64::from(BACKLOG)..damaged[0]).contains(&held),
        "the cursor stops before the first damage, at {held}"
    );
}
