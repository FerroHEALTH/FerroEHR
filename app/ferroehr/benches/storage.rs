// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Criterion benches over the storage layer's hot paths against a real
//! `PostgreSQL` 18 database, with per-bench CPU flamegraphs and a JSON record
//! carrying the database-side facts beside the wall-clock.
//!
//! The operations are driven through [`FerroEhrService`] and the
//! `ferroehr::extensions` public surface only, and the database-side facts are
//! read for whatever relations the catalogue reports in the pool's
//! `search_path`. Nothing here names a table, a column or an index, so the same
//! harness measures a rewritten schema unchanged.
//!
//! **A benchmark, never a conformance record** — conformance is the CNF suite
//! Veredictum runs — and no openEHR spec governs storage mechanics or
//! benchmarking: our own design/extension.
//!
//! ```text
//! STORAGE_BENCH_CLASS=poc cargo bench -p ferroehr --bench storage
//! cargo bench -p ferroehr --bench storage -- --profile-time 10
//! ```
//!
//! The record lands in `docs/benchmarks/storage/<generation>/record.json`; its
//! shape is documented in that directory's README. Under `--profile-time`
//! criterion measures nothing, so no record is written and an existing one is
//! left intact; a run that measured only some of its operations says so in the
//! record's `timings_measured` flag.
//!
//! NOTE: the profiler glue below implements criterion's `Profiler` trait over
//! the `pprof` sampler directly, because pprof's own `criterion` feature is
//! pinned to criterion ^0.5 (verified on crates.io 2026-08-04) and this
//! workspace is on criterion 0.8.

#![expect(
    clippy::expect_used,
    reason = "bench fixture setup, database acquisition and profiler I/O: a \
              failed seed or an unwritable profile directory must abort the \
              bench loudly (there is no caller to return an error to)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the record this harness writes is a \
              JSON artifact, and the service read seams it times serve canonical \
              JSON bodies"
)]
#![expect(
    clippy::print_stderr,
    clippy::print_stdout,
    reason = "the bench harness is a dev binary; the seed progress and the \
              profiler lifecycle go to the operator on stdio (no tracing \
              subscriber is installed)"
)]

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use criterion::measurement::WallTime;
use criterion::profiler::Profiler;
use criterion::{
    Bencher, BenchmarkGroup, Criterion, SamplingMode, criterion_group, criterion_main,
};
use futures::StreamExt as _;
use serde_json::{Value, json};
use sqlx::{PgPool, Row as _};
use tokio::runtime::Runtime;

use ferroehr::aql::ir::Params;
use ferroehr::aql::lineage::ArchetypeLineage;
use ferroehr::aql::sql::SqlCtx;
use ferroehr::config::profile::SpecProfile;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_base::prelude::ObjectVersionId;
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
use openehr_rm::prelude::{Composition, PartyProxy};

/// The vendored CKM template the seed commits against: an encounter carrying
/// eight OBSERVATION archetypes, so a CONTAINS chain has a real ENTRY to find.
const SEED_TEMPLATE: &str = "vital-signs";

/// The identity every seeded version is composed and committed by.
const BENCH_PARTY: &str = "storage bench";

/// `audit_change_type` group codes (RM common master06 §Contributions).
const CREATION: &str = "249";
/// The `audit_change_type` modification code.
const MODIFICATION: &str = "251";

/// How many seed commits run at once: the work is I/O-bound on `PostgreSQL`,
/// so the width keeps the pool busy rather than the CPU.
const SEED_CONCURRENCY: usize = 16;

/// The retention window the prune bench asks for, in days.
const PRUNE_RETENTION_DAYS: i64 = 7;

/// How long a counter read waits for the pool's backends to publish.
///
/// NOTE: `PostgreSQL` docs §Cumulative Statistics System — a server process
/// "transmits new counts to shared memory just before going idle, but not more
/// frequently than once per `PGSTAT_MIN_INTERVAL`"; a shorter window leaves
/// the tail of one operation pending and attributes it to the next, which is
/// how a single probed commit reads back as having touched nothing at all.
const STATS_SETTLE: Duration = Duration::from_secs(11);

// ── the seed ────────────────────────────────────────────────────────────────

/// The corpus size one bench class seeds.
#[derive(Clone, Copy)]
struct BenchClass {
    /// The class token the record carries.
    name: &'static str,
    /// How many EHRs the corpus holds.
    ehrs: usize,
    /// How many compositions are spread over them.
    compositions: usize,
}

impl BenchClass {
    /// The class `STORAGE_BENCH_CLASS` selects, `poc` by default.
    fn from_env() -> Self {
        // A bench knob, not server configuration: it sizes the corpus this
        // harness seeds and never reaches a running server.
        #[expect(
            clippy::disallowed_methods,
            reason = "`STORAGE_BENCH_CLASS` is a bench knob read by the harness itself; \
                      the config tree governs the server, which this never touches"
        )]
        let raw = std::env::var("STORAGE_BENCH_CLASS").unwrap_or_default();
        match raw.as_str() {
            "s" | "S" => Self {
                name: "s",
                ehrs: 100,
                compositions: 3_000,
            },
            _ => Self {
                name: "poc",
                ehrs: 20,
                compositions: 300,
            },
        }
    }
}

/// One seeded composition kept for the read benches.
struct Seeded {
    /// The EHR it lives in.
    ehr: EhrId,
    /// Its versioned-object id.
    vo: VoId,
    /// The `OBJECT_VERSION_ID` of its first version (superseded during the
    /// seed, so a point read by version uid reads a non-current version).
    first_version_uid: String,
}

