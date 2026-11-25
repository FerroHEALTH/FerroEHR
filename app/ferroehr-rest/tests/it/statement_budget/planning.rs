// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The planning ceiling of every hot path (#6392): over [`REQUESTS`] requests
//! of a path on one connection per pool, `PostgreSQL` plans each statement at
//! most [`PLANS`] times: it settles on the generic plan within five
//! executions (`PostgreSQL` docs, `PREPARE`, "Notes").
//!
//! The plans are read from `pg_stat_statements` with `track_planning` on for
//! the test's database, whose tables leave autovacuum and are analyzed in
//! full before each path is warmed, so the statistics a window plans with are
//! exact and do not move inside it. No openEHR spec governs the planning of a statement:
//! our own design.

use std::fmt::Write as _;

use axum::Router;
use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;

use super::paths::{World, exchange, order};
use super::{OWN, drained};

/// The measured requests of each path.
pub(super) const REQUESTS: u64 = 20;

/// The plans each statement may take over a path's requests: the five custom
/// plans the plan cache makes before it weighs the generic one, and the
/// generic one.
const PLANS: u64 = 6;

/// The openings of the statements known to plan on every call, each held
/// over the ceiling: one that settles fails the test until it leaves the list.
const KNOWN: [&str; 0] = [];

/// The assembled router of [`super::served`] over pools of one connection
/// each, with `track_planning` on; the second pool reaches the database
/// directly for the test's own reads.
async fn single_connection() -> (
    testkit::TestDb,
    PgPool,
    std::sync::Arc<ferroehr::service::FerroEhrService>,
    Router,
) {
    let (db, direct) = crate::common::migrated_pool().await;
    testkit::statements::track_planning(&direct)
        .await
        .expect("the test role sets track_planning on its database");
    let mut settings = ferroehr::db::DbConfig::new(db.url());
    settings.max_connections = Some(1);
    settings.min_connections = Some(0);
    let pools = ferroehr::db::connect_domains(&settings)
        .await
        .expect("the domain pools connect");
    let audit = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .connect_lazy_with(direct.connect_options().as_ref().clone());
    let (db, _, svc, app) = Box::pin(crate::access_evaluator_http::app_over_pools(
        crate::party_binding_http::mapping(),
        db,
        audit,
        &pools,
    ))
    .await;
    super::listening(&db, &svc).await;
    (db, direct, svc, app)
}

/// The statements of the window that `PostgreSQL` planned more than
/// [`PLANS`] times, each with its plans and calls, and the ledger of every
/// statement it planned.
async fn planned(pool: &PgPool) -> (Vec<(String, String)>, String) {
    let rows: Vec<(i64, i64, f64, f64, String)> = sqlx::query_as(
        "SELECT plans, calls, total_plan_time / plans, total_exec_time / greatest(calls, 1), \
         regexp_replace(query, '\\s+', ' ', 'g') FROM pg_stat_statements \
         WHERE dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         AND plans > 0 AND query NOT LIKE '%pg_stat_statements%' \
         AND query NOT LIKE '%' || $1 || '%' ORDER BY plans DESC, query",
    )
    .bind(OWN)
    .fetch_all(pool)
    .await
    .expect("pg_stat_statements reads");
    let mut over = Vec::new();
    let mut ledger = String::new();
    for (plans, calls, plan_ms, exec_ms, query) in rows {
        let text: String = query.chars().take(160).collect();
        writeln!(
            ledger,
            "  {plans:>4} plans {calls:>4} calls {plan_ms:>7.3} ms a plan {exec_ms:>7.3} ms a call  \
             {text}"
        )
        .unwrap();
        if u64::try_from(plans).unwrap() > PLANS {
            over.push((query, format!("{plans} plans in {calls} calls: {text}")));
        }
    }
    (over, ledger)
}

/// Takes the test's tables out of autovacuum, whose ANALYZE would otherwise
/// fall inside a path's window at a time the run's speed decides: a new
/// statistic invalidates the cached plans and plans each statement again
/// (`PostgreSQL` docs, `PREPARE`, "Notes"; Routine Vacuuming, "Updating
/// Planner Statistics").
async fn without_autovacuum(pool: &PgPool) {
    sqlx::query(
        "DO $$ DECLARE t regclass; BEGIN \
         FOR t IN SELECT c.oid::regclass FROM pg_class c \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             WHERE c.relkind = 'r' AND pg_has_role(c.relowner, 'USAGE') \
               AND n.nspname NOT IN ('pg_catalog', 'information_schema') LOOP \
           EXECUTE format('ALTER TABLE %s SET (autovacuum_enabled = off)', t); \
         END LOOP; END $$",
    )
    .execute(pool)
    .await
    .expect("the test's tables leave autovacuum");
}

/// Analyzes the test's database reading every row, so each path's window
/// plans with exact statistics of the writes before it: a sampled `ANALYZE`
/// reads 300 rows per point of `default_statistics_target`, and a table past
/// the sample gets different statistics on every run (`PostgreSQL` docs,
/// `ANALYZE`, "Notes"; #6544).
async fn settle_statistics(pool: &PgPool) {
    sqlx::raw_sql("BEGIN; SET LOCAL default_statistics_target = 10000; ANALYZE; COMMIT")
        .execute(pool)
        .await
        .expect("the test's database is analyzed");
}

/// Every statement of every hot path settles on a generic plan: over
/// [`REQUESTS`] requests on one connection per pool, none is planned more
/// than [`PLANS`] times.
#[tokio::test]
async fn every_hot_path_settles_each_statement_on_a_generic_plan() {
    ferroehr::aql::sql::without_reference_cells();
    let (_db, pool, svc, app) = Box::pin(single_connection()).await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    without_autovacuum(&pool).await;
    let world = World::build(&app).await;
    let mut report = String::new();
    let mut over = Vec::new();
    let mut known = [false; KNOWN.len()];
    for path in order() {
        world.prepare(&app, &svc, path).await;
        settle_statistics(&pool).await;
        world.warm(&app, path).await;
        drained(&pool).await;
        let evicted = testkit::statements::statement_window(&pool)
            .await
            .expect("the statement counts reset");
        for n in 0..REQUESTS {
            let n = usize::try_from(super::paths::WARM + n).unwrap();
            let (status, headers, body) = exchange(&app, world.call(path, n).request()).await;
            assert!(
                status.is_success(),
                "{}: {status} {}",
                path.name(),
                String::from_utf8_lossy(&body)
            );
            world.observe(path, n, &headers);
        }
        drained(&pool).await;
        let (statements, ledger) = planned(&pool).await;
        testkit::statements::assert_statements_kept(&pool, evicted)
            .await
            .expect("no statement evicted while the test counted them");
        writeln!(report, "{}:\n{ledger}", path.name()).unwrap();
        for (query, line) in statements {
            match KNOWN.iter().position(|opening| query.starts_with(opening)) {
                Some(at) => known[at] = true,
                None => over.push(format!("{}: {line}", path.name())),
            }
        }
    }
    eprintln!("plans per statement over {REQUESTS} requests:\n{report}");
    assert!(
        over.is_empty(),
        "a statement is planned more than {PLANS} times over {REQUESTS} requests:\n{}\n{report}",
        over.join("\n")
    );
    let settled: Vec<_> = KNOWN
        .iter()
        .zip(known)
        .filter_map(|(opening, seen)| (!seen).then_some(opening))
        .collect();
    assert!(
        settled.is_empty(),
        "these known statements now settle, so they leave KNOWN: {settled:?}"
    );
}
