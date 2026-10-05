// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cohort-query benchmark: how the two-statement crossing and the clinical
//! query behave as the cohort grows from 100 to the corpus size.
//!
//! **No openEHR spec governs the cohort surface — our own design/extension**,
//! and this is a MEASUREMENT, not an acceptance gate: it is `#[ignore]`d, it
//! writes its record to `docs/benchmarks/cohort/record.json`, and it
//! asserts nothing about timing. The conformance instrument stays the only
//! acceptance authority.
//!
//! ```text
//! COHORT_BENCH_N=100000 cargo nextest run -p ferroehr \
//!     -E 'test(cohort_bench)' --run-ignored all --no-capture
//! ```
//!
//! The corpus nests: every person matches `c100000`, the first 10 000 also
//! match `c10000`, the first 1 000 also `c1000`, the first 100 also `c100`. So
//! the four cohorts are 100 / 1 000 / 10 000 / N EHRs out of the same corpus,
//! and the four measurements differ only in how wide the crossing is.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::print_stdout,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11), and a measurement \
              reports its record on stdout"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use futures::StreamExt as _;
use serde_json::{Value, json};

use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::linkage::cohort::config::{CohortConfig, PredicateBinding, PredicateKind};
use ferroehr::service::linkage::cohort::{CohortPredicate, CohortQueryRequest};

use crate::fixtures::{composition, uv};

/// Where the record lands, relative to the repository root.
const RECORD: &str = "docs/benchmarks/cohort/record.json";
/// How many seeds run at once. The service call is I/O-bound on PostgreSQL, so
/// the width is about keeping the pool busy, not about CPU.
const SEED_CONCURRENCY: usize = 16;
/// How many times each cohort is executed for the percentiles.
const REPEATS: usize = 20;
/// The address CLUSTER archetype the city predicate binds to.
const ADDRESS_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-CLUSTER.address.v1";
/// The AQL every measured execution runs.
const BENCH_AQL: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";

/// The bench's cohort configuration: suppression off (a measurement withholds
/// nothing) and the ceiling above the corpus.
fn bench_config(n: usize) -> CohortConfig {
    CohortConfig {
        small_cell_threshold: 0,
        max_cohort_size: u32::try_from(n).unwrap_or(u32::MAX).saturating_mul(2),
        predicates: BTreeMap::from([(
            "city".to_owned(),
            PredicateBinding {
                archetype: ADDRESS_ARCHETYPE.to_owned(),
                node: "at0012".to_owned(),
                kind: PredicateKind::Text,
            },
        )]),
    }
}

/// A PERSON whose address CLUSTER carries one `at0012` city ELEMENT per cohort
/// the person belongs to.
///
/// One ELEMENT per membership is what makes the four cohorts genuinely nest
/// under a single equality predicate: `city = c1000` reaches every person in
/// the first thousand, `city = c100` the first hundred of those, and the
/// measured sizes are exactly 100 / 1 000 / 10 000 / N rather than the
/// disjoint bands a one-value-per-person corpus would give.
fn person(name: &str, cities: &[String]) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0002",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0003",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0004",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }],
        "details": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "structure" },
            "items": [{
                "_type": "CLUSTER",
                "archetype_node_id": ADDRESS_ARCHETYPE,
                "archetype_details": { "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": ADDRESS_ARCHETYPE },
                    "rm_version": "1.1.0" },
                "name": { "_type": "DV_TEXT", "value": "address" },
                "items": cities.iter().map(|city| json!({
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0012",
                    "name": { "_type": "DV_TEXT", "value": "city" },
                    "value": { "_type": "DV_TEXT", "value": city }
                })).collect::<Vec<Value>>()
            }]
        }
    })
}

/// The four cohort sizes measured against a corpus of `n` people.
fn cohort_sizes(n: usize) -> Vec<usize> {
    let mut sizes: Vec<usize> = [100, 1_000, 10_000, n]
        .into_iter()
        .filter(|size| *size <= n)
        .collect();
    sizes.sort_unstable();
    sizes.dedup();
    sizes
}

/// The city values person `index` carries: one per cohort it belongs to.
fn cohort_values(index: usize, n: usize) -> Vec<String> {
    cohort_sizes(n)
        .into_iter()
        .filter(|size| index < *size)
        .map(|size| format!("c{size}"))
        .collect()
}

