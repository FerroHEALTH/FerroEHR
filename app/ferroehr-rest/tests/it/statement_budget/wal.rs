// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The write-ahead log the access log writes per entry (#5968): the bytes
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
/// over the statements that take, release and chain it, by kind of request:
/// the bytes measured when each entry was written once (#5968), 2% above.
/// Before, an entry written into the intake and again into the record cost
/// 5045, 4048, 3052 and 5328.
const CEILINGS: [(Kind, u64); 4] = [
    (Kind::Read, 3327),
    (Kind::Single, 2643),
    (Kind::Population, 2047),
    (Kind::Write, 3512),
];

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

/// Each kind of request's entries cost the access log at most its ceiling
/// in [`CEILINGS`] of write-ahead log each.
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
    for (kind, ceiling) in CEILINGS {
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
        let per_entry = ours.div_euclid(written);
        writeln!(
            report,
            "{}: {written} entries over {REQUESTS} requests; the access log wrote {ours} bytes, \
             {per_entry} per entry (ceiling {ceiling}); every statement {all} bytes, {} per request",
            kind.name(),
            all.div_euclid(REQUESTS),
        )
        .expect("a String takes the report");
        for (bytes, fpi, calls, query) in statements.iter().take(12) {
            let text: String = query.chars().take(120).collect();
            writeln!(
                report,
                "  {bytes:>9} B {fpi:>4} fpi {calls:>5} calls  {text}"
            )
            .expect("a String takes the line");
        }
        over |= per_entry > ceiling;
    }
    eprintln!("{report}");
    assert!(
        !over,
        "an entry exceeds its write-ahead log ceiling:\n{report}"
    );
}

/// The compositions measured: the IPS of 402 node rows, and the gate's lab
/// result of a few dozen.
#[derive(Clone, Copy)]
enum Fixture {
    /// The vendored IPS composition the harness's template validates.
    Ips,
    /// The CKM generic laboratory test result example.
    Lab,
}

impl Fixture {
    fn name(self) -> &'static str {
        match self {
            Self::Ips => "IPS commit",
            Self::Lab => "lab commit",
        }
    }

    fn body(self) -> String {
        match self {
            Self::Ips => crate::common::ips_canonical_composition().to_string(),
            Self::Lab => corpus("generic-lab-test-result.example.json"),
        }
    }
}

/// A file of the shared corpus's CKM templates.
fn corpus(file: &str) -> String {
    std::fs::read_to_string(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../corpus/templates/ckm")
            .join(file),
    )
    .expect("the CKM corpus file is vendored")
}

/// The most write-ahead log one composition commit may write, in bytes, over
/// every statement of the commit but the access log's, and the most
/// statements `PostgreSQL` may run nested in it that name a clinical relation
/// (foreign-key checks and triggers), by fixture: the bytes measured with the
/// lean node storage (#5969), 2% above, and its nested statements exactly.
/// Before, with `fk_node_version` and the full node indexes, an IPS commit
/// wrote 288,892 bytes and ran 410 nested statements, and a lab commit 11,502
/// bytes and 18.
const COMMIT_CEILINGS: [(Fixture, u64, u64); 2] =
    [(Fixture::Ips, 244_930, 8), (Fixture::Lab, 10_658, 8)];

