// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The write-ahead log the access log writes per entry (#NNNN): the bytes
//! the statements that take an entry in, release it and chain it log,
//! read from `pg_stat_statements` for the test's own database, so the
//! other tests on the shared server do not count. The server runs without
//! full-page writes, as the testkit container and CI's server do, so the
//! count is the records' own bytes and does not depend on when a checkpoint
//! ran. No openEHR spec governs audit storage: our own design.

use std::fmt::Write as _;

use axum::Router;
use http::StatusCode;
use sqlx::PgPool;

use super::{BASE, EHRS, drained, entries, population, single};
use crate::access_evaluator_http::{app, ehr_with_composition, send};

/// The requests of each kind measured.
const REQUESTS: u64 = 20;

/// The most write-ahead log the access log may write per entry, in bytes,
/// over the statements that take, release and chain it.
const CEILING: u64 = 0;

/// A kind of request, and how to send one.
#[derive(Clone, Copy)]
enum Kind {
    /// A composition read as the nurse.
    Read,
    /// A single-EHR AQL query.
    Single,
    /// A population AQL query.
    Population,
    /// A composition commit as the administrator.
    Write,
}

impl Kind {
    const ALL: [Self; 4] = [Self::Read, Self::Single, Self::Population, Self::Write];

    fn name(self) -> &'static str {
        match self {
            Self::Read => "composition read",
            Self::Single => "single-EHR AQL",
            Self::Population => "population AQL",
            Self::Write => "composition commit",
        }
    }

    async fn send(self, app: &Router, ehr: &str, vo: &str) {
        let (status, body) = match self {
            Self::Read => {
                send(
                    app,
                    "GET",
                    &format!("{BASE}/ehr/{ehr}/composition/{vo}"),
                    "nurse",
                    None,
                    None,
                )
                .await
            }
            Self::Single | Self::Population => {
                let body = if matches!(self, Self::Single) {
                    single(ehr)
                } else {
                    population(ehr)
                };
                send(
                    app,
                    "POST",
                    &format!("{BASE}/query/aql"),
                    "nurse",
                    None,
                    Some(("application/json", body)),
                )
                .await
            }
            Self::Write => {
                send(
                    app,
                    "POST",
                    &format!("{BASE}/ehr/{ehr}/composition"),
                    "root",
                    None,
                    Some((
                        "application/json",
                        crate::common::ips_canonical_composition().to_string(),
                    )),
                )
                .await
            }
        };
        assert!(
            status == StatusCode::OK || status == StatusCode::CREATED,
            "{}: {status} {body}",
            self.name()
        );
    }
}

/// The write-ahead log of the test's database's top-level statements since
/// the last reset, by statement, its own reads left out: bytes, full-page
/// images, calls and the text.
async fn logged(pool: &PgPool) -> Vec<(u64, u64, u64, String)> {
    let rows: Vec<(i64, i64, i64, String)> = sqlx::query_as(
        "SELECT /* statement_budget_own */ wal_bytes::bigint, wal_fpi, calls, \
         regexp_replace(query, '\\s+', ' ', 'g') FROM pg_stat_statements \
         WHERE toplevel AND dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         AND query NOT LIKE '%pg_stat_statements%' AND query NOT LIKE '%statement_budget_own%' \
         ORDER BY wal_bytes DESC, query",
    )
    .fetch_all(pool)
    .await
    .expect("pg_stat_statements reads");
    rows.into_iter()
        .map(|(bytes, fpi, calls, query)| {
            (
                u64::try_from(bytes).expect("a count"),
                u64::try_from(fpi).expect("a count"),
                u64::try_from(calls).expect("a count"),
                query,
            )
        })
        .collect()
}

/// Whether `query` is one of the access log's own statements: the intake
/// write, the release of a reservation and the chainer's round.
fn access_log(query: &str) -> bool {
    [
        "SELECT audit.access_log_intake_take",
        "SELECT audit.audit_intake_release",
        "SELECT audit.access_log_chain_move",
        "SELECT head_seq, head_digest FROM audit.access_log_chain_take",
        "SELECT id, encode(content_digest",
    ]
    .iter()
    .any(|prefix| query.starts_with(prefix))
}

/// Each kind of request's entries cost the access log at most [`CEILING`]
/// bytes of write-ahead log each.
#[tokio::test]
async fn an_access_log_entry_stays_within_its_wal_ceiling() {
    let (_db, pool, _svc, app) = Box::pin(app()).await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    let full_pages: String = sqlx::query_scalar("SELECT current_setting('full_page_writes')")
        .fetch_one(&pool)
        .await
        .expect("full_page_writes reads");
    assert_eq!(
        full_pages, "off",
        "the count is of the records alone, as on the testkit and CI servers"
    );
    let mut made = Vec::new();
    for _ in 0..EHRS {
        made.push(ehr_with_composition(&app).await);
    }
    let mut report = String::new();
    let mut over = false;
    for kind in Kind::ALL {
        for (ehr, vo) in &made {
            kind.send(&app, ehr, vo).await;
        }
        drained(&pool).await;
        let before = entries(&pool).await;
        sqlx::query(
            "SELECT pg_stat_statements_reset(0, \
             (SELECT oid FROM pg_database WHERE datname = current_database()), 0)",
        )
        .execute(&pool)
        .await
        .expect("the statement counts reset");
        for (_, (ehr, vo)) in (0..REQUESTS).zip(made.iter().cycle()) {
            kind.send(&app, ehr, vo).await;
        }
        drained(&pool).await;
        let written = entries(&pool).await - before;
        let statements = logged(&pool).await;
        let ours: u64 = statements
            .iter()
            .filter(|(_, _, _, query)| access_log(query))
            .map(|(bytes, ..)| bytes)
            .sum();
        let all: u64 = statements.iter().map(|(bytes, ..)| bytes).sum();
        assert!(written > 0, "{}: the requests write entries", kind.name());
        let per_entry = ours / written;
        writeln!(
            report,
            "{}: {written} entries over {REQUESTS} requests; the access log wrote {ours} bytes, \
             {per_entry} per entry (ceiling {CEILING}); every statement {all} bytes, {} per request",
            kind.name(),
            all / REQUESTS,
        )
        .expect("a String takes the report");
        for (bytes, fpi, calls, query) in statements.iter().take(12) {
            let text: String = query.chars().take(120).collect();
            writeln!(report, "  {bytes:>9} B {fpi:>4} fpi {calls:>5} calls  {text}")
                .expect("a String takes the line");
        }
        over |= per_entry > CEILING;
    }
    eprintln!("{report}");
    assert!(!over, "an entry exceeds its write-ahead log ceiling:\n{report}");
}
