// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `LATEST_VERSION` read of an object whose versions share one node tree
//! reads that tree once (#4944), against a real `PostgreSQL` 18.
//!
//! A version whose content equals its predecessor's reads its predecessor's
//! node rows (#4415), so one node row of a long identical-content chain
//! belongs to every version of the chain. The storage bench's `supersede`
//! chains, about 1 200 identical versions each and grown after the seed's
//! `ANALYZE`, made the flat FROM shape join each such row to every version of
//! its chain before the head filter removed all but one (run 41799106844).
//! The fixture rebuilds that state the way the bench builds it: one payload
//! committed over the population, one object per EHR superseded with the
//! same payload, the statistics taken, autovacuum held off so they stay those
//! of the seed, and then two chains. The plan is read with
//! `EXPLAIN (ANALYZE, BUFFERS)` inside a rolled-back transaction
//! (<https://www.postgresql.org/docs/18/sql-explain.html>). No openEHR spec
//! governs storage access paths: our own design.

use std::sync::Arc;

use serde_json::Value;
use sqlx::{Executor as _, PgPool};

use ferroehr::aql::ir::Params;
use ferroehr::aql::lineage::ArchetypeLineage;
use ferroehr::aql::sql::SqlCtx;
use ferroehr::config::profile::SpecProfile;
use ferroehr::service::FerroEhrService;

use crate::fixtures::{create_ehr, db_now, observation_composition, uv};

/// EHRs of the seed.
const SEED_EHRS: usize = 20;
/// Compositions of the seed, spread over its EHRs, all of one payload.
const SEED_COMPOSITIONS: usize = 300;
/// Versions of each identical-content chain: far more than the store holds
/// objects, so a read that reaches a shared row through every version of its
/// chain outgrows the bound.
const CHAIN: u32 = 1_200;

/// The relations whose statistics the chains must not refresh.
const FROZEN: &str = "SELECT format('%I.%I', n.nspname, c.relname) \
     FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
     WHERE c.relkind = 'r' AND c.relname IN ('version_hot', 'node_hot', 'vo_head', 'commit_audit') \
     ORDER BY 1";

/// The store the reads run over.
struct Seeded {
    /// The objects the store holds, each a row of a `LATEST_VERSION` read.
    objects: u32,
    /// An instant after the seed and before the chains.
    before_chains: jiff::Timestamp,
}

/// The seed, its statistics, and then two chains of [`CHAIN`] identical
/// versions the statistics never see.
async fn seed(svc: &FerroEhrService, pool: &PgPool) -> Seeded {
    let mut ehrs = Vec::new();
    for _ in 0..SEED_EHRS {
        ehrs.push(create_ehr(svc).await);
    }
    let body = observation_composition("bench payload", 120.0);
    for ehr in ehrs.iter().cycle().take(SEED_COMPOSITIONS) {
        svc.create_composition(*ehr, uv(&body, "249", None))
            .await
            .expect("commit a seed composition");
    }
    // The bench's kept objects: superseded once with the same payload, so the
    // statistics already see a shared tree.
    for ehr in &ehrs {
        let first = svc
            .create_composition(*ehr, uv(&body, "249", None))
            .await
            .expect("commit a kept composition");
        let preceding = first.version_uid();
        svc.update_composition(*ehr, first.vo_id, uv(&body, "251", Some(&preceding)))
            .await
            .expect("supersede the kept composition");
    }
    pool.execute("ANALYZE").await.expect("analyze the seed");
    ferroehr::db::parent_statistics::refresh_pools(
        &ferroehr::db::domain::DomainPools::from_shared(pool),
    )
    .await
    .expect("analyze the seeded partitioned parents");
    let frozen: Vec<String> = sqlx::query_scalar(FROZEN)
        .fetch_all(pool)
        .await
        .expect("name the relations the chains grow");
    for relation in frozen {
        pool.execute(sqlx::AssertSqlSafe(format!(
            "ALTER TABLE {relation} SET (autovacuum_enabled = off)"
        )))
        .await
        .expect("hold autovacuum off the relations the chains grow");
    }
    let before_chains = db_now(pool).await;
    let ehr = *ehrs.first().expect("the seed holds EHRs");
    for _ in 0..2 {
        let mut head = svc
            .create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("the chain's first version");
        for _ in 1..CHAIN {
            let preceding = head.version_uid();
            head = svc
                .update_composition(ehr, head.vo_id, uv(&body, "251", Some(&preceding)))
                .await
                .expect("an identical supersession");
        }
    }
    let objects = SEED_COMPOSITIONS + SEED_EHRS + 2;
    Seeded {
        objects: u32::try_from(objects).expect("a small store"),
        before_chains,
    }
}

/// The lowered statement of a population AQL query.
fn lowered(aql: &str) -> ferroehr::aql::sql::PreparedQuery {
    let ast = openehr_query::parser::parse_str(aql).expect("the AQL parses");
    let params = Params::default();
    let ir = ferroehr::aql::plan(&ast, &params, SpecProfile::default()).expect("the AQL plans");
    let ctx = SqlCtx {
        system_id: "fan.out.test".to_owned(),
        ehr_ids: Vec::new(),
        ehr_scope_kind: ferroehr::aql::sql::EhrScopeKind::Named,
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: Some(50),
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    ferroehr::aql::sql::build(&ir, &params, &ctx).expect("the AQL lowers to SQL")
}

/// `EXPLAIN (ANALYZE, BUFFERS)` of `aql` under the server's own planner
/// settings, rolled back.
async fn explain(pool: &PgPool, aql: &str) -> Value {
    let prepared = lowered(aql);
    let mut tx = pool.begin().await.expect("begin the explain transaction");
    let plan: Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!(
            "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) {}",
            prepared.sql
        )),
        prepared.values,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("explain the AQL statement");
    tx.rollback().await.expect("roll the explain back");
    plan
}