/// Everything the benches read off one seeded corpus.
struct Corpus {
    /// The EHRs the corpus spans.
    ehrs: Vec<EhrId>,
    /// The example COMPOSITION every commit writes.
    payload: Value,
    /// The template the payload is bound to.
    template_id: String,
    /// The ENTRY RM type the payload carries, which the CONTAINS chain targets.
    entry_rm_type: String,
    /// One kept composition per EHR, each with two versions.
    kept: Vec<Seeded>,
    /// An instant after the seed, for `version_at_time`.
    at: String,
    /// How long seeding took.
    seconds: f64,
}

/// The vendored corpus file for the seed template.
fn corpus_file(extension: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!(
        "../../corpus/templates/ckm/{SEED_TEMPLATE}.{extension}"
    ))
}

/// A `PARTY_IDENTIFIED` proxy named [`BENCH_PARTY`].
fn committer() -> PartyProxy {
    openehr_its::json::from_canonical_value(&json!({
        "_type": "PARTY_IDENTIFIED", "name": BENCH_PARTY
    }))
    .expect("the bench committer decodes as a PARTY_PROXY")
}

/// The SM `UPDATE_VERSION` envelope carrying `data`, superseding `preceding`.
fn update_version(
    data: &Value,
    change_code: &str,
    preceding: Option<&str>,
) -> UpdateVersion<Composition> {
    UpdateVersion {
        preceding_version_uid: preceding.map(|raw| {
            raw.parse()
                .expect("the preceding uid is an OBJECT_VERSION_ID")
        }),
        lifecycle_state: lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the seed payload decodes as a COMPOSITION"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded(change_code),
            description: None,
            committer: committer(),
            additional_properties: BTreeMap::new(),
        }),
        signature: None,
        additional_properties: BTreeMap::new(),
    }
}

/// The first ENTRY RM type in a composition's content, `OBSERVATION` when the
/// payload carries none.
fn entry_rm_type(payload: &Value) -> String {
    payload
        .get("content")
        .and_then(Value::as_array)
        .and_then(|content| {
            content.iter().find_map(|item| {
                item.get("_type")
                    .and_then(Value::as_str)
                    .filter(|ty| {
                        matches!(
                            *ty,
                            "OBSERVATION" | "EVALUATION" | "INSTRUCTION" | "ACTION" | "ADMIN_ENTRY"
                        )
                    })
                    .map(str::to_owned)
            })
        })
        .unwrap_or_else(|| "OBSERVATION".to_owned())
}

/// Seed one corpus: the vendored template, `class.ehrs` EHRs, and
/// `class.compositions` commits of the template's own example composition
/// spread over them, one per EHR kept and superseded for the read benches.
///
/// The single seam the synthgen corpus replaces — everything below reads only
/// the [`Corpus`] this returns.
async fn seed(svc: &Arc<FerroEhrService>, class: BenchClass) -> Corpus {
    let start = Instant::now();
    let xml = std::fs::read_to_string(corpus_file("opt")).expect("the vendored OPT is readable");
    let template_id = openehr_its::opt14::from_xml(&xml)
        .expect("the vendored OPT parses")
        .template_id
        .value;
    svc.upload_opt(xml).await.expect("upload the seed template");
    // `medium` is the generator's fully-populated, committable level.
    let payload = svc
        .template_adl14_example(template_id.clone(), Some("medium".to_owned()), None)
        .await
        .expect("the template generates an example composition");

    let ehrs: Vec<EhrId> = futures::stream::iter(0..class.ehrs)
        .map(|_| {
            let svc = Arc::clone(svc);
            async move { svc.create_ehr(None).await.expect("create_ehr") }
        })
        .buffer_unordered(SEED_CONCURRENCY)
        .collect()
        .await;

    let commits: Vec<usize> = (0..class.compositions).collect();
    futures::stream::iter(commits)
        .map(|index| {
            let svc = Arc::clone(svc);
            let payload = payload.clone();
            let ehr = *ehrs.get(index % ehrs.len()).expect("the corpus holds EHRs");
            async move {
                svc.create_composition(ehr, update_version(&payload, CREATION, None))
                    .await
                    .expect("seed a composition");
            }
        })
        .buffer_unordered(SEED_CONCURRENCY)
        .collect::<Vec<()>>()
        .await;

    // One kept composition per EHR, superseded once so the version chain the
    // read benches walk has more than a single entry.
    let kept: Vec<Seeded> = futures::stream::iter(ehrs.clone())
        .map(|ehr| {
            let svc = Arc::clone(svc);
            let payload = payload.clone();
            async move {
                let first = svc
                    .create_composition(ehr, update_version(&payload, CREATION, None))
                    .await
                    .expect("seed the kept composition");
                let first_version_uid = first.version_uid();
                svc.update_composition(
                    ehr,
                    first.vo_id,
                    update_version(&payload, MODIFICATION, Some(&first_version_uid)),
                )
                .await
                .expect("supersede the kept composition");
                Seeded {
                    ehr,
                    vo: first.vo_id,
                    first_version_uid,
                }
            }
        })
        .buffer_unordered(SEED_CONCURRENCY)
        .collect()
        .await;

    let seconds = start.elapsed().as_secs_f64();
    println!(
        "seeded {} EHRs and {} compositions in {seconds:.1}s",
        ehrs.len(),
        class.compositions + kept.len()
    );
    Corpus {
        entry_rm_type: entry_rm_type(&payload),
        at: jiff::Timestamp::now().to_string(),
        ehrs,
        payload,
        template_id,
        kept,
        seconds,
    }
}