/// Each composition commit writes at most its ceiling in [`COMMIT_CEILINGS`]
/// of write-ahead log and runs at most its nested statements (#5969).
#[tokio::test]
async fn a_composition_commit_stays_within_its_wal_ceiling() {
    let (_db, pool, _svc, app) = Box::pin(app()).await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_stat_statements")
        .execute(&pool)
        .await
        .expect("pg_stat_statements: the server loads it (shared_preload_libraries)");
    let (status, body) = send(
        &app,
        "POST",
        &format!("{BASE}/definition/template/adl1.4"),
        "root",
        None,
        Some(("application/xml", corpus("generic-lab-test-result.opt"))),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "lab OPT upload: {body}");
    let mut made = Vec::new();
    for _ in 0..EHRS {
        made.push(ehr_with_composition(&app).await);
    }
    let mut report = String::new();
    let mut over = false;
    for (fixture, ceiling, nested_ceiling) in COMMIT_CEILINGS {
        let body = fixture.body();
        let commit = |ehr: String| {
            let body = body.clone();
            let app = app.clone();
            async move {
                let (status, answer) = send(
                    &app,
                    "POST",
                    &format!("{BASE}/ehr/{ehr}/composition"),
                    "root",
                    None,
                    Some(("application/json", body)),
                )
                .await;
                assert_eq!(status, StatusCode::CREATED, "{}: {answer}", fixture.name());
            }
        };
        for (ehr, _) in &made {
            commit(ehr.clone()).await;
        }
        drained(&pool).await;
        sqlx::query(
            "SELECT pg_stat_statements_reset(0, \
             (SELECT oid FROM pg_database WHERE datname = current_database()), 0)",
        )
        .execute(&pool)
        .await
        .expect("the statement counts reset");
        for (_, (ehr, _)) in (0..REQUESTS).zip(made.iter().cycle()) {
            commit(ehr.clone()).await;
        }
        drained(&pool).await;
        let statements = logged(&pool).await;
        let commits: u64 = statements
            .iter()
            .filter(|(_, _, _, query)| !access_log(query))
            .map(|(bytes, ..)| bytes)
            .sum();
        let per_commit = commits.div_euclid(REQUESTS);
        let nested = nested_calls(&pool).await.div_euclid(REQUESTS);
        writeln!(
            report,
            "{}: {REQUESTS} commits wrote {commits} bytes outside the access log, {per_commit} \
             per commit (ceiling {ceiling}); {nested} nested statements per commit (ceiling \
             {nested_ceiling})",
            fixture.name(),
        )
        .expect("a String takes the report");
        for (bytes, fpi, calls, query) in statements.iter().take(12) {
            let text: String = query.chars().take(120).collect();
            writeln!(
                report,
                "  {bytes:>9} B {fpi:>4} fpi {calls:>5} calls  {text}"
            )
            .expect("a String takes the line");
        }
        for (calls, query) in nested_ledger(&pool).await.iter().take(8) {
            let text: String = query.chars().take(120).collect();
            writeln!(report, "  nested {calls:>5} calls  {text}").expect("a String takes the line");
        }
        over |= per_commit > ceiling || nested > nested_ceiling;
    }
    eprintln!("{report}");
    assert!(
        !over,
        "a commit exceeds its write-ahead log or nested-statement ceiling:\n{report}"
    );
}

/// The calls of the statements `PostgreSQL` ran nested in the test's
/// database's statements since the last reset that name a clinical relation:
/// the foreign-key checks and triggers of a commit, and none of the audit
/// schema's intake, release and chainer statements.
async fn nested_calls(pool: &PgPool) -> u64 {
    let calls: i64 = sqlx::query_scalar(
        "SELECT /* statement_budget_own */ coalesce(sum(calls), 0)::bigint \
         FROM pg_stat_statements \
         WHERE NOT toplevel AND dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         AND query LIKE '%\"clinical\".%'",
    )
    .fetch_one(pool)
    .await
    .expect("pg_stat_statements reads");
    u64::try_from(calls).expect("a count")
}

/// The nested statements since the last reset, most called first.
async fn nested_ledger(pool: &PgPool) -> Vec<(i64, String)> {
    sqlx::query_as(
        "SELECT /* statement_budget_own */ calls, regexp_replace(query, '\\s+', ' ', 'g') \
         FROM pg_stat_statements \
         WHERE NOT toplevel AND dbid = (SELECT oid FROM pg_database WHERE datname = current_database()) \
         ORDER BY calls DESC, query",
    )
    .fetch_all(pool)
    .await
    .expect("pg_stat_statements reads")
}
