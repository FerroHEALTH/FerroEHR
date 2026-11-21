// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The write-ahead log the access log writes per entry (#5968, #6010): the
//! row records of its relations, read with `pg_walinspect` for the test's
//! own database, so the other tests on the shared server do not count. An
//! entry's records and a chainer pass's records are held to separate
//! ceilings, so the count does not depend on how many passes the chainer
//! ran. The server runs without full-page writes, as the testkit container
//! and CI's server do. No openEHR spec governs audit storage: our own design.

use std::fmt::Write as _;

use axum::Router;
use http::StatusCode;
use sqlx::PgPool;

use super::{BASE, EHRS, drained, entries, population, single};
use crate::access_evaluator_http::{ehr_with_composition, send};

/// The requests of each kind measured.
const REQUESTS: u64 = 20;

/// The most write-ahead log the access log may write, by kind of request:
/// per entry, over the row records of the entry, its link, the intake queue
/// row that names it, the shared parts and their indexes, and per chainer
/// pass, over the access log's chain head. Each is the value measured with
/// `pg_walinspect`, 2% above: per entry 2746, 2196, 1876 and 2639 once the
/// row part leaves the request and the version to their columns and the
/// shared part keeps what the classification read (#6005), and 207 per pass
/// that links (#6010). The token index taking the request and resource ids
/// as keys in place of their two hash B-trees (#6202) measured 2634, 2087,
/// 1777 and 2543 per entry, and each entry ceiling fell by its drop. The
/// access-log write taking no row lock on a shared part (#6241) measured
/// 2580, 2033, 1759 and 2543 beside the merge base, and each entry ceiling
/// fell by its drop.
const CEILINGS: [(Kind, (u64, u64)); 4] = [
    (Kind::Read, (2651, 211)),
    (Kind::Single, (2081, 211)),
    (Kind::Population, (1797, 211)),
    (Kind::Write, (2596, 211)),
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

/// Refuses a server that keeps less than 1 GB of past WAL: a checkpoint during
/// a parallel run then removes a segment between a window's two positions,
/// and `pg_walinspect` cannot read it (`58P01`, #6035). The testkit container
/// and CI's server set it (`tools/testkit/src/server_settings.rs`,
/// `scripts/lib/test-postgres.sh`).
pub(super) async fn kept(pool: &PgPool) {
    let kept: i64 = sqlx::query_scalar(
        "SELECT /* statement_budget_own */ pg_size_bytes(current_setting('wal_keep_size'))",
    )
    .fetch_one(pool)
    .await
    .expect("wal_keep_size reads");
    assert!(
        kept >= 1 << 30,
        "the test server keeps {kept} bytes of past WAL, so a checkpoint can remove a segment a \
         WAL budget reads: start it with wal_keep_size=1GB (remove a testkit container started \
         before #6035, `docker rm -f -v ferroehr-testkit-pg19`, and the next run starts one that \
         does)"
    );
}

/// The current WAL insert position, as text.
pub(super) async fn insert_lsn(pool: &PgPool) -> String {
    sqlx::query_scalar("SELECT /* statement_budget_own */ pg_current_wal_insert_lsn()::text")
        .fetch_one(pool)
        .await
        .expect("the insert position reads")
}

/// Waits until the server has flushed its WAL past `to`: `pg_walinspect` reads
/// only flushed WAL, and the servers commit asynchronously, so their WAL
/// writer flushes it shortly after.
pub(super) async fn flushed(pool: &PgPool, to: &str) {
    for _ in 0..400 {
        let past: bool = sqlx::query_scalar(
            "SELECT /* statement_budget_own */ pg_current_wal_flush_lsn() >= $1::pg_lsn",
        )
        .bind(to)
        .fetch_one(pool)
        .await
        .expect("the flush position reads");
        if past {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    panic!("the server did not flush its write-ahead log");
}

/// The row records between `from` and `to` that touch a relation of the
/// test's database outside the system catalogs, by relation: name, record
/// bytes and records.
///
/// A record counts only when it names a relation the database's own catalog
/// holds, a table, index or TOAST table of a schema of the server's: the
/// statistics and catalog rows autovacuum and autoanalyze write when a
/// parallel run's timing puts them in the window are left out, as are the
/// records of the shared catalogs and of every other database. A TOAST
/// table's index belongs to the schema of the table the TOAST table serves,
/// so the index of `pg_statistic`'s is a catalog relation too: an autoanalyze
/// in the window wrote 770 bytes through it, the figure a path's budget was
/// over by on CI (#6035).
///
/// Only the records a statement writes for each row count: a heap insert,
/// update, delete or lock, a B-tree leaf insert and a GIN pending-list insert.
/// Page maintenance (pruning, splits, deduplication, a new pending-list page,
/// a hint-bit image, a sequence's prefetch) falls on whichever row first
/// crosses a page boundary, which the random ids and the chainer's timing
/// move between runs, so it is left out.
pub(super) async fn by_relation(pool: &PgPool, from: &str, to: &str) -> Vec<(String, u64, u64)> {
    let rows: Vec<(String, i64, i64)> = sqlx::query_as(
        "SELECT /* statement_budget_own */ c.relname, \
                sum(b.record_length)::bigint, count(*) \
           FROM (SELECT DISTINCT ON (start_lsn, relfilenode) start_lsn, relfilenode, record_length \
                   FROM pg_get_wal_block_info($1::pg_lsn, $2::pg_lsn) \
                  WHERE reldatabase = (SELECT oid FROM pg_database WHERE datname = current_database()) \
                    AND ((resource_manager = 'Heap' \
                          AND split_part(record_type, '+', 1) \
                              IN ('INSERT', 'UPDATE', 'HOT_UPDATE', 'DELETE', 'LOCK')) \
                      OR (resource_manager = 'Heap2' AND record_type LIKE 'MULTI_INSERT%') \
                      OR (resource_manager = 'Btree' AND record_type = 'INSERT_LEAF') \
                      OR (resource_manager = 'Gin' AND record_type = 'UPDATE_META_PAGE'))) b \
           JOIN pg_class c ON pg_relation_filenode(c.oid) = b.relfilenode \
           LEFT JOIN pg_index i ON i.indexrelid = c.oid \
           LEFT JOIN pg_class owner ON owner.reltoastrelid = coalesce(i.indrelid, c.oid) \
          WHERE coalesce(owner.relnamespace, c.relnamespace) NOT IN \
                ('pg_catalog'::regnamespace, 'information_schema'::regnamespace) \
          GROUP BY 1 ORDER BY 2 DESC, 1",
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await
    .expect("pg_walinspect reads");
    rows.into_iter()
        .map(|(name, bytes, records)| {
            (
                name,
                u64::try_from(bytes).expect("a count"),
                u64::try_from(records).expect("a count"),
            )
        })
        .collect()
}

/// Whether `relation` holds what an entry costs once: the entry, its link,
/// the intake queue row that names it, the shared parts, their indexes and
/// sequences.
fn entry_relation(relation: &str) -> bool {
    !pass_relation(relation)
        && [
            "access_log_",
            "ix_access_log_",
            "pk_access_log_",
            "uq_access_log_",
        ]
        .iter()
        .any(|prefix| relation.starts_with(prefix))
}

/// Whether `relation` is the access log's chain head, which a chainer pass
/// that links entries updates once whatever their number.
pub(super) fn pass_relation(relation: &str) -> bool {
    relation == "access_log_chain_state"
}

/// The chainer passes between `from` and `to`: the transactions that wrote a
/// row record of the access log's chain head in the test's database, each
/// pass one transaction that locks the head and, when it links entries,
/// moves it (#6010).
///
/// They are read from the window's own WAL, as the bytes are: a count of
/// `pg_stat_statements` calls is evicted with the other entries of a
/// statement called few times once a full parallel run fills the shared
/// server's statement table (`pg_stat_statements.max`), and counts passes
/// that ran after the window too.
async fn passes(pool: &PgPool, from: &str, to: &str) -> u64 {
    let passes: i64 = sqlx::query_scalar(
        "SELECT /* statement_budget_own */ count(DISTINCT xid::text) \
           FROM pg_get_wal_block_info($1::pg_lsn, $2::pg_lsn) \
          WHERE reldatabase = (SELECT oid FROM pg_database WHERE datname = current_database()) \
            AND relfilenode = pg_relation_filenode('audit.access_log_chain_state') \
            AND resource_manager = 'Heap' \
            AND split_part(record_type, '+', 1) \
                IN ('INSERT', 'UPDATE', 'HOT_UPDATE', 'DELETE', 'LOCK')",
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await
    .expect("pg_walinspect reads");
    u64::try_from(passes).expect("a count")
}

/// Each kind of request's entries cost the access log at most their per-entry
/// ceiling in [`CEILINGS`], and each chainer pass at most its per-pass one.
#[tokio::test]
async fn an_access_log_entry_stays_within_its_wal_ceiling() {
    // The budgets count what a release build does, without the debug reference cells.
    ferroehr::aql::sql::without_reference_cells();
    let (_db, pool, _svc, app) = Box::pin(super::served()).await;
    sqlx::query("CREATE EXTENSION IF NOT EXISTS pg_walinspect")
        .execute(&pool)
        .await
        .expect("pg_walinspect: a contrib module the server ships");
    kept(&pool).await;
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
    for (kind, (entry_ceiling, pass_ceiling)) in CEILINGS {
        for (ehr, vo) in &made {
            kind.send(&app, ehr, vo).await;
        }
        drained(&pool).await;
        let before = entries(&pool).await;
        // The shared server's other tests write WAL fast enough that a checkpoint may recycle
        // the window's segments before they are read; a temporary slot keeps them.
        let mut keeper = ferroehr::db::acquire::acquire(&pool)
            .await
            .expect("a connection for the slot");
        sqlx::query(
            "SELECT /* statement_budget_own */ pg_create_physical_replication_slot( \
             'statement_budget_wal_' || pg_backend_pid(), true, true)",
        )
        .execute(&mut *keeper)
        .await
        .expect("a temporary slot keeps the window's WAL");
        let from = insert_lsn(&pool).await;
        for (_, (ehr, vo)) in (0..REQUESTS).zip(made.iter().cycle()) {
            kind.send(&app, ehr, vo).await;
        }
        drained(&pool).await;
        let to = insert_lsn(&pool).await;
        flushed(&pool, &to).await;
        let written = entries(&pool).await - before;
        assert!(written > 0, "{}: the requests write entries", kind.name());
        let passes = passes(&pool, &from, &to).await;
        assert!(passes > 0, "{}: the chainer ran", kind.name());
        let relations = by_relation(&pool, &from, &to).await;
        sqlx::query(
            "SELECT /* statement_budget_own */ pg_drop_replication_slot( \
             'statement_budget_wal_' || pg_backend_pid())",
        )
        .execute(&mut *keeper)
        .await
        .expect("the slot drops");
        drop(keeper);
        let bytes_of = |of: fn(&str) -> bool| -> u64 {
            relations
                .iter()
                .filter(|(name, ..)| of(name))
                .map(|(_, bytes, _)| bytes)
                .sum()
        };
        let per_entry = bytes_of(entry_relation).div_euclid(written);
        let per_pass = bytes_of(pass_relation).div_euclid(passes);
        writeln!(
            report,
            "{}: {written} entries over {REQUESTS} requests and {passes} chainer passes: \
             {per_entry} B per entry (ceiling {entry_ceiling}), {per_pass} B per pass \
             (ceiling {pass_ceiling})",
            kind.name(),
        )
        .expect("a String takes the report");
        for (name, bytes, records) in relations
            .iter()
            .filter(|(name, ..)| entry_relation(name) || pass_relation(name))
        {
            writeln!(report, "  {bytes:>9} B {records:>5} records  {name}")
                .expect("a String takes the line");
        }
        over |= per_entry > entry_ceiling || per_pass > pass_ceiling;
    }
    eprintln!("{report}");
    assert!(
        !over,
        "the access log exceeds a write-ahead log ceiling:\n{report}"
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

/// The most write-ahead log one composition commit may write, in bytes of
/// row records over the relations of the commit (everything but the access
/// log's and the audit intake's), and the most statements `PostgreSQL` may
/// run nested in it that name a clinical relation (foreign-key checks and
/// triggers), by fixture: the bytes measured with the lean node storage
/// (#5969), 2% above, and its nested statements exactly. With
/// `fk_node_version` and the full node indexes an IPS commit measured
/// 278,420 bytes and 410 nested statements, and a lab commit 10,667 bytes and
/// 18; with the lean storage 233,279 and 9,827. The `event_time` column with
/// its index and the magnitude index (#6022) brought them to 237,374 and
/// 10,092: the IPS ceiling holds, and the lab ceiling is that measurement, 2%
/// above, since two of a lab commit's ten node rows are an event and a
/// history (the decision of 2026-11-18 on #6022). The index on the code of an
/// ELEMENT value (#6033) adds 36 entries of 72 bytes to an IPS commit, which
/// measured 239,966: its ceiling is that measurement, 2% above, the rise that
/// issue's 3% allows and the orchestrator accepted on 2026-11-19. A lab
/// commit carries no coded ELEMENT value and keeps its ceiling. The version
/// tree uniqueness has since served the read by version id in place of a
/// separate index (#6207): 96 bytes fewer on each commit, and both ceilings
/// fell by it. A node row has since stored its own path step, and the node
/// key carries no tier (#6204): 13,650 bytes fewer on an IPS commit and 255
/// on a lab commit, and the ceilings fell by as much. A member repeated
/// across a version's node rows has since been stored once (#6229): 42,162
/// bytes fewer on an IPS commit, whose entries repeat one participation list,
/// and its ceiling fell by as much; a lab commit repeats none and keeps its
/// ceiling.
const COMMIT_CEILINGS: [(Fixture, u64, u64); 2] =
    [(Fixture::Ips, 188_857, 8), (Fixture::Lab, 9942, 8)];

/// Whether `relation` is written by a commit rather than by the access log or
/// the audit intake beside it.
fn commit_relation(relation: &str) -> bool {
    !entry_relation(relation) && !pass_relation(relation) && !relation.contains("audit_")
}

/// Each composition commit writes at most its ceiling in [`COMMIT_CEILINGS`]
/// of write-ahead log and runs at most its nested statements (#5969).
#[tokio::test]
async fn a_composition_commit_stays_within_its_wal_ceiling() {
    // The budgets count what a release build does, without the debug reference cells.
    ferroehr::aql::sql::without_reference_cells();
    let (_db, pool, _svc, app) = Box::pin(super::served()).await;
    let made = commit_fixture(&app, &pool).await;
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
        let (relations, evicted) = commit_window(&pool, async {
            for (_, (ehr, _)) in (0..REQUESTS).zip(made.iter().cycle()) {
                commit(ehr.clone()).await;
            }
        })
        .await;
        let commits: u64 = relations
            .iter()
            .filter(|(name, ..)| commit_relation(name))
            .map(|(_, bytes, _)| bytes)
            .sum();
        let per_commit = commits.div_euclid(REQUESTS);
        let nested = nested_calls(&pool).await.div_euclid(REQUESTS);
        testkit::statements::assert_statements_kept(&pool, evicted)
            .await
            .expect("no statement evicted while the test counted them");
        writeln!(
            report,
            "{}: {REQUESTS} commits wrote {commits} bytes of row records, {per_commit} per \
             commit (ceiling {ceiling}); {nested} nested statements per commit (ceiling \
             {nested_ceiling})",
            fixture.name(),
        )
        .expect("a String takes the report");
        for (name, bytes, records) in relations
            .iter()
            .filter(|(name, ..)| commit_relation(name))
            .take(12)
        {
            writeln!(report, "  {bytes:>9} B {records:>6} records  {name}")
                .expect("a String takes the line");
        }
        over |= per_commit > ceiling || nested > nested_ceiling;
    }
    eprintln!("{report}");
    assert!(
        !over,
        "a commit exceeds its write-ahead log or nested-statement ceiling:\n{report}"
    );
}

/// Loads the measuring extensions, uploads the lab template and makes the
/// [`EHRS`] EHRs the commits go to, each with a composition.
async fn commit_fixture(app: &Router, pool: &PgPool) -> Vec<(String, String)> {
    for extension in ["pg_stat_statements", "pg_walinspect"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "CREATE EXTENSION IF NOT EXISTS {extension}"
        )))
        .execute(pool)
        .await
        .expect("the server loads the extension");
    }
    kept(pool).await;
    let (status, body) = send(
        app,
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
        made.push(ehr_with_composition(app).await);
    }
    made
}

/// The write-ahead log by relation that `commits` writes, with the
/// statement counts reset before it and a temporary slot keeping the
/// window's WAL, and the statement table's evictions when the count began.
async fn commit_window(
    pool: &PgPool,
    commits: impl Future<Output = ()>,
) -> (Vec<(String, u64, u64)>, i64) {
    drained(pool).await;
    let evicted = testkit::statements::statement_window(pool)
        .await
        .expect("the statement counts reset");
    let mut keeper = ferroehr::db::acquire::acquire(pool)
        .await
        .expect("a connection for the slot");
    sqlx::query(
        "SELECT /* statement_budget_own */ pg_create_physical_replication_slot( \
         'statement_budget_wal_' || pg_backend_pid(), true, true)",
    )
    .execute(&mut *keeper)
    .await
    .expect("a temporary slot keeps the window's WAL");
    let from = insert_lsn(pool).await;
    commits.await;
    drained(pool).await;
    let to = insert_lsn(pool).await;
    flushed(pool, &to).await;
    let relations = by_relation(pool, &from, &to).await;
    sqlx::query(
        "SELECT /* statement_budget_own */ pg_drop_replication_slot( \
         'statement_budget_wal_' || pg_backend_pid())",
    )
    .execute(&mut *keeper)
    .await
    .expect("the slot drops");
    drop(keeper);
    (relations, evicted)
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
