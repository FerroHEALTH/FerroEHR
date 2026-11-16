// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit intake and the chainer (#5747, #5745): writers put entries into
//! the append-only intake without left on the head, chainers on several
//! replicas link them into one chain, and the chain they leave verifies,
//! every derived column equal to its derivation and every append sealed.

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use ferroehr::access_log::chainer::{Owed, Round};
use ferroehr::access_log::intake::{IntakeEntry, take_in};
use sqlx::{Connection, PgPool};

use crate::access_log_checkpoint::{checkpoints, entry, generate_key, keyring};

/// The system the entries of these tests name.
const SYSTEM: &str = "cdr-intake.example.org";

/// The EHR the entries of these tests name.
const EHR: &str = "7d44b88c-4199-4bad-97dc-0000000000e1";

/// Writes the entries `numbers` into the intake, one transaction each.
async fn write(pool: &PgPool, numbers: impl Iterator<Item = u32>) {
    for n in numbers {
        let mut conn = pool.acquire().await.expect("a connection");
        let mut tx = conn.begin().await.expect("begin");
        let intake = IntakeEntry::of(&mut entry(n, SYSTEM, EHR), None).expect("a stored form");
        take_in(&mut tx, vec![intake], &[])
            .await
            .expect("the intake write");
        tx.commit().await.expect("commit");
    }
}

/// Four writers and three chainers, each chainer as another replica would
/// run it, leave one chain: every entry once, at positions 1 to n with no
/// gap, every link and derived column checked by the verifier, and each
/// round sealed by a checkpoint that verifies under the key.
#[tokio::test]
async fn chainers_on_several_replicas_keep_one_chain() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let log = Arc::new(checkpoints(&pool, SYSTEM, keyring(&generate_key())));
    let writing = Arc::new(AtomicBool::new(true));

    let mut chainers = tokio::task::JoinSet::new();
    for _ in 0..3 {
        let log = Arc::clone(&log);
        let writing = Arc::clone(&writing);
        chainers.spawn(async move {
            loop {
                let still_writing = writing.load(Ordering::Acquire);
                match log
                    .store()
                    .chain_round(16, Owed::default())
                    .await
                    .expect("a chainer round")
                {
                    Round::Chained { entries: 0, .. } if !still_writing => return,
                    Round::Busy | Round::Chained { .. } => {}
                }
                tokio::task::yield_now().await;
            }
        });
    }
    let mut writers = tokio::task::JoinSet::new();
    for writer in 0..4_u32 {
        let pool = pool.clone();
        writers.spawn(async move { write(&pool, (0..50).map(|i| writer * 1000 + i)).await });
    }
    while let Some(written) = writers.join_next().await {
        written.expect("a writer");
    }
    writing.store(false, Ordering::Release);
    while let Some(round) = chainers.join_next().await {
        round.expect("a chainer");
    }
    log.store()
        .settle(Owed::default())
        .await
        .expect("the intake is chained");

    let positions: Vec<i64> =
        sqlx::query_scalar("SELECT chain_seq FROM audit.access_log_record ORDER BY chain_seq")
            .fetch_all(&pool)
            .await
            .expect("the positions");
    assert_eq!(
        positions,
        (1..=200).collect::<Vec<i64>>(),
        "one chain with no gap"
    );
    let uids: BTreeSet<String> =
        sqlx::query_scalar("SELECT uid::text FROM audit.access_log_record")
            .fetch_all(&pool)
            .await
            .expect("the uids")
            .into_iter()
            .collect();
    assert_eq!(uids.len(), 200, "every entry chained once");
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_intake")
        .fetch_one(&pool)
        .await
        .expect("the intake");
    assert_eq!(left, 0, "the intake is empty");
    let findings: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.verify_access_log_chain()")
        .fetch_one(&pool)
        .await
        .expect("the verifier");
    assert_eq!(findings, 0, "the verifier finds the chain intact");
    let verification = log.verify(None).await.expect("verify the checkpoints");
    assert!(verification.intact(), "{verification:?}");
    let checkpoints: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_checkpoint")
        .fetch_one(&pool)
        .await
        .expect("the checkpoints");
    assert!(checkpoints >= 1, "each round seals its last link");
}

/// The intake is append-only: the runtime refuses an update, a delete and a
/// truncate of an entry left to be chained.
#[tokio::test]
async fn the_intake_refuses_every_change_but_the_chainers_move() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    write(&pool, std::iter::once(1)).await;
    let outside = ferroehr::system_log::store::AuditRecord {
        event: ferroehr::system_log::event::AuditEvent::new(
            ferroehr::system_log::event::EventActionCode::Execute,
            ferroehr::system_log::event::ObjectClass::Authentication,
            ferroehr::system_log::event::EventOutcome::Success,
        ),
        subject: None,
        entry: None,
    };
    ferroehr::system_log::store::AuditStore::new(pool.clone())
        .write(vec![outside], &[])
        .await
        .expect("a record of the system chain waits in its intake");
    for statement in [
        "UPDATE audit.access_log_intake SET content = '{}'",
        "DELETE FROM audit.access_log_intake",
        "TRUNCATE audit.access_log_intake",
        "UPDATE audit.audit_event_intake SET action = 'R'",
        "DELETE FROM audit.audit_event_intake",
        "TRUNCATE audit.audit_event_intake",
    ] {
        let refused = sqlx::query(statement)
            .execute(&pool)
            .await
            .expect_err(statement);
        assert!(
            refused.to_string().contains("append-only"),
            "{statement}: {refused}"
        );
    }
    let left: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_intake")
        .fetch_one(&pool)
        .await
        .expect("the intake");
    assert_eq!(left, 1, "the entry still waits");
}

/// A link the chainer did not compute from the head is refused by the
/// insert trigger, so a writer that skips the head cannot extend the chain.
#[tokio::test]
async fn a_record_inserted_past_the_head_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    write(&pool, std::iter::once(1)).await;
    let refused = sqlx::query(
        "SELECT audit.access_log_chain_move( \
         ARRAY(SELECT id FROM audit.access_log_intake), ARRAY[7]::bigint[], \
         ARRAY[repeat('0', 64)], ARRAY[repeat('0', 64)], false, false)",
    )
    .execute(&pool)
    .await
    .expect_err("a link that does not follow the head");
    assert!(
        refused.to_string().contains("follows position"),
        "{refused}"
    );
}
