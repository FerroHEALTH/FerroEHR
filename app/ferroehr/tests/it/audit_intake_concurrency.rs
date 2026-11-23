// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Concurrent intake writes whose entries name the same shared parts neither
//! deadlock nor lose an entry (#5881): each statement stores the rows several
//! writers share in one fixed order, and the gathered writer answers each
//! request by its own outcome. An entry carries no search token since the
//! frontier round renders them, one batch at a time (#6322).

use std::collections::BTreeSet;

use ferroehr::access_log::intake::{GroupWriter, IntakeEntry, take_in};
use sqlx::PgPool;

use crate::access_log_checkpoint::entry;

/// The system the entries of these tests name.
const SYSTEM: &str = "cdr-intake-concurrency.example.org";

/// The EHR the entries of these tests name.
const EHR: &str = "7d44b88c-4199-4bad-97dc-0000000000e2";

/// The writers that run at once.
const WRITERS: u32 = 16;

/// The rounds each writer runs.
const ROUNDS: u32 = 40;

/// The entries of one writer's statement.
const ENTRIES: u32 = 6;

/// The entry number of `entry` of `writer` in `round`, unique across a test.
fn number(round: u32, writer: u32, entry: u32) -> u32 {
    (round * WRITERS + writer) * ENTRIES + entry
}

/// One writer's statement of `round`.
fn batch(round: u32, writer: u32) -> Vec<IntakeEntry> {
    (0..ENTRIES)
        .map(|e| {
            IntakeEntry::of(&mut entry(number(round, writer, e), SYSTEM, EHR), None)
                .expect("a stored form")
        })
        .collect()
}

/// Asserts the intake holds every entry the writers wrote, once.
async fn every_entry_once(pool: &PgPool) {
    let uids: Vec<String> = sqlx::query_scalar("SELECT e.uid::text FROM audit.access_log_entry e")
        .fetch_all(pool)
        .await
        .expect("the intake");
    let distinct: BTreeSet<&String> = uids.iter().collect();
    let expected = usize::try_from(ROUNDS * WRITERS * ENTRIES).expect("small");
    assert_eq!(uids.len(), expected, "every entry is in the intake");
    assert_eq!(distinct.len(), expected, "each entry once");
}

/// Sixteen statements at once, each naming the same shared part, all commit:
/// no deadlock, no entry lost.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test writes the intake on raw pool connections, as the gathered writer does"
)]
async fn concurrent_statements_with_a_shared_part_all_commit() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    for round in 0..ROUNDS {
        let mut writers = tokio::task::JoinSet::new();
        for writer in 0..WRITERS {
            let pool = pool.clone();
            writers.spawn(async move {
                let entries = batch(round, writer);
                let mut conn = pool.acquire().await.expect("a connection");
                take_in(&mut conn, &entries, None, &BTreeSet::new()).await
            });
        }
        while let Some(written) = writers.join_next().await {
            written.expect("a writer").unwrap_or_else(|error| {
                panic!("round {round}: the intake write failed: {error:?}")
            });
        }
    }
    every_entry_once(&pool).await;
}

/// Requests through the gathered writer, many at once naming one shared
/// part, are each answered with success, and every entry is written.
#[tokio::test]
async fn the_gathered_writer_answers_every_overlapping_request() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let writer = GroupWriter::spawn(&pool);
    for round in 0..ROUNDS {
        let mut requests = tokio::task::JoinSet::new();
        for request in 0..WRITERS {
            let writer = writer.clone();
            requests.spawn(async move { writer.write(batch(round, request)).await });
        }
        while let Some(answered) = requests.join_next().await {
            answered
                .expect("a request")
                .unwrap_or_else(|error| panic!("round {round}: a request was refused: {error:?}"));
        }
    }
    every_entry_once(&pool).await;
}
