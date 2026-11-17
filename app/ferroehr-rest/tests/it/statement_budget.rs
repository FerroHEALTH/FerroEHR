// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The statement budget of an AQL request (#5885): the round trips the
//! request sends and the statements `PostgreSQL` runs nested in them, in its
//! functions, triggers and foreign-key checks, held to a stated budget for a
//! single-EHR query and a population query over the assembled router with the
//! access evaluator, the audit drain and the local access log.
//!
//! The round trips are counted in process: every statement sqlx sends inside
//! the request's span (`sqlx::query` events). The nested statements are read
//! from `pg_stat_statements` with `track = all` for the test's own database,
//! so the server loads the module (`shared_preload_libraries`), as the testkit
//! container and CI's server do. The chainer runs on the audit runtime beside
//! the requests and batches what they wrote, so its nested statements are
//! budgeted per round, and the requests' per request and per entry written.
//! The harness runs no LISTEN refresher, so the caller and policy reads a
//! server serves from its copies are sent on every request and counted here.
//! No openEHR spec governs the statements of a request: our own design.

use std::fmt::Write as _;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use axum::Router;
use http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;
use tracing::Instrument as _;
use tracing_subscriber::layer::{Context, SubscriberExt as _};
use tracing_subscriber::registry::LookupSpan;

use crate::access_evaluator_http::{app, ehr_with_composition, send};
use crate::common::BASE;
use ferroehr::db::pool::SESSION_SETUP_SPAN;

mod intake;

/// The EHRs the population query serves, each with one composition.
const EHRS: u64 = 5;

/// The measured requests of each kind.
const REQUESTS: u64 = 20;

/// The span the counted round trips are sent in.
const REQUEST_SPAN: &str = "statement_budget_request";

/// The marker of the test's own reads, left out of the count, which each
/// of them carries as a comment.
const OWN: &str = "statement_budget_own";

/// The nested statements of a chainer round of the access log that takes
/// the head: the lock.
const CHAINER_TAKE: u64 = 1;
/// The nested statements of a chainer round that moves entries into the
/// access log: the move's own statements and the head's triggers.
const CHAINER_ROUND: u64 = 8;
/// The nested statements of a chainer round of the system chain.
const SYSTEM_ROUND: u64 = 6;
/// The nested statements of each access-log entry a request writes: the
/// foreign-key checks of its intake row.
const ENTRY: u64 = 2;

/// The budget of one kind of request: its round trips, and the statements
/// nested in them.
struct Budget {
    /// What the request is.
    name: &'static str,
    /// The AQL body of the request, given the EHR it is pinned to.
    body: fn(&str) -> String,
    /// The EHRs one request serves.
    served: u64,
    /// The round trips one request sends.
    round_trips: u64,
    /// The statements nested in one request's round trips, beside the
    /// [`ENTRY`] statements of each entry it writes.
    nested: u64,
}

const BUDGETS: [Budget; 2] = [
    Budget {
        name: "single-EHR AQL",
        body: single,
        served: 1,
        round_trips: 10,
        nested: 3,
    },
    Budget {
        name: "population AQL",
        body: population,
        served: EHRS,
        round_trips: 8,
        nested: 3,
    },
];

/// The single-EHR query of the capacity harness's shape: pinned to `ehr`.
fn single(ehr: &str) -> String {
    json!({
        "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c WHERE e/ehr_id/value = $ehr",
        "query_parameters": { "ehr": ehr },
    })
    .to_string()
}

/// The population query of the performance gate.
fn population(_: &str) -> String {
    json!({ "q": "SELECT e/ehr_id/value FROM EHR e LIMIT 20" }).to_string()
}

/// Counts the `sqlx::query` events sent inside a [`REQUEST_SPAN`] span and
/// outside a connection's session setup.
#[derive(Clone, Default)]
struct RoundTrips(Arc<AtomicU64>);

