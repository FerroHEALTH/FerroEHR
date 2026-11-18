// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Concurrent intake writes whose entries name the same new search tokens
//! and shared sets, in different orders, neither deadlock nor lose an entry
//! (#5881): each statement stores the rows several writers share in one
//! fixed order, and the gathered writer answers each request by its own
//! outcome.

use std::collections::BTreeSet;

use ferroehr::access_log::intake::{GroupWriter, IntakeEntry, IntakeToken, take_in};
use sqlx::PgPool;

use crate::access_log_checkpoint::entry;

/// The system the entries of these tests name.
const SYSTEM: &str = "cdr-intake-concurrency.example.org";

/// The EHR the entries of these tests name.
const EHR: &str = "7d44b88c-4199-4bad-97dc-0000000000e2";

/// The writers that run at once.
const WRITERS: u32 = 16;

/// The rounds each writer runs; every round names new tokens.
const ROUNDS: u32 = 40;

/// The entries of one writer's statement.
const ENTRIES: u32 = 6;

/// The tokens every writer of a round names.
const TOKENS: u32 = 12;

/// The tokens of one entry of `writer` in `round`: every token of the round,
/// in an order that differs per writer and per entry, half of them in the
/// entry's shared set.
fn tokens(round: u32, writer: u32, entry: u32) -> Vec<IntakeToken> {
    let mut codes: Vec<u32> = (0..TOKENS).collect();
    codes.rotate_left(usize::try_from((writer * 5 + entry * 3) % TOKENS).expect("small"));
    if (writer + entry) % 2 == 1 {
        codes.reverse();
    }
    codes
        .into_iter()
        .map(|code| IntakeToken {
            parameter: if code % 3 == 0 {
                "purpose"
            } else {
                "entity.identifier"
            },
            system: Some("urn:example:intake-concurrency".to_owned()),
            code: Some(format!("{round}-{code}")),
            shares: (code + writer + entry).is_multiple_of(2 + entry % 3),
        })
        .collect()
}

/// The entry number of `entry` of `writer` in `round`, unique across a test.
fn number(round: u32, writer: u32, entry: u32) -> u32 {
    (round * WRITERS + writer) * ENTRIES + entry
}

/// One writer's statement of `round`.
fn batch(round: u32, writer: u32) -> (Vec<IntakeEntry>, Vec<Vec<IntakeToken>>) {
    (0..ENTRIES)
        .map(|e| {
            let intake = IntakeEntry::of(&mut entry(number(round, writer, e), SYSTEM, EHR), None)
                .expect("a stored form");
            (intake, tokens(round, writer, e))
        })
        .unzip()
}

/// Asserts the intake holds every entry the writers wrote, once, and each
/// token once.
async fn every_entry_once(pool: &PgPool) {
    let uids: Vec<String> = sqlx::query_scalar(
        "SELECT e.uid::text FROM audit.access_log_intake i \
         CROSS JOIN LATERAL unnest(i.entries) AS n(id) JOIN audit.access_log_entry e ON e.id = n.id",
    )
        .fetch_all(pool)
        .await
        .expect("the intake");
    let distinct: BTreeSet<&String> = uids.iter().collect();
    let expected = usize::try_from(ROUNDS * WRITERS * ENTRIES).expect("small");
    assert_eq!(uids.len(), expected, "every entry is in the intake");
    assert_eq!(distinct.len(), expected, "each entry once");
    let tokens: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit.audit_search_token \
         WHERE system = 'urn:example:intake-concurrency'",
    )
    .fetch_one(pool)
    .await
    .expect("the tokens");
    assert_eq!(tokens, i64::from(ROUNDS * TOKENS), "each token once");
}

/// Sixteen statements at once, each naming the same new tokens and sets in
/// its own order, all commit: no deadlock, no entry lost.
#[tokio::test]
#[expect(
    clippy::disallowed_methods,
    reason = "a test writes the intake on raw pool connections, as the gathered writer does"
)]
async fn concurrent_statements_with_overlapping_tokens_all_commit() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    for round in 0..ROUNDS {
        let mut writers = tokio::task::JoinSet::new();
        for writer in 0..WRITERS {
            let pool = pool.clone();
            writers.spawn(async move {
                let (entries, tokens) = batch(round, writer);
                let mut conn = pool.acquire().await.expect("a connection");
                take_in(&mut conn, &entries, &tokens, None, &BTreeSet::new()).await
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

/// Requests through the gathered writer, many at once with overlapping
/// tokens, are each answered with success, and every entry is written.
#[tokio::test]
async fn the_gathered_writer_answers_every_overlapping_request() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let writer = GroupWriter::spawn(&pool);
    for round in 0..ROUNDS {
        let mut requests = tokio::task::JoinSet::new();
        for request in 0..WRITERS {
            let writer = writer.clone();
            requests.spawn(async move {
                let (entries, tokens) = batch(round, request);
                writer.write(entries, tokens).await
            });
        }
        while let Some(answered) = requests.join_next().await {
            answered
                .expect("a request")
                .unwrap_or_else(|error| panic!("round {round}: a request was refused: {error:?}"));
        }
    }
    every_entry_once(&pool).await;
}
