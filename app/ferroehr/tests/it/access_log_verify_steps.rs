// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access log's verification walks its chain in bounded steps (#5779)
//! against a real `PostgreSQL` 18: each step verifies at most its budget of
//! records and moves the cursor, a walk reaches the head it started with
//! however long the backlog, damage inside the backlog is reported with its
//! position and stops the cursor before it, and a lag past
//! `max_verify_lag` reads `DEGRADED` on the health surface.
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
use ferroehr::system_log::store::chains::{AuditChain, VerifyScope};
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

/// The access log's verification cursor, 0 before the first.
async fn cursor(pool: &PgPool) -> i64 {
    sqlx::query_scalar(
        "SELECT coalesce((SELECT through_seq FROM audit.access_log_chain_verified), 0)",
    )
    .fetch_one(pool)
    .await
    .expect("the cursor")
}

/// Run `statement` with the table's triggers off, as only its owner can.
async fn tamper(pool: &PgPool, statement: &str) {
    let mut conn = pool.acquire().await.expect("connection");
    let mut tx = conn.begin().await.expect("begin");
    sqlx::query("ALTER TABLE audit.access_log_record DISABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("disable the triggers");
    sqlx::query(AssertSqlSafe(statement.to_owned()))
        .execute(&mut *tx)
        .await
        .expect("the tampering statement itself must succeed");
    sqlx::query("ALTER TABLE audit.access_log_record ENABLE TRIGGER USER")
        .execute(&mut *tx)
        .await
        .expect("enable the triggers");
    tx.commit().await.expect("commit");
}

/// A backlog of 25 entries walked four records a step reaches the head in
/// seven steps, each moving the cursor by at most four; entries appended
/// during the walk wait for the next one and read as its lag.
#[tokio::test]
async fn a_walk_reaches_the_head_in_bounded_steps_that_each_move_the_cursor() {
    let testdb = testkit::db().await.expect("testkit database");
    let pool = testdb.pool();
    let audit = AuditStore::new(pool.clone());
    let log = AccessLogStore::new(pool.clone());
    append(&log, 1..=25).await;

    let mut walk = audit
        .chain_walk(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
        .await
        .expect("the walk starts");
    assert_eq!(walk.head(), Some(25));
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
        "each step moves the cursor by at most its budget, to the head the walk started with"
    );

    let lag = audit
        .chain_lag(AuditChain::AccessLog)
        .await
        .expect("the lag");
    assert_eq!(lag.entries, 3, "the entries appended during the walk");
    assert!(lag.oldest.is_some());

    let mut next = audit
        .chain_walk(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
        .await
        .expect("the next walk starts");
    assert_eq!(next.position(), 25, "the next walk starts at the cursor");
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
    append(&log, 1..=8).await;
    assert!(
        audit
            .verify_chain_advancing(AuditChain::AccessLog, VerifyScope::SinceCheckpoint)
            .await
            .expect("verify")
            .is_empty()
    );
    assert_eq!(cursor(&pool).await, 8);

    append(&log, 9..=30).await;
    for seq in [15, 27] {
        tamper(
            &pool,
            &format!(
                "UPDATE audit.access_log_record \
                 SET content = replace(content, '\"was_delivered\":true', '\"was_delivered\":false') \
                 WHERE chain_seq = {seq}"
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
        "the cursor stops at the end of the last intact step before the damage"
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

/// The system chain walks in the same bounded steps: each moves its cursor,
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
            "SELECT coalesce((SELECT through_seq FROM audit.audit_chain_checkpoint), 0)",
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
        "UPDATE audit.audit_event SET principal = 'nobody' WHERE chain_seq = 10",
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
    append(&log, 1..=40).await;

    let check = ChainCheck::new(None)
        .with_first_step(ferroehr::system_log::store::chain_walk::StepPace::starting_at(3));
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