impl<S> tracing_subscriber::Layer<S> for RoundTrips
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_event(&self, event: &tracing::Event<'_>, ctx: Context<'_, S>) {
        if event.metadata().target() != "sqlx::query" {
            return;
        }
        let Some(scope) = ctx.event_scope(event) else {
            return;
        };
        let names: Vec<&str> = scope.map(|span| span.name()).collect();
        // A new connection's session setup is the pool's work (#5899).
        if names.contains(&REQUEST_SPAN) && !names.contains(&SESSION_SETUP_SPAN) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
}

/// One statement `pg_stat_statements` counted.
struct Counted {
    toplevel: bool,
    calls: u64,
    query: String,
}

/// The statements of the test's database since the last reset, its own reads
/// left out.
async fn counted(pool: &PgPool) -> Vec<Counted> {
    let rows: Vec<(bool, i64, String)> = sqlx::query_as(
        "SELECT toplevel, calls, regexp_replace(query, '\\s+', ' ', 'g') FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         AND query NOT LIKE '%pg_stat_statements%' AND query NOT LIKE '%' || $1 || '%' \
         ORDER BY toplevel DESC, calls DESC, query",
    )
    .bind(OWN)
    .fetch_all(pool)
    .await
    .expect("pg_stat_statements reads");
    rows.into_iter()
        .map(|(toplevel, calls, query)| Counted {
            toplevel,
            calls: u64::try_from(calls).expect("a count"),
            query,
        })
        .collect()
}

/// The calls of the top-level statements that start with `prefix`.
fn calls(statements: &[Counted], prefix: &str) -> u64 {
    statements
        .iter()
        .filter(|s| s.toplevel && s.query.starts_with(prefix))
        .map(|s| s.calls)
        .sum()
}

/// The ledger of `statements`, one line per statement.
fn ledger(statements: &[Counted]) -> String {
    let mut out = String::new();
    for s in statements {
        let level = if s.toplevel { "top" } else { "nested" };
        let text: String = s.query.chars().take(160).collect();
        writeln!(out, "{level:>6} {:>5}  {text}", s.calls).expect("a String takes the line");
    }
    out
}