// ── the database-side facts ─────────────────────────────────────────────────

/// The cumulative counters one relation carries at a snapshot.
#[derive(Clone)]
struct TableStats {
    /// Rows inserted since the database was created.
    ins: i64,
    /// Rows updated.
    upd: i64,
    /// Of those, updated HOT (no index entry written).
    hot_upd: i64,
    /// Rows deleted.
    del: i64,
    /// The planner's live-row estimate.
    live: i64,
    /// The planner's dead-row estimate.
    dead: i64,
    /// `pg_total_relation_size`: heap, indexes and TOAST together.
    bytes: i64,
}

/// One point-in-time read of the instruments the `PostgreSQL` docs name.
struct Snapshot {
    /// Per-relation counters, keyed by `schema.relation`.
    tables: BTreeMap<String, TableStats>,
    /// Buffer hits for this database.
    blks_hit: i64,
    /// Buffer reads that missed shared buffers.
    blks_read: i64,
    /// Committed transactions.
    xact_commit: i64,
    /// The write-ahead-log insert position, when the role may read it.
    wal_lsn: Option<String>,
}

/// Read `pg_stat_user_tables`, `pg_stat_database` and the WAL position for
/// every relation in the pool's own `search_path`.
///
/// The relations are discovered through the catalogue rather than named, which
/// is what lets one harness measure two schema generations.
async fn snapshot(pool: &PgPool) -> Snapshot {
    tokio::time::sleep(STATS_SETTLE).await;
    // A session reuses the statistics snapshot it first read, so the cache is
    // dropped before every read (PostgreSQL docs §Cumulative Statistics System).
    drop(
        sqlx::query("SELECT pg_stat_clear_snapshot()")
            .execute(pool)
            .await,
    );
    let rows = sqlx::query(
        "SELECT schemaname, relname, n_tup_ins, n_tup_upd, n_tup_hot_upd, n_tup_del, \
                n_live_tup, n_dead_tup, pg_total_relation_size(relid) AS total_bytes \
           FROM pg_stat_user_tables \
          WHERE schemaname = ANY (current_schemas(false)) \
          ORDER BY schemaname, relname",
    )
    .fetch_all(pool)
    .await
    .expect("read pg_stat_user_tables");
    let mut tables = BTreeMap::new();
    for row in rows {
        let schema: String = row.try_get("schemaname").expect("schemaname");
        let relname: String = row.try_get("relname").expect("relname");
        tables.insert(
            format!("{schema}.{relname}"),
            TableStats {
                ins: row.try_get("n_tup_ins").unwrap_or_default(),
                upd: row.try_get("n_tup_upd").unwrap_or_default(),
                hot_upd: row.try_get("n_tup_hot_upd").unwrap_or_default(),
                del: row.try_get("n_tup_del").unwrap_or_default(),
                live: row.try_get("n_live_tup").unwrap_or_default(),
                dead: row.try_get("n_dead_tup").unwrap_or_default(),
                bytes: row.try_get("total_bytes").unwrap_or_default(),
            },
        );
    }
    let db = sqlx::query(
        "SELECT blks_hit, blks_read, xact_commit FROM pg_stat_database \
          WHERE datname = current_database()",
    )
    .fetch_one(pool)
    .await
    .expect("read pg_stat_database");
    Snapshot {
        tables,
        blks_hit: db.try_get("blks_hit").unwrap_or_default(),
        blks_read: db.try_get("blks_read").unwrap_or_default(),
        xact_commit: db.try_get("xact_commit").unwrap_or_default(),
        // NOTE: PostgreSQL docs §Backup Control Functions — reading the WAL
        // position may be refused for an unprivileged role, which the record
        // reports as an absent figure rather than a zero.
        wal_lsn: sqlx::query_scalar::<_, String>("SELECT pg_current_wal_lsn()::text")
            .fetch_one(pool)
            .await
            .ok(),
    }
}

/// The facts one operation produced: per-relation deltas plus the absolutes
/// that only make sense at the end of it.
async fn facts(pool: &PgPool, before: &Snapshot) -> Value {
    let after = snapshot(pool).await;
    let mut tables = Vec::new();
    for (name, stats) in &after.tables {
        let base = before.tables.get(name);
        let delta = |pick: fn(&TableStats) -> i64| pick(stats) - base.map_or(0, pick);
        // A relation nothing touched in this operation is noise in the record.
        if delta(|s| s.ins) == 0 && delta(|s| s.upd) == 0 && delta(|s| s.del) == 0 {
            continue;
        }
        tables.push(json!({
            "relation": name,
            "n_tup_ins_delta": delta(|s| s.ins),
            "n_tup_upd_delta": delta(|s| s.upd),
            "n_tup_hot_upd_delta": delta(|s| s.hot_upd),
            "n_tup_del_delta": delta(|s| s.del),
            "n_live_tup": stats.live,
            "n_dead_tup": stats.dead,
            "total_bytes": stats.bytes,
        }));
    }
    let wal_bytes = wal_delta(pool, before.wal_lsn.as_deref(), after.wal_lsn.as_deref()).await;
    json!({
        "tables": tables,
        "blks_hit_delta": after.blks_hit - before.blks_hit,
        "blks_read_delta": after.blks_read - before.blks_read,
        "xact_commit_delta": after.xact_commit - before.xact_commit,
        "wal_bytes": wal_bytes,
    })
}