/// The summarised top plan node of an `EXPLAIN (ANALYZE, FORMAT JSON)` run.
///
/// Whole plans are hundreds of lines and go stale as noise; what a reader of
/// the record needs is the shape (which node type), the time, the row count and
/// which indexes the planner reached for.
fn summarise_plan(explain: &Value) -> Value {
    let plan = explain
        .get(0)
        .and_then(|first| first.get("Plan"))
        .unwrap_or(&Value::Null);
    let mut indexes = Vec::new();
    collect_indexes(plan, &mut indexes);
    indexes.sort();
    indexes.dedup();
    json!({
        "node_type": plan["Node Type"],
        "actual_total_ms": plan["Actual Total Time"],
        "rows": plan["Actual Rows"],
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

/// The p50 and p95 of a set of millisecond samples, nearest-rank.
///
/// The rank is computed in integers (`ceil(pct * n / 100)`) rather than through
/// a float quantile: with twenty samples the two differ at the boundary, and a
/// published percentile should not depend on a rounding mode.
fn percentiles(mut samples: Vec<f64>) -> (f64, f64) {
    samples.sort_by(f64::total_cmp);
    let at = |pct: usize| {
        let rank = (pct * samples.len()).div_ceil(100).max(1);
        samples.get(rank - 1).copied().unwrap_or(0.0)
    };
    (at(50), at(95))
}

/// Seed `n` parties, each with its own EHR, mapping and composition.
///
/// Concurrent because the work is I/O-bound on `PostgreSQL`: the width keeps the
/// pool busy rather than the CPU. Returns the EHR ids in completion order,
/// which is all the clinical `EXPLAIN` needs.
async fn seed_corpus(svc: &Arc<FerroEhrService>, n: usize) -> Vec<EhrId> {
    let start = Instant::now();
    let seeded: Vec<EhrId> = futures::stream::iter(0..n)
        .map(|index| {
            let svc = Arc::clone(svc);
            let cities = cohort_values(index, n);
            async move {
                let party = Box::pin(svc.create_party(uv(
                    &person(&format!("p{index}"), &cities),
                    "249",
                    None,
                )))
                .await
                .unwrap_or_else(|e| panic!("create_party ({index}): {e:?}"));
                let ehr = svc.create_ehr(None).await.expect("create_ehr");
                svc.link(party, ehr).await.expect("link");
                svc.create_composition(ehr, uv(&composition(&format!("c{index}")), "249", None))
                    .await
                    .unwrap_or_else(|e| panic!("create_composition ({index}): {e:?}"));
                ehr
            }
        })
        .buffer_unordered(SEED_CONCURRENCY)
        .collect()
        .await;
    println!(
        "seeded {} parties + EHRs + compositions in {:.1}s",
        seeded.len(),
        start.elapsed().as_secs_f64()
    );
    seeded
}

/// Measure one cohort size: `REPEATS` executions plus the two plans.
async fn measure(
    svc: &FerroEhrService,
    pool: &sqlx::PgPool,
    seeded: &[EhrId],
    size: usize,
) -> Value {
    let request = CohortQueryRequest {
        aql: BENCH_AQL.to_owned(),
        predicates: vec![CohortPredicate {
            name: "city".to_owned(),
            value: format!("c{size}"),
        }],
        purpose: Some("benchmark".to_owned()),
        query: ferroehr::service::query::request::AqlQueryRequest::default(),
    };
    let mut samples = Vec::with_capacity(REPEATS);
    let mut served = 0;
    for _ in 0..REPEATS {
        let start = Instant::now();
        let outcome = svc
            .execute_cohort_query(request.clone())
            .await
            .unwrap_or_else(|e| panic!("cohort {size}: {e:?}"));
        samples.push(start.elapsed().as_secs_f64() * 1000.0);
        served = outcome.cohort_size;
    }
    let (p50, p95) = percentiles(samples);

    let predicate_plan = explain(
        &ferroehr::db::domain_pool_from(pool, ferroehr::db::domain::Domain::Party),
        ferroehr::service::linkage::cohort::predicate::predicate_sql(PredicateKind::Text),
        &[
            "at0012".to_owned(),
            ADDRESS_ARCHETYPE.to_ascii_lowercase(),
            format!("c{size}"),
        ],
    )
    .await;
    let taken = usize::try_from(served)
        .unwrap_or(seeded.len())
        .min(seeded.len());
    let clinical_plan = explain_clinical(pool, seeded.get(..taken).unwrap_or(&[])).await;

    json!({
        "size": served,
        "p50_ms": p50,
        "p95_ms": p95,
        "predicate_plan": predicate_plan,
        "clinical_plan": clinical_plan,
    })
}

/// Seed the corpus and measure each cohort, writing the record.
///
/// Ignored by default: it seeds a six-figure corpus and takes minutes.
#[tokio::test]
#[ignore = "a measurement, not a gate: seeds a six-figure corpus (COHORT_BENCH_N)"]
async fn cohort_bench() {
    // The corpus size is the one knob a measurement takes, and it has no place
    // in the server's configuration tree, so it is read from the environment
    // directly — the disallowed-method rule guards production config reads.
    #[expect(
        clippy::disallowed_methods,
        reason = "a bench knob, not server configuration: `COHORT_BENCH_N` sizes the corpus \
                  this ignored measurement seeds and never reaches the running server"
    )]
    let n: usize = std::env::var("COHORT_BENCH_N")
        .ok()
        .and_then(|raw| raw.parse().ok())
        // The default corpus: six figures, which is what makes the widest
        // cohort a real measurement rather than a smoke test.
        .unwrap_or(100_000);
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_cohort(bench_config(n)),
    );

    let seeded = seed_corpus(&svc, n).await;
    let mut cohorts = Vec::new();
    for size in cohort_sizes(n) {
        cohorts.push(measure(&svc, &pool, &seeded, size).await);
    }

    let record = json!({
        "corpus": { "parties": n, "ehrs": n, "compositions": n },
        "measured_at_commit": git_head(),
        "environment": environment(),
        "cohorts": cohorts,
    });
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(RECORD);
    let path = path.canonicalize().unwrap_or(path);
    std::fs::write(
        &path,
        format!(
            "{}\n",
            serde_json::to_string_pretty(&record).expect("render")
        ),
    )
    .unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    println!("cohort benchmark record: {}", path.display());
}