/// Waits until the chainer moved every intake row into the log.
async fn drained(pool: &PgPool) {
    for _ in 0..400 {
        let waiting: i64 = sqlx::query_scalar(
            "SELECT /* statement_budget_own */ (SELECT count(*) FROM audit.access_log_intake) \
             + (SELECT count(*) FROM audit.audit_event_intake)",
        )
        .fetch_one(pool)
        .await
        .expect("the intake reads");
        if waiting == 0 {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the chainer did not move the intake into the log");
}

/// The access-log entries in the log.
async fn entries(pool: &PgPool) -> u64 {
    let count: i64 = sqlx::query_scalar(
        "SELECT /* statement_budget_own */ count(*) FROM audit.access_log_record",
    )
    .fetch_one(pool)
    .await
    .expect("the log reads");
    u64::try_from(count).expect("a count")
}

/// Sends `requests` queries of `budget` as the nurse inside the request
/// span, pinned to each of `ehrs` in turn.
async fn query(app: &Router, budget: &Budget, ehrs: &[String], requests: u64) {
    for (_, ehr) in (0..requests).zip(ehrs.iter().cycle()) {
        let (status, body) = send(
            app,
            "POST",
            &format!("{BASE}/query/aql"),
            "nurse",
            None,
            Some(("application/json", (budget.body)(ehr))),
        )
        .instrument(tracing::info_span!(REQUEST_SPAN))
        .await;
        assert_eq!(status, StatusCode::OK, "{}: {body}", budget.name);
        let rows = serde_json::from_str::<Value>(&body).expect("a RESULT_SET")["rows"]
            .as_array()
            .map_or(0, Vec::len);
        assert_eq!(
            u64::try_from(rows).expect("a count"),
            budget.served,
            "{}: one row per served EHR: {body}",
            budget.name
        );
    }
}

/// Each AQL request stays within its round trips and its nested statements.
#[tokio::test]
async fn an_aql_request_stays_within_its_statement_budget() {
    let round_trips = RoundTrips::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(round_trips.clone()));
    let (_db, pool, _svc, app) = Box::pin(app()).await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect(
            "pg_stat_statements: the server loads it (shared_preload_libraries); a testkit \
             container started before #5885 does not: docker rm -f ferroehr-testkit-pg18",
        );
    let track: String = sqlx::query_scalar("SELECT current_setting('pg_stat_statements.track')")
        .fetch_one(&pool)
        .await
        .expect("pg_stat_statements.track reads");
    assert_eq!(track, "all", "the server counts nested statements");
    let mut ehrs = Vec::new();
    for _ in 0..EHRS {
        ehrs.push(ehr_with_composition(&app).await.0);
    }
    let mut report = String::new();
    let mut over = false;
    for budget in &BUDGETS {
        query(&app, budget, &ehrs, EHRS).await;
        drained(&pool).await;
        let before = entries(&pool).await;
        sqlx::query(
            "SELECT pg_stat_statements_reset(0, \
             (SELECT oid FROM pg_database WHERE datname = current_database()), 0)",
        )
        .execute(&pool)
        .await
        .expect("the statement counts reset");
        let sent_before = round_trips.0.load(Ordering::Relaxed);
        query(&app, budget, &ehrs, REQUESTS).await;
        let sent = round_trips.0.load(Ordering::Relaxed) - sent_before;
        drained(&pool).await;
        let statements = counted(&pool).await;
        let written = entries(&pool).await - before;
        let nested: u64 = statements
            .iter()
            .filter(|s| !s.toplevel)
            .map(|s| s.calls)
            .sum();
        let takes = calls(
            &statements,
            "SELECT head_seq, head_digest FROM audit.access_log_chain_take",
        );
        let rounds = calls(&statements, "SELECT audit.access_log_chain_move");
        let system = calls(&statements, "SELECT audit.audit_event_chain_move");
        let chainer = takes * CHAINER_TAKE + rounds * CHAINER_ROUND + system * SYSTEM_ROUND;
        assert!(sent > 0, "{}: the round trips are counted", budget.name);
        assert_eq!(
            written,
            REQUESTS * (budget.served + 1),
            "{}: an entry per served EHR and one for the query",
            budget.name
        );
        writeln!(
            report,
            "{}: {sent} round trips over {REQUESTS} requests (budget {} each); {nested} nested \
             statements against a budget of {chainer} for the chainer ({takes} takes, {rounds} \
             rounds, {system} system rounds), {ENTRY} for each of {written} entries and {} per \
             request\n{}",
            budget.name,
            budget.round_trips,
            budget.nested,
            ledger(&statements),
        )
        .expect("a String takes the report");
        over |= sent > REQUESTS * budget.round_trips
            || nested > chainer + written * ENTRY + REQUESTS * budget.nested;
    }
    assert!(!over, "a request exceeds its statement budget:\n{report}");
    eprintln!("{report}");
}

/// A request that opens a new connection counts only what it sends: the
/// connection's session setup is the pool's (#5899).
#[tokio::test]
async fn a_new_connections_session_setup_is_not_a_round_trip() {
    let round_trips = RoundTrips::default();
    let _guard =
        tracing::subscriber::set_default(tracing_subscriber::registry().with(round_trips.clone()));
    let (_db, pool) = crate::common::migrated_pool().await;
    let fresh = ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Clinical);
    async {
        sqlx::query("SELECT 1")
            .execute(&fresh)
            .await
            .expect("a statement on a new connection");
        sqlx::query("SELECT 1")
            .execute(&fresh)
            .await
            .expect("a statement on the kept connection");
    }
    .instrument(tracing::info_span!(REQUEST_SPAN))
    .await;
    assert_eq!(
        round_trips.0.load(Ordering::Relaxed),
        2,
        "the two statements, and not the new connection's session setup"
    );
}