/// Every relation the run put rows into, with its size, its live and dead
/// tuple estimates, and the exact dead-tuple percentage `pgstattuple` reads
/// off the heap at the end of the run.
async fn relations(pool: &PgPool) -> Value {
    let now = snapshot(pool).await;
    let exact = dead_tuple_percent(pool).await;
    let rows: Vec<Value> = now
        .tables
        .iter()
        .filter(|(_, stats)| stats.ins > 0)
        .map(|(name, stats)| {
            json!({
                "relation": name,
                "n_live_tup": stats.live,
                "n_dead_tup": stats.dead,
                "n_tup_upd": stats.upd,
                "n_tup_hot_upd": stats.hot_upd,
                "total_bytes": stats.bytes,
                "dead_tuple_percent": exact.get(name).cloned().unwrap_or(Value::Null),
            })
        })
        .collect();
    Value::Array(rows)
}

/// The exact dead-tuple percentage per plain relation from `pgstattuple`
/// (`PostgreSQL` docs, Additional Supplied Modules §pgstattuple), keyed by
/// `schema.relation`; empty when the extension cannot be created.
///
/// NOTE: `pgstattuple` scans the heap and refuses partitioned parents, so only
/// `relkind = 'r'` relations are read; the estimates beside it stay for the rest.
async fn dead_tuple_percent(pool: &PgPool) -> BTreeMap<String, Value> {
    let mut out = BTreeMap::new();
    if sqlx::query("CREATE EXTENSION IF NOT EXISTS pgstattuple")
        .execute(pool)
        .await
        .is_err()
    {
        return out;
    }
    let rows = sqlx::query(
        "SELECT s.schemaname, s.relname, (pgstattuple(s.relid)).dead_tuple_percent AS pct \
           FROM pg_stat_user_tables s JOIN pg_class c ON c.oid = s.relid \
          WHERE s.schemaname = ANY (current_schemas(false)) AND c.relkind = 'r' AND s.n_tup_ins > 0",
    )
    .fetch_all(pool)
    .await
    .unwrap_or_default();
    for row in rows {
        let schema: String = row.try_get("schemaname").unwrap_or_default();
        let relname: String = row.try_get("relname").unwrap_or_default();
        let pct: Option<f64> = row.try_get("pct").ok();
        out.insert(
            format!("{schema}.{relname}"),
            pct.map_or(Value::Null, Value::from),
        );
    }
    out
}

/// The WAL bytes written between two positions, absent when the role could not
/// read them.
async fn wal_delta(pool: &PgPool, before: Option<&str>, after: Option<&str>) -> Value {
    let (Some(before), Some(after)) = (before, after) else {
        return Value::Null;
    };
    sqlx::query_scalar::<_, i64>("SELECT pg_wal_lsn_diff($1::pg_lsn, $2::pg_lsn)::bigint")
        .bind(after)
        .bind(before)
        .fetch_one(pool)
        .await
        .ok()
        .map_or(Value::Null, Value::from)
}

/// What one representative commit costs the database: the WAL it writes and
/// the buffers it touches, around a single first-version commit.
///
/// NOTE: the commit runs inside the service's own transaction, so its cost is
/// attributed by the WAL position and the buffer counters rather than by
/// `EXPLAIN (ANALYZE, WAL)` of a statement the public API never hands out.
async fn commit_probe(svc: &FerroEhrService, pool: &PgPool, corpus: &Corpus) -> Value {
    let ehr = *corpus.ehrs.first().expect("the corpus holds an EHR");
    let before = snapshot(pool).await;
    svc.create_composition(ehr, update_version(&corpus.payload, CREATION, None))
        .await
        .expect("the probed commit");
    json!({
        "operation": "create_composition",
        "commits": 1,
        "database": facts(pool, &before).await,
    })
}

/// `EXPLAIN (ANALYZE, BUFFERS, WAL, FORMAT JSON)` of the population CONTAINS
/// statement, inside a transaction that is rolled back
/// (`PostgreSQL` docs §EXPLAIN, "ANALYZE … use it inside a transaction and
/// roll back afterwards").
async fn explain_population(pool: &PgPool, corpus: &Corpus) -> Value {
    let aql = population_aql(corpus);
    let ast = openehr_query::parser::parse_str(&aql).expect("the population AQL parses");
    let params = Params::new();
    let ir = ferroehr::aql::plan(&ast, &params, SpecProfile::default())
        .expect("the population AQL plans");
    let ctx = SqlCtx {
        system_id: "bench.ferroehr.org".to_owned(),
        ehr_ids: Vec::new(),
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    let prepared =
        ferroehr::aql::sql::build(&ir, &params, &ctx).expect("the population AQL lowers to SQL");
    let mut tx = pool.begin().await.expect("begin the explain transaction");
    let plan: Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!(
            "EXPLAIN (ANALYZE, BUFFERS, WAL, FORMAT JSON) {}",
            prepared.sql
        )),
        prepared.values,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("explain the population statement");
    tx.rollback().await.expect("roll the explain back");
    summarise_plan(&aql, &plan)
}

