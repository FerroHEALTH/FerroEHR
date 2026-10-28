// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An AQL expression at the parser's depth bound runs end to end, and one
//! level beyond it is a `400` before any recursive pass sees it (#4284).
//!
//! The query at the bound goes through every recursive pass the server runs
//! (parse, lowering, SQL generation) and through PostgreSQL's own parser,
//! so the bound is proven to sit below every limit behind it. It runs on a
//! thread with the stack the server gives every runtime thread (16 MiB,
//! `ferroehr-server`'s `RUNTIME_THREAD_STACK_BYTES`), which is the stack the
//! passes have in production.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use openehr_query::parser::MAX_EXPRESSION_DEPTH;

use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::status::CallStatusType;

/// The stack the server gives each runtime thread.
const SERVER_THREAD_STACK_BYTES: usize = 16 * 1024 * 1024;

/// Runs `body` on a runtime whose threads carry the server's stack.
fn on_server_stack<F>(body: impl FnOnce() -> F + Send + 'static)
where
    F: Future<Output = ()>,
{
    std::thread::Builder::new()
        .stack_size(SERVER_THREAD_STACK_BYTES)
        .spawn(move || {
            tokio::runtime::Builder::new_multi_thread()
                .worker_threads(2)
                .thread_stack_size(SERVER_THREAD_STACK_BYTES)
                .enable_all()
                .build()
                .expect("the runtime builds")
                .block_on(body());
        })
        .expect("the thread spawns")
        .join()
        .expect("the test body completes");
}

fn scope(ehr_id: &str) -> AqlQueryRequest {
    AqlQueryRequest {
        ehr_ids: vec![ehr_id.to_owned()],
        ..AqlQueryRequest::default()
    }
}

/// `NOT` repeated `n` times (even, so the predicate keeps its meaning) over a
/// match on `ehr_id`.
fn not_chain(n: usize, ehr_id: &str) -> String {
    format!(
        "SELECT e/ehr_id/value FROM EHR e WHERE {}e/ehr_id/value = '{ehr_id}'",
        "NOT ".repeat(n)
    )
}

/// `operators` ORs, the last term matching `ehr_id`.
fn or_chain(operators: usize, ehr_id: &str) -> String {
    let mut terms: Vec<String> = (0..operators)
        .map(|i| format!("e/ehr_id/value = 'absent-{i}'"))
        .collect();
    terms.push(format!("e/ehr_id/value = '{ehr_id}'"));
    format!(
        "SELECT e/ehr_id/value FROM EHR e WHERE {}",
        terms.join(" OR ")
    )
}

#[test]
fn chains_at_the_depth_bound_run_through_every_pass_and_postgresql() {
    on_server_stack(|| async {
        let db = testkit::db().await.expect("testkit database");
        let pool = db.pool();
        let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
        let ehr_id = svc.create_ehr(None).await.expect("create_ehr").to_string();
        let even_bound = MAX_EXPRESSION_DEPTH - MAX_EXPRESSION_DEPTH % 2;
        for aql in [
            not_chain(even_bound, &ehr_id),
            or_chain(MAX_EXPRESSION_DEPTH, &ehr_id),
        ] {
            let answer = svc
                .execute_ad_hoc_query(aql, scope(&ehr_id))
                .await
                .expect("a query at the bound runs");
            let rows = answer.result_set["rows"]
                .as_array()
                .expect("a rows array")
                .len();
            assert_eq!(rows, 1, "the chain matches the one EHR");
        }
    });
}

#[test]
fn chains_beyond_the_depth_bound_are_refused_as_invalid_queries() {
    on_server_stack(|| async {
        let db = testkit::db().await.expect("testkit database");
        let pool = db.pool();
        let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
        let ehr_id = svc.create_ehr(None).await.expect("create_ehr").to_string();
        for aql in [
            not_chain(MAX_EXPRESSION_DEPTH + 2, &ehr_id),
            or_chain(MAX_EXPRESSION_DEPTH + 1, &ehr_id),
            not_chain(20_000, &ehr_id),
            or_chain(30_000, &ehr_id),
        ] {
            let refused = svc
                .execute_ad_hoc_query(aql, scope(&ehr_id))
                .await
                .expect_err("a chain beyond the bound is refused");
            assert_eq!(refused.status, CallStatusType::PreconditionViolation);
            assert!(
                refused.message.contains("deepest this parser accepts"),
                "the refusal says why, got {:?}",
                refused.message
            );
        }
    });
}
