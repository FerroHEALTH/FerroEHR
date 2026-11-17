// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The intake statement's own budget (#5889): with every search token and
//! shared set already stored, `audit.access_log_intake_take()` runs three
//! nested statements (the token hold, the grouped pass, the write) and
//! checks no foreign key per row (#5887), and its
//! time per entry does not grow with the batch, so a batch of 512 entries
//! costs about 16 batches of 32. The statement is called directly with
//! synthetic indexed entries of 20 search tokens each, five of them in a
//! shared set every entry names. No openEHR spec governs the intake: our own
//! design.

use std::time::{Duration, Instant};

use sqlx::PgPool;

/// The search tokens of each synthetic entry.
const TOKENS: usize = 20;

/// The tokens of each entry that belong to its shared set.
const SHARED: usize = 5;

/// The calls each batch size is timed over; the fastest counts.
const RUNS: usize = 3;

/// The token columns of a batch, by token.
#[derive(Default)]
struct Tokens {
    entries: Vec<i32>,
    parameters: Vec<String>,
    systems: Vec<Option<String>>,
    codes: Vec<Option<String>>,
    shares: Vec<bool>,
}

/// The tokens of `entries` entries: the shared ones the same for all, the
/// others each entry's own, the same on every call with the same `entries`.
fn tokens(entries: usize) -> Tokens {
    let mut out = Tokens::default();
    for (position, entry) in (1_i32..).zip(0..entries) {
        for token in 0..TOKENS {
            out.entries.push(position);
            let shares = token < SHARED;
            out.parameters
                .push(if shares { "type" } else { "entity.identifier" }.to_owned());
            out.systems.push(Some("urn:ferroehr:budget".to_owned()));
            out.codes.push(Some(if shares {
                format!("shared-{token}")
            } else {
                format!("own-{entry}-{token}")
            }));
            out.shares.push(shares);
        }
    }
    out
}

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
    let t = tokens(entries);
    let digests: Vec<Vec<u8>> = sqlx::query_scalar(
        "SELECT audit.search_token_digest(t.p, t.s, t.c) \
         FROM unnest($1::text[], $2::text[], $3::text[]) WITH ORDINALITY AS t(p, s, c, i) \
         ORDER BY t.i",
    )
    .bind(&t.parameters)
    .bind(&t.systems)
    .bind(&t.codes)
    .fetch_all(pool)
    .await
    .expect("the token digests");
    let none: Vec<Option<String>> = vec![None; entries];
    let uids: Vec<String> = (0..entries)
        .map(|_| uuid::Uuid::now_v7().to_string())
        .collect();
    let content_digests: Vec<String> = (0..entries).map(|n| format!("{:064x}", n + 1)).collect();
    let now = jiff::Timestamp::now();
    let started = Instant::now();
    sqlx::query(
        "SELECT audit.access_log_intake_take($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, \
         $13, $14, $15, $16, $17, $18, $19, $20, $21, $22, $23, $24, $25, $26, $27, $28, $29, \
         $30, $31, $32, $33)",
    )
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
    .bind(vec![jiff_sqlx::Timestamp::from(now); entries])
    .bind(vec!["R"; entries])
    .bind(vec![0_i16; entries])
    .bind(&none)
    .bind(&none)
    .bind(&none)
    .bind(&none)
    .bind(&none)
    .bind(&none)
    .bind(vec![""; entries])
    .bind(vec![policy; entries])
    .bind(Vec::<String>::new())
    .bind(Vec::<String>::new())
    .bind(&t.entries)
    .bind(&t.parameters)
    .bind(&t.systems)
    .bind(&t.codes)
    .bind(&t.shares)
    .bind(None::<uuid::Uuid>)
    .bind(&digests)
    .execute(pool)
    .await?;
    Ok(started.elapsed())
}

/// The fastest of [`RUNS`] calls of `entries` entries, every token and set
/// stored by a call before.
async fn fastest(pool: &PgPool, policy: &[u8], entries: usize) -> Duration {
    take(pool, policy, entries).await;
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        best = best.min(take(pool, policy, entries).await);
    }
    best
}

/// With its tokens and sets stored, the statement nests three statements and
/// takes no more time per entry at 512 entries than at 32.
#[tokio::test]
async fn the_intake_statement_grows_with_its_tokens_alone() {
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
    sqlx::query(
        "SELECT pg_stat_statements_reset(0, \
         (SELECT oid FROM pg_database WHERE datname = current_database()), 0)",
    )
    .execute(&pool)
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
    let (keys, own): (Vec<_>, Vec<_>) = statements
        .iter()
        .partition(|(_, query)| query.contains("FOR KEY SHARE"));
    let sum = |rows: &[&(i64, String)]| rows.iter().map(|(calls, _)| calls).sum::<i64>();
    assert_eq!(
        sum(&own),
        3,
        "the token hold, the grouped pass and the write: {statements:#?}"
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
    let written: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_intake")
        .fetch_one(&pool)
        .await
        .expect("the intake reads");
    assert_eq!(written, 0, "nothing of a refused statement is written");
}