/// The top plan node's shape, cost and indexes — a whole plan is hundreds of
/// lines and goes stale as noise.
fn summarise_plan(aql: &str, explain: &Value) -> Value {
    let plan = explain
        .get(0)
        .and_then(|first| first.get("Plan"))
        .unwrap_or(&Value::Null);
    let mut indexes = Vec::new();
    collect_indexes(plan, &mut indexes);
    indexes.sort();
    indexes.dedup();
    json!({
        "statement": aql,
        "node_type": plan.get("Node Type").cloned().unwrap_or(Value::Null),
        "actual_total_ms": plan.get("Actual Total Time").cloned().unwrap_or(Value::Null),
        "rows": plan.get("Actual Rows").cloned().unwrap_or(Value::Null),
        "shared_hit_blocks": plan.get("Shared Hit Blocks").cloned().unwrap_or(Value::Null),
        "shared_read_blocks": plan.get("Shared Read Blocks").cloned().unwrap_or(Value::Null),
        "wal_bytes": plan.get("WAL Bytes").cloned().unwrap_or(Value::Null),
        "wal_records": plan.get("WAL Records").cloned().unwrap_or(Value::Null),
        "index_names": indexes,
    })
}

/// Every `Index Name` anywhere in a plan tree.
fn collect_indexes(node: &Value, out: &mut Vec<String>) {
    if let Some(name) = node.get("Index Name").and_then(Value::as_str) {
        out.push(name.to_owned());
    }
    if let Some(children) = node.get("Plans").and_then(Value::as_array) {
        for child in children {
            collect_indexes(child, out);
        }
    }
}

// ── the benched operations ──────────────────────────────────────────────────

/// The CONTAINS chain the query benches run: EHR → COMPOSITION → the ENTRY the
/// seeded payload actually carries.
fn population_aql(corpus: &Corpus) -> String {
    format!(
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c CONTAINS {} o LIMIT 50",
        corpus.entry_rm_type
    )
}

/// Configure one group for a database-bound operation: flat sampling (the
/// criterion book's recommendation for long-running benches) and a small
/// sample, so a class-`poc` run stays minutes rather than hours.
fn configure(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.sampling_mode(SamplingMode::Flat);
    group.sample_size(10);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(5));
}

/// Run one benched operation and return its record entry: what criterion
/// measured, beside the database-side facts the measurement moved.
fn measured<B>(
    rt: &Runtime,
    pool: &PgPool,
    group: &mut BenchmarkGroup<'_, WallTime>,
    group_name: &str,
    name: &str,
    body: B,
) -> Value
where
    B: FnMut(&mut Bencher<'_, WallTime>),
{
    let before = rt.block_on(snapshot(pool));
    group.bench_function(name, body);
    let database = rt.block_on(facts(pool, &before));
    operation(group_name, name, &database)
}

/// The write path: a first version, a blind supersession, and a supersession
/// that states the version it replaces (the `If-Match` shape).
fn bench_commit(
    c: &mut Criterion,
    rt: &Runtime,
    svc: &FerroEhrService,
    pool: &PgPool,
    corpus: &Corpus,
) -> Vec<Value> {
    const GROUP: &str = "storage_commit";
    let ehr = *corpus.ehrs.first().expect("the corpus holds an EHR");
    let mut group = c.benchmark_group(GROUP);
    configure(&mut group);
    let mut ops = Vec::new();

    ops.push(measured(
        rt,
        pool,
        &mut group,
        GROUP,
        "create_first_version",
        |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        svc.create_composition(
                            ehr,
                            update_version(&corpus.payload, CREATION, None),
                        )
                        .await
                        .expect("commit a first version");
                    }
                    start.elapsed()
                })
            });
        },
    ));

    // Each supersession needs the chain it extends, so the benched vo is
    // created once and its head carried across iterations.
    let mut head = rt.block_on(async {
        svc.create_composition(ehr, update_version(&corpus.payload, CREATION, None))
            .await
            .expect("the superseded chain's first version")
    });
    ops.push(measured(rt, pool, &mut group, GROUP, "supersede", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let start = Instant::now();
                for _ in 0..iters {
                    head = svc
                        .update_composition(
                            ehr,
                            head.vo_id,
                            update_version(&corpus.payload, MODIFICATION, None),
                        )
                        .await
                        .expect("supersede");
                }
                start.elapsed()
            })
        });
    }));

    let mut checked = rt.block_on(async {
        svc.create_composition(ehr, update_version(&corpus.payload, CREATION, None))
            .await
            .expect("the If-Match chain's first version")
    });
    ops.push(measured(
        rt,
        pool,
        &mut group,
        GROUP,
        "supersede_if_match",
        |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        let preceding = checked.version_uid();
                        checked = svc
                            .update_composition(
                                ehr,
                                checked.vo_id,
                                update_version(&corpus.payload, MODIFICATION, Some(&preceding)),
                            )
                            .await
                            .expect("supersede under If-Match");
                    }
                    start.elapsed()
                })
            });
        },
    ));
    group.finish();
    ops
}