/// `EXPLAIN (ANALYZE, FORMAT JSON)` over a text-bound statement.
async fn explain(pool: &sqlx::PgPool, sql: &str, binds: &[String]) -> Value {
    let mut query = sqlx::query_scalar::<_, Value>(sqlx::AssertSqlSafe(format!(
        "EXPLAIN (ANALYZE, FORMAT JSON) {sql}"
    )));
    for bind in binds {
        query = query.bind(bind.clone());
    }
    let plan = query.fetch_one(pool).await.expect("explain the predicate");
    summarise_plan(&plan)
}

/// `EXPLAIN (ANALYZE, FORMAT JSON)` over the clinical statement the engine
/// builds for `ehr_ids`, bound exactly as the executor binds it.
async fn explain_clinical(pool: &sqlx::PgPool, ehr_ids: &[EhrId]) -> Value {
    use ferroehr::aql::ir::Params;
    use ferroehr::aql::lineage::ArchetypeLineage;
    use ferroehr::aql::sql::SqlCtx;

    let ctx = SqlCtx {
        system_id: "bench.example.com".to_owned(),
        ehr_ids: ehr_ids.to_vec(),
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    let ast = openehr_query::parser::parse_str(BENCH_AQL).expect("the AQL parses");
    let params = Params::new();
    let ir = ferroehr::aql::plan(
        &ast,
        &params,
        ferroehr::config::profile::SpecProfile::default(),
    )
    .expect("the AQL plans");
    let prepared = ferroehr::aql::sql::build(&ir, &params, &ctx).expect("the AQL lowers");
    let plan: Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("EXPLAIN (ANALYZE, FORMAT JSON) {}", prepared.sql)),
        prepared.values,
    )
    .fetch_one(pool)
    .await
    .expect("explain the clinical statement");
    summarise_plan(&plan)
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

/// The environment the conformance party statement declares, verbatim, so a
/// reader compares like with like rather than guessing at the hardware.
fn environment() -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/conformance/party/ferroehr/ixit.json");
    std::fs::read_to_string(path)
        .ok()
        .and_then(|raw| serde_json::from_str::<Value>(&raw).ok())
        .and_then(|ixit| ixit.get("environment").cloned())
        .unwrap_or(Value::Null)
}
