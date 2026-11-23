// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The intake statement's own budget (#5889, #6322): the one statement that
//! writes a batch of access-log entries
//! (`ferroehr::access_log::intake::PLAIN`) runs no nested statement and
//! checks no foreign key per row (#5887), and its time per entry does not
//! grow with the batch, so a batch of 512 entries costs about 16 batches of
//! 32. An entry carries no search token since the frontier round renders
//! them (#6322). The statement is called directly with synthetic indexed
//! entries. No openEHR spec governs the intake: our own design.

use std::time::{Duration, Instant};

use sqlx::PgPool;

/// The calls each batch size is timed over; the fastest counts.
const RUNS: usize = 3;

/// Writes one batch of `entries` entries into the intake under `policy` and
/// returns how long the statement took.
async fn take(pool: &PgPool, policy: &[u8], entries: usize) -> Duration {
    call(pool, policy, entries, None)
        .await
        .expect("the intake statement")
}

/// Calls the intake statement with one batch of `entries` entries under
/// `policy`, each naming the shared part `shared`, which the batch does not
/// carry; how long the statement took, or its error.
async fn call(
    pool: &PgPool,
    policy: &[u8],
    entries: usize,
    shared: Option<&str>,
) -> Result<Duration, sqlx::Error> {
    let none: Vec<Option<String>> = vec![None; entries];
    let uids: Vec<String> = (0..entries)
        .map(|_| uuid::Uuid::now_v7().to_string())
        .collect();
    let content_digests: Vec<String> = (0..entries).map(|n| format!("{:064x}", n + 1)).collect();
    let now = jiff::Timestamp::now();
    let started = Instant::now();
    sqlx::query(ferroehr::access_log::intake::PLAIN)
        .bind(vec!["ACCESS_LOG_ENTRY"; entries])
        .bind(&uids)
        .bind(vec![now.to_string(); entries])
        .bind(vec!["{}"; entries])
        .bind(vec![shared; entries])
        .bind(&content_digests)
        .bind(&none)
        .bind(&none)
        .bind(&none)
        .bind(&none)
        .bind(&none)
        .bind(&none)
        .bind(&none)
        .bind(vec!["R"; entries])
        .bind(vec![0_i16; entries])
        .bind(&none)
        .bind(&none)
        .bind(&none)
        .bind(&none)
        .bind(&none)
        .bind(&none)
        .bind(vec![policy; entries])
        .bind(None::<uuid::Uuid>)
        .bind(Vec::<String>::new())
        .bind(Vec::<String>::new())
        .execute(pool)
        .await?;
    Ok(started.elapsed())
}

/// The fastest of [`RUNS`] calls of `entries` entries, after one call that
/// warms the statement.
async fn fastest(pool: &PgPool, policy: &[u8], entries: usize) -> Duration {
    take(pool, policy, entries).await;
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        best = best.min(take(pool, policy, entries).await);
    }
    best
}

/// The statement nests no statement and takes no more time per entry at 512
/// entries than at 32.
#[tokio::test]
async fn the_intake_statement_grows_with_its_entries_alone() {
    let (_db, pool) = crate::common::migrated_pool().await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    let policy: Vec<u8> = sqlx::query_scalar(
        "SELECT audit.register_render_policy('{\"statement_budget\": true}'::jsonb)",
    )
    .fetch_one(&pool)
    .await
    .expect("the render policy");

    take(&pool, &policy, 1).await;
    let evicted = testkit::statements::statement_window(&pool)
        .await
        .expect("the statement counts reset");
    take(&pool, &policy, 1).await;
    let statements: Vec<(i64, String)> = sqlx::query_as(
        "SELECT calls, left(regexp_replace(query, '\\s+', ' ', 'g'), 120) FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         AND NOT toplevel ORDER BY query",
    )
    .fetch_all(&pool)
    .await
    .expect("pg_stat_statements reads");
    testkit::statements::assert_statements_kept(&pool, evicted)
        .await
        .expect("no statement evicted while the test counted them");
    let (keys, own): (Vec<_>, Vec<_>) = statements
        .iter()
        .partition(|(_, query)| query.contains("FOR KEY SHARE"));
    let sum = |rows: &[&(i64, String)]| rows.iter().map(|(calls, _)| calls).sum::<i64>();
    assert_eq!(
        sum(&own),
        0,
        "the write is one statement, with no search token to resolve (#6322): {statements:#?}"
    );
    assert_eq!(
        sum(&keys),
        0,
        "no foreign-key check per intake row (#5887): {statements:#?}"
    );

    let small = fastest(&pool, &policy, 32).await;
    let large = fastest(&pool, &policy, 512).await;
    let per_small = small.as_secs_f64() / 32.0;
    let per_large = large.as_secs_f64() / 512.0;
    assert!(
        per_large <= 3.0 * per_small,
        "per entry: {per_large:.6} s at 512 entries against {per_small:.6} s at 32 ({large:?} \
         and {small:?} a call)"
    );
}

/// The SQLSTATE of `error`, when the database answered one.
fn code(error: &sqlx::Error) -> Option<String> {
    error
        .as_database_error()
        .and_then(sqlx::error::DatabaseError::code)
        .map(std::borrow::Cow::into_owned)
}

/// An intake row naming a render policy or a shared part the store does not
/// hold is refused as the foreign keys refused it, with no key to lock
/// (#5887).
#[tokio::test]
async fn an_unknown_render_policy_or_shared_part_is_refused() {
    let (_db, pool) = crate::common::migrated_pool().await;
    let error = call(&pool, &[0_u8; 32], 1, None)
        .await
        .expect_err("a policy the store does not hold");
    assert_eq!(code(&error).as_deref(), Some("23503"), "{error}");
    let policy: Vec<u8> = sqlx::query_scalar(
        "SELECT audit.register_render_policy('{\"statement_budget\": true}'::jsonb)",
    )
    .fetch_one(&pool)
    .await
    .expect("the render policy");
    let unknown = format!("{:064x}", 7);
    let error = call(&pool, &policy, 1, Some(&unknown))
        .await
        .expect_err("a shared part neither stored nor in the batch");
    assert_eq!(code(&error).as_deref(), Some("23503"), "{error}");
    let written: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_entry")
        .fetch_one(&pool)
        .await
        .expect("the intake reads");
    assert_eq!(written, 0, "nothing of a refused statement is written");
}