/// The read paths: by version uid, by versioned-object uid, at an instant, and
/// the revision history.
fn bench_read(
    c: &mut Criterion,
    rt: &Runtime,
    svc: &FerroEhrService,
    pool: &PgPool,
    corpus: &Corpus,
) -> Vec<Value> {
    const GROUP: &str = "storage_read";
    let kept = corpus.kept.first().expect("the corpus keeps a composition");
    let version_uid: ObjectVersionId = kept
        .first_version_uid
        .parse()
        .expect("the kept version uid is an OBJECT_VERSION_ID");
    let mut group = c.benchmark_group(GROUP);
    configure(&mut group);
    let mut ops = Vec::new();

    ops.push(measured(
        rt,
        pool,
        &mut group,
        GROUP,
        "by_version_uid",
        |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        svc.get_composition_at_version(kept.ehr, version_uid.clone())
                            .await
                            .expect("read by version uid");
                    }
                    start.elapsed()
                })
            });
        },
    ));
    ops.push(measured(
        rt,
        pool,
        &mut group,
        GROUP,
        "latest_by_versioned_object_uid",
        |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        svc.get_composition_latest(kept.ehr, kept.vo)
                            .await
                            .expect("read the latest version");
                    }
                    start.elapsed()
                })
            });
        },
    ));
    ops.push(measured(
        rt,
        pool,
        &mut group,
        GROUP,
        "version_at_time",
        |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        svc.composition_version_at_time(kept.ehr, kept.vo, Some(corpus.at.clone()))
                            .await
                            .expect("read the version at an instant");
                    }
                    start.elapsed()
                })
            });
        },
    ));
    ops.push(measured(
        rt,
        pool,
        &mut group,
        GROUP,
        "revision_history",
        |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        svc.composition_revision_history(kept.ehr, kept.vo)
                            .await
                            .expect("read the revision history");
                    }
                    start.elapsed()
                })
            });
        },
    ));
    group.finish();
    ops
}

/// The CONTAINS chain, scoped to one EHR and over the whole population.
fn bench_query(
    c: &mut Criterion,
    rt: &Runtime,
    svc: &FerroEhrService,
    pool: &PgPool,
    corpus: &Corpus,
) -> Vec<Value> {
    const GROUP: &str = "storage_query";
    let aql = population_aql(corpus);
    let one_ehr = AqlQueryRequest {
        ehr_ids: vec![corpus.ehrs.first().expect("an EHR").to_string()],
        ..AqlQueryRequest::default()
    };
    let mut group = c.benchmark_group(GROUP);
    configure(&mut group);
    let mut ops = Vec::new();

    ops.push(measured(
        rt,
        pool,
        &mut group,
        GROUP,
        "contains_one_ehr",
        |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        svc.execute_ad_hoc_query(aql.clone(), one_ehr.clone())
                            .await
                            .expect("the single-EHR CONTAINS query executes");
                    }
                    start.elapsed()
                })
            });
        },
    ));
    ops.push(measured(
        rt,
        pool,
        &mut group,
        GROUP,
        "contains_population",
        |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        svc.execute_ad_hoc_query(aql.clone(), AqlQueryRequest::default())
                            .await
                            .expect("the population CONTAINS query executes");
                    }
                    start.elapsed()
                })
            });
        },
    ));
    group.finish();
    ops
}

/// The lifecycle paths: archive and restore of one EHR, and one retention
/// prune of the event outbox.
fn bench_lifecycle(
    c: &mut Criterion,
    rt: &Runtime,
    svc: &FerroEhrService,
    pool: &PgPool,
    corpus: &Corpus,
) -> Vec<Value> {
    const GROUP: &str = "storage_lifecycle";
    let subject = corpus
        .ehrs
        .last()
        .expect("the corpus holds an EHR")
        .to_string();
    let mut group = c.benchmark_group(GROUP);
    configure(&mut group);
    let mut ops = Vec::new();

    // Archive and restore are each other's setup, so only the measured half of
    // the pair is timed; `iter_custom` is what makes that expressible.
    ops.push(measured(rt, pool, &mut group, GROUP, "archive_ehr", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    let start = Instant::now();
                    svc.archive_ehrs(vec![subject.clone()])
                        .await
                        .expect("archive the EHR");
                    total += start.elapsed();
                    svc.restore_archived_ehrs(vec![subject.clone()])
                        .await
                        .expect("restore the EHR");
                }
                total
            })
        });
    }));
    ops.push(measured(rt, pool, &mut group, GROUP, "restore_ehr", |b| {
        b.iter_custom(|iters| {
            rt.block_on(async {
                let mut total = Duration::ZERO;
                for _ in 0..iters {
                    svc.archive_ehrs(vec![subject.clone()])
                        .await
                        .expect("archive the EHR");
                    let start = Instant::now();
                    svc.restore_archived_ehrs(vec![subject.clone()])
                        .await
                        .expect("restore the EHR");
                    total += start.elapsed();
                }
                total
            })
        });
    }));

    // NOTE: the seed stamps no `published_at`, so the prune deletes nothing —
    // which is what makes it repeatable across iterations: the measurement is
    // the retention statement's scan over the seeded outbox.
    ops.push(measured(
        rt,
        pool,
        &mut group,
        GROUP,
        "retention_prune",
        |b| {
            b.iter_custom(|iters| {
                rt.block_on(async {
                    let start = Instant::now();
                    for _ in 0..iters {
                        ferroehr::extensions::outbox::prune(pool, PRUNE_RETENTION_DAYS)
                            .await
                            .expect("prune the outbox");
                    }
                    start.elapsed()
                })
            });
        },
    ));
    group.finish();
    ops
}

// ── the record ──────────────────────────────────────────────────────────────

/// Criterion's output root, `CRITERION_HOME` when set.
fn criterion_home() -> PathBuf {
    #[expect(
        clippy::disallowed_methods,
        reason = "`CRITERION_HOME` is criterion's own documented output-directory variable, \
                  read here to find the estimates criterion just wrote"
    )]
    let home = std::env::var("CRITERION_HOME").ok();
    home.map_or_else(
        || Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/criterion"),
        PathBuf::from,
    )
}