/// The rows every node of a plan tree returned over all its loops, with the
/// relation it read, outermost first.
fn emitted(node: &Value, out: &mut Vec<(f64, String)>) {
    let rows = node
        .get("Actual Rows")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let loops = node
        .get("Actual Loops")
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    let relation = node
        .get("Relation Name")
        .and_then(Value::as_str)
        .unwrap_or("");
    let label = format!(
        "{} {relation}",
        node.get("Node Type").and_then(Value::as_str).unwrap_or("?")
    );
    out.push((rows * loops, label));
    for child in node
        .get("Plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        emitted(child, out);
    }
}

/// Which relations a read must touch a bounded number of rows of.
#[derive(Debug, Clone, Copy)]
enum Bounded {
    /// The node rows and the version rows: a shape that enters through the
    /// node rows.
    NodesAndVersions,
    /// The node rows: the streaming shape, whose version spine reads every
    /// version of the population before the head join keeps the current one,
    /// as it did before any version shared a tree.
    Nodes,
}

/// Every scan of a relation `bounded` names returns fewer rows than half a
/// chain over the whole read: a node row reaches one version of its object,
/// never every version that shares its tree.
async fn assert_bounded(pool: &PgPool, aql: &str, bounded: Bounded, objects: u32) {
    let plan = explain(pool, aql).await;
    let mut nodes = Vec::new();
    emitted(&plan[0]["Plan"], &mut nodes);
    let bound = f64::from(CHAIN) / 2.0;
    let relations: &[&str] = match bounded {
        Bounded::NodesAndVersions => &["node_hot", "version_hot"],
        Bounded::Nodes => &["node_hot"],
    };
    let rendered = nodes
        .iter()
        .map(|(rows, label)| format!("{rows:>9} {label}"))
        .collect::<Vec<_>>()
        .join("\n");
    let unbounded: Vec<&(f64, String)> = nodes
        .iter()
        .filter(|(rows, label)| {
            *rows >= bound && relations.iter().any(|r| label.ends_with(&format!(" {r}")))
        })
        .collect();
    assert!(
        unbounded.is_empty(),
        "#4944: {aql}\nanswers from {objects} objects, and a scan of {relations:?} returned \
         {bound} rows or more:\n{rendered}"
    );
}

/// The leaves of one OBSERVATION, under its repeated EVENT.
const LEAVES: &str = "o/data[at0001]/events[at0002]/data[at0003]/items[at0004]/value";

/// The population read of the OBSERVATION rows of the seed's payload.
const OBSERVATIONS: &str = "FROM EHR e CONTAINS COMPOSITION c \
     CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.minimal.v1]";

#[tokio::test]
async fn a_latest_read_of_a_shared_tree_reads_it_once() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let seeded = seed(&svc, &pool).await;
    let reads = [
        // The bench's `flat_2_sibling_leaves` and `flat_3_sibling_leaves`:
        // the flat FROM shape (`ORDER BY`).
        (
            format!(
                "SELECT {LEAVES}/magnitude, {LEAVES}/units {OBSERVATIONS} \
                 ORDER BY c/uid/value LIMIT 50"
            ),
            Bounded::NodesAndVersions,
        ),
        (
            format!(
                "SELECT {LEAVES}/magnitude, {LEAVES}/units, {LEAVES}/precision {OBSERVATIONS} \
                 ORDER BY c/uid/value LIMIT 50"
            ),
            Bounded::NodesAndVersions,
        ),
        // A leaf condition in the flat shape, and a value anchor in both.
        (
            format!(
                "SELECT c/uid/value {OBSERVATIONS} WHERE {LEAVES}/magnitude > 1 \
                 ORDER BY c/uid/value LIMIT 50"
            ),
            Bounded::NodesAndVersions,
        ),
        (
            format!(
                "SELECT c/uid/value {OBSERVATIONS} WHERE {LEAVES}/units = 'mm[Hg]' \
                 ORDER BY c/uid/value LIMIT 50"
            ),
            Bounded::NodesAndVersions,
        ),
        (
            format!("SELECT c/uid/value {OBSERVATIONS} WHERE {LEAVES}/units = 'mm[Hg]' LIMIT 50"),
            Bounded::NodesAndVersions,
        ),
        // The bench's `streaming_*` paths: the LIMIT-streaming shape.
        (
            format!("SELECT {LEAVES}/magnitude, {LEAVES}/units {OBSERVATIONS} LIMIT 50"),
            Bounded::Nodes,
        ),
    ];
    for (aql, bounded) in &reads {
        assert_bounded(&pool, aql, *bounded, seeded.objects).await;
    }
}

/// The bench's `aql_version_at_time`: the trunk version in force at an
/// instant before the chains, which no version of a chain is.
#[tokio::test]
async fn a_read_at_an_instant_reads_a_shared_tree_once() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let seeded = seed(&svc, &pool).await;
    let aql = format!(
        "SELECT c/uid/value FROM EHR e CONTAINS VERSION \
         v[commit_audit/time_committed <= '{}'] CONTAINS COMPOSITION c LIMIT 50",
        seeded.before_chains
    );
    assert_bounded(&pool, &aql, Bounded::NodesAndVersions, seeded.objects).await;
}