/// What criterion recorded for one benchmark id.
struct Measured {
    /// How many times the routine ran across every sample.
    iterations: f64,
    /// The mean point estimate, in nanoseconds per iteration.
    mean_ns: Value,
    /// The median point estimate, in nanoseconds per iteration.
    median_ns: Value,
    /// Nanoseconds per iteration for each sample, ascending.
    per_iteration: Vec<f64>,
}

/// The timings criterion recorded for one benchmark id, empty when it ran in
/// profile mode (`--profile-time` measures nothing).
fn timings(group: &str, name: &str) -> Measured {
    let dir = criterion_home().join(group).join(name).join("new");
    let estimates: Option<Value> = std::fs::read_to_string(dir.join("estimates.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let sample: Option<Value> = std::fs::read_to_string(dir.join("sample.json"))
        .ok()
        .and_then(|raw| serde_json::from_str(&raw).ok());
    let point = |key: &str| {
        estimates
            .as_ref()
            .and_then(|e| e.get(key))
            .and_then(|e| e.get("point_estimate"))
            .cloned()
            .unwrap_or(Value::Null)
    };
    let mut per_iteration = Vec::new();
    let mut iterations = 0.0_f64;
    if let Some(sample) = &sample {
        let times = sample.get("times").and_then(Value::as_array);
        let iters = sample.get("iters").and_then(Value::as_array);
        if let (Some(times), Some(iters)) = (times, iters) {
            for (time, count) in times.iter().zip(iters.iter()) {
                if let (Some(time), Some(count)) = (time.as_f64(), count.as_f64())
                    && count > 0.0
                {
                    iterations += count;
                    per_iteration.push(time / count);
                }
            }
        }
    }
    per_iteration.sort_by(f64::total_cmp);
    Measured {
        iterations,
        mean_ns: point("mean"),
        median_ns: point("median"),
        per_iteration,
    }
}

/// One record entry: the benchmark's timings beside the database-side facts
/// its measurement moved.
fn operation(group: &str, name: &str, database: &Value) -> Value {
    let measured = timings(group, name);
    // Nearest-rank percentiles over criterion's own samples: integer ranks, so
    // a published figure does not depend on a rounding mode.
    let at = |pct: usize| -> Value {
        if measured.per_iteration.is_empty() {
            return Value::Null;
        }
        let rank = (pct * measured.per_iteration.len()).div_ceil(100).max(1);
        measured
            .per_iteration
            .get(rank - 1)
            .copied()
            .map_or(Value::Null, Value::from)
    };
    json!({
        "name": name,
        "iterations": measured.iterations,
        "samples": measured.per_iteration.len(),
        "mean_ns": measured.mean_ns,
        "median_ns": measured.median_ns,
        "p50_ns": at(50),
        "p95_ns": at(95),
        "p99_ns": at(99),
        "database": database,
    })
}

/// Whether one record entry carries timings at all.
fn has_timings(operation: &Value) -> bool {
    operation
        .get("samples")
        .and_then(Value::as_u64)
        .is_some_and(|samples| samples > 0)
}

/// The schema generation the record is filed under.
///
/// `ext.storage_generation()` is the authority when a schema declares one;
/// `STORAGE_BENCH_GENERATION` overrides for a schema that does not, and the
/// first generation is the default.
async fn generation(pool: &PgPool) -> String {
    if let Ok(name) = sqlx::query_scalar::<_, String>("SELECT ext.storage_generation()")
        .fetch_one(pool)
        .await
    {
        return name;
    }
    #[expect(
        clippy::disallowed_methods,
        reason = "`STORAGE_BENCH_GENERATION` labels the record this harness writes; \
                  it is a bench knob and never reaches a running server"
    )]
    let override_name = std::env::var("STORAGE_BENCH_GENERATION").ok();
    override_name
        .filter(|name| !name.is_empty())
        .unwrap_or_else(|| "generation-1".to_owned())
}

/// The machine's total memory as the operating system reports it.
fn memory_bytes() -> Value {
    let probe = if cfg!(target_os = "macos") {
        std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .and_then(|out| String::from_utf8(out.stdout).ok())
    } else {
        std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|raw| {
                raw.lines()
                    .find_map(|line| line.strip_prefix("MemTotal:"))
                    .and_then(|rest| rest.split_whitespace().next())
                    // /proc/meminfo reports kibibytes.
                    .and_then(|kib| kib.parse::<i64>().ok())
                    .map(|kib| (kib * 1024).to_string())
            })
    };
    probe
        .and_then(|raw| raw.trim().parse::<i64>().ok())
        .map_or(Value::Null, Value::from)
}

/// The commit the measurement was taken at, so the record is re-checkable.
fn git_head() -> String {
    std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .and_then(|out| String::from_utf8(out.stdout).ok())
        .map_or_else(|| "unknown".to_owned(), |rev| rev.trim().to_owned())
}

/// Write the record under `docs/benchmarks/storage/<generation>/`.
fn write_record(generation: &str, record: &Value) {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/benchmarks/storage")
        .join(generation);
    std::fs::create_dir_all(&dir).expect("the record directory must be creatable");
    let path = dir.join("record.json");
    std::fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(record).expect("render the record")
        ),
    )
    .expect("the record must be writable");
    println!("storage benchmark record: {}", path.display());
}

// ── the harness ─────────────────────────────────────────────────────────────

/// Seed one corpus, run every group against it, and write the record.
fn storage(c: &mut Criterion) {
    let class = BenchClass::from_env();
    let rt = Runtime::new().expect("the bench tokio runtime must start");
    let db = rt
        .block_on(testkit::db())
        .expect("the testkit database must be available");
    let pool = db.pool();
    // The service builds a lazy sqlx pool of its own, and sqlx spawns that
    // pool's maintenance task at construction, so it is built inside the
    // runtime rather than beside it.
    let svc = rt.block_on(async {
        Arc::new(FerroEhrService::new(
            &ferroehr::db::domain::DomainPools::from_shared(&pool),
        ))
    });
    let corpus = rt.block_on(seed(&svc, class));

    let groups = vec![
        json!({ "name": "storage_commit",
                "operations": bench_commit(c, &rt, &svc, &pool, &corpus) }),
        json!({ "name": "storage_read",
                "operations": bench_read(c, &rt, &svc, &pool, &corpus) }),
        json!({ "name": "storage_query",
                "operations": bench_query(c, &rt, &svc, &pool, &corpus) }),
        json!({ "name": "storage_lifecycle",
                "operations": bench_lifecycle(c, &rt, &svc, &pool, &corpus) }),
    ];
    let operations: Vec<&Value> = groups
        .iter()
        .filter_map(|group| group.get("operations"))
        .filter_map(Value::as_array)
        .flatten()
        .collect();
    let timed = operations.iter().filter(|op| has_timings(op)).count();
    if timed == 0 {
        println!(
            "storage bench: nothing was measured (profile mode, or a filtered run); \
             the record is left as it was"
        );
        return;
    }

    let postgres_version = rt
        .block_on(sqlx::query_scalar::<_, String>("SELECT version()").fetch_one(&pool))
        .unwrap_or_else(|_| "unknown".to_owned());
    let generation = rt.block_on(generation(&pool));
    let record = json!({
        "schema_generation": generation,
        "postgres_version": postgres_version,
        "bench_class": class.name,
        "cpu_count": std::thread::available_parallelism().map_or(0, std::num::NonZeroUsize::get),
        "memory_bytes": memory_bytes(),
        "measured_at_commit": git_head(),
        "measured_at": jiff::Timestamp::now().to_string(),
        "timings_measured": timed == operations.len(),
        "seed": {
            "ehrs": class.ehrs,
            "compositions": class.compositions,
            "template_id": corpus.template_id,
            "entry_rm_type": corpus.entry_rm_type,
            "seconds": corpus.seconds,
        },
        "groups": groups,
        "commit_probe": rt.block_on(commit_probe(&svc, &pool, &corpus)),
        "explain": rt.block_on(explain_population(&pool, &corpus)),
        "relations": rt.block_on(relations(&pool)),
    });
    write_record(&generation, &record);
}

/// Criterion `Profiler` glue over the `pprof` sampler: start a guard when
/// criterion enters profile mode, render `flamegraph.svg` into the bench's
/// profile directory when it leaves.
struct FlamegraphProfiler {
    frequency: i32,
    guard: Option<pprof::ProfilerGuard<'static>>,
}

impl FlamegraphProfiler {
    const fn new(frequency: i32) -> Self {
        Self {
            frequency,
            guard: None,
        }
    }
}

impl Profiler for FlamegraphProfiler {
    fn start_profiling(&mut self, _benchmark_id: &str, _benchmark_dir: &Path) {
        self.guard = Some(
            pprof::ProfilerGuardBuilder::default()
                .frequency(self.frequency)
                // The unwind-hazard blocklist pprof's docs recommend
                // (<https://docs.rs/pprof/latest/pprof/>).
                .blocklist(&["libc", "libgcc", "pthread", "vdso"])
                .build()
                .expect("the pprof sampler must start"),
        );
    }

    fn stop_profiling(&mut self, benchmark_id: &str, benchmark_dir: &Path) {
        let Some(guard) = self.guard.take() else {
            return;
        };
        let report = guard.report().build().expect("the pprof report must build");
        std::fs::create_dir_all(benchmark_dir).expect("the profile directory must be creatable");
        let path = benchmark_dir.join("flamegraph.svg");
        let file = File::create(&path).expect("the flamegraph file must be creatable");
        // NOTE: #2406 — pprof's `flamegraph` feature pins inferno ^0.11
        // (quick-xml 0.26, RUSTSEC-2026-0194/0195); this is pprof 0.15's own
        // fold, rendered through the direct inferno 0.12 dependency instead.
        let lines: Vec<String> = report
            .data
            .iter()
            .map(|(frames, count)| {
                let mut segments = vec![frames.thread_name_or_id()];
                for frame in frames.frames.iter().rev() {
                    for symbol in frame.iter().rev() {
                        segments.push(symbol.to_string());
                    }
                }
                format!("{} {count}", segments.join(";"))
            })
            .collect();
        if !lines.is_empty() {
            inferno::flamegraph::from_lines(
                &mut inferno::flamegraph::Options::default(),
                lines.iter().map(String::as_str),
                file,
            )
            .expect("the flamegraph must render");
        }
        eprintln!("profiled {benchmark_id}: {}", path.display());
    }
}

criterion_group! {
    name = benches;
    config = Criterion::default().with_profiler(FlamegraphProfiler::new(999));
    targets = storage
}
criterion_main!(benches);
