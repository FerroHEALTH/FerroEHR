// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The access path of every hot path of the storage, pinned as a plan shape.
//!
//! Each hypothesis of the storage performance model claims a specific access
//! path: a primary-key probe on `vo_head` for "what is current", a descending
//! commit-time probe for `version_at_time`, one pruned partition for AQL over
//! the hot tier, a BRIN bitmap scan for a commit-time range, two partition
//! probes for a point read on the partitioned parent. None of those claims
//! needs a clock to check: `EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON)` names the
//! node type, the index and the relation a statement actually touched, and the
//! `PostgreSQL` documentation gives the instrument its own shape — "if you
//! wish to analyze a data-modifying query without changing your tables, you can
//! roll the command back afterwards ... use it inside a transaction and roll
//! back afterwards" (<https://www.postgresql.org/docs/18/sql-explain.html>).
//! Every statement below therefore runs inside a transaction that is rolled
//! back.
//!
//! What each test explains is the statement the read path itself emits: the
//! point reads and the metadata reads expose their statement as a named
//! constant in `ferroehr::storage::version_repo`, and the AQL statements come
//! out of `ferroehr::aql::sql::build`, so no copy of any statement lives here
//! to drift away from the one that runs in production.
//!
//! Sequential scans are discouraged for the duration of each explain
//! transaction. The seeded corpus is a few hundred rows, where a sequential
//! scan is genuinely the cheaper plan and the planner is right to choose it, so
//! a cost-based choice at this scale would pin nothing about a production
//! store; what these tests pin is the access path each statement HAS. Turning
//! the setting off "discourages the planner from using [a sequential scan] if
//! there are other methods available" and cannot suppress one entirely
//! (<https://www.postgresql.org/docs/18/runtime-config-query.html>), so an
//! index that disappears, or a predicate rewritten so no index can serve it,
//! still surfaces here as a `Seq Scan` and fails the assertion.
//!
//! No openEHR spec governs storage mechanics, index choice or partitioning —
//! our own design/extension.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; a failed seed or explain is broken \
              test infrastructure and must panic where it happens (the Rust Book ch11)"
)]

use std::sync::Arc;

use serde_json::Value;
use sqlx::{Executor as _, PgPool, Postgres, Transaction};

use ferroehr::aql::ir::Params;
use ferroehr::aql::lineage::ArchetypeLineage;
use ferroehr::aql::sql::SqlCtx;
use ferroehr::config::profile::SpecProfile;
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::storage::version_repo::{meta, read};

use crate::fixtures::{composition, uv};

// ── the seeded store ─────────────────────────────────────────────────────────

/// The repository every test in this module explains statements against.
struct Seeded {
    /// The EHR whose content stays in the hot tier.
    ehr: EhrId,
    /// A COMPOSITION container of [`Seeded::ehr`] carrying three versions, so a
    /// revision history and a time-travel read both have a tree to walk.
    vo: VoId,
    /// The storage ordinal of that container's current version.
    sys_version: i32,
    /// The instant the container's first version was committed.
    first_committed: jiff::Timestamp,
}

/// A repository with content in both tiers: several EHRs of live content, and
/// one EHR archived so the `cold` partitions are populated rather than empty.
///
/// A plan over an empty partition proves nothing about pruning — `EXPLAIN
/// ANALYZE` would report the same "never executed" node whether the planner
/// pruned the partition or merely found nothing in it — so the archived EHR is
/// part of the fixture, not decoration.
async fn seed(svc: &FerroEhrService, pool: &PgPool) -> Seeded {
    let mut first: Option<Seeded> = None;
    for index in 0..8 {
        let ehr = svc.create_ehr(None).await.expect("create an EHR");
        let body = composition(&format!("plan shapes {index}"));
        let created = svc
            .create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("commit the first version");
        let mut latest = created.version_uid();
        let mut committed = created;
        for _ in 0..2 {
            committed = svc
                .update_composition(ehr, committed.vo_id, uv(&body, "251", Some(&latest)))
                .await
                .expect("commit a superseding version");
            latest = committed.version_uid();
        }
        if first.is_none() {
            first = Some(Seeded {
                ehr,
                vo: committed.vo_id,
                sys_version: committed.sys_version,
                first_committed: committed.time_committed,
            });
        }
    }
    // One more EHR, archived: its version, node and attestation rows move into
    // the `cold` partitions, which is what gives a pruning assertion something
    // to exclude.
    let archived = svc.create_ehr(None).await.expect("create the archived EHR");
    svc.create_composition(archived, uv(&composition("archived"), "249", None))
        .await
        .expect("commit into the EHR that is about to be archived");
    svc.archive_ehrs(vec![archived.to_string()])
        .await
        .expect("archive an EHR into the cold tier");
    for relation in ["version", "node"] {
        let cold: i64 = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
            "SELECT count(*) FROM {relation} WHERE tier = 'cold'"
        )))
        .fetch_one(pool)
        .await
        .expect("count the archived rows");
        assert!(
            cold > 0,
            "the archive populated the cold {relation} partition, so a pruning \
             assertion excludes rows that exist"
        );
    }
    pool.execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded corpus");
    first.expect("the seed committed at least one container")
}

// ── the instrument ───────────────────────────────────────────────────────────

/// The `EXPLAIN` prefix every statement below is run under.
const EXPLAIN: &str = "EXPLAIN (ANALYZE, BUFFERS, FORMAT JSON) ";

/// Open the transaction an explain runs in: sequential scans discouraged, and
/// rolled back by the caller so `ANALYZE` leaves nothing behind (`PostgreSQL`
/// docs, <https://www.postgresql.org/docs/18/sql-explain.html>).
async fn explain_tx(pool: &PgPool) -> Transaction<'static, Postgres> {
    let mut tx = pool.begin().await.expect("begin the explain transaction");
    tx.execute("SET LOCAL enable_seqscan = off")
        .await
        .expect("discourage sequential scans for this transaction");
    tx
}

/// One scan node of a plan tree: what it read, how, and how many rows came out.
#[derive(Debug)]
struct Scan {
    /// The plan node type verbatim (`Index Scan`, `Bitmap Heap Scan`, …).
    node_type: String,
    /// The relation the node read — a partition name where the relation is
    /// partitioned.
    relation: String,
    /// The index the node used, where it used one.
    index: Option<String>,
    /// Rows the node returned, averaged over its loops — `PostgreSQL` 18
    /// reports this as a fraction rather than an integer
    /// (<https://www.postgresql.org/docs/18/sql-explain.html>).
    rows: f64,
    /// Whether the executor ever ran the node. A pruned partition does not
    /// appear in the plan at all; a node that appears but never ran was left
    /// unexecuted at run time, which is a different fact.
    executed: bool,
    /// Shared buffers hit by the node over all its loops, and the loops: the
    /// quotient is the cost of one probe, which no machine load can move.
    shared_hit_blocks: f64,
    loops: f64,
}

impl Scan {
    /// Shared buffers one execution of the node touched.
    fn blocks_per_loop(&self) -> f64 {
        self.shared_hit_blocks / self.loops.max(1.0)
    }
}

/// Every relation-reading node of a plan tree, outermost first.
fn scans(plan: &Value) -> Vec<Scan> {
    fn walk(node: &Value, out: &mut Vec<Scan>) {
        if let Some(relation) = node.get("Relation Name").and_then(Value::as_str) {
            out.push(Scan {
                node_type: node
                    .get("Node Type")
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned(),
                relation: relation.to_owned(),
                index: node
                    .get("Index Name")
                    .and_then(Value::as_str)
                    .map(ToOwned::to_owned),
                rows: node
                    .get("Actual Rows")
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
                executed: node
                    .get("Actual Loops")
                    .and_then(Value::as_i64)
                    .is_some_and(|loops| loops > 0),
                shared_hit_blocks: node
                    .get("Shared Hit Blocks")
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
                loops: node
                    .get("Actual Loops")
                    .and_then(Value::as_f64)
                    .unwrap_or_default(),
            });
        }
        // An index name can also sit on a Bitmap Index Scan, whose own node
        // carries no relation name; it is reported under its heap parent.
        if let (None, Some(index)) = (
            node.get("Relation Name"),
            node.get("Index Name").and_then(Value::as_str),
        ) && let Some(last) = out.last_mut()
            && last.index.is_none()
        {
            last.index = Some(index.to_owned());
        }
        for child in node
            .get("Plans")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            walk(child, out);
        }
    }
    let mut out = Vec::new();
    let root = plan
        .get(0)
        .and_then(|first| first.get("Plan"))
        .expect("EXPLAIN FORMAT JSON returns a one-element array carrying a Plan");
    walk(root, &mut out);
    out
}

/// A one-line-per-scan rendering, so a failed assertion says what the planner
/// did instead of only what it should have done.
fn render(scans: &[Scan]) -> String {
    scans
        .iter()
        .map(|scan| {
            format!(
                "  {} on {} via {} → {} rows{}",
                scan.node_type,
                scan.relation,
                scan.index.as_deref().unwrap_or("(no index)"),
                scan.rows,
                if scan.executed {
                    ""
                } else {
                    " (never executed)"
                }
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The scans of `relation`, which for a partitioned relation is the set of
/// partitions the plan did not prune.
fn on<'a>(scans: &'a [Scan], relation: &str) -> Vec<&'a Scan> {
    scans
        .iter()
        .filter(|scan| scan.relation == relation)
        .collect()
}

/// The distinct partitions of `relation` a plan read, in name order.
fn partitions<'a>(scans: &'a [Scan], relation: &str) -> Vec<&'a str> {
    let mut names: Vec<&str> = scans
        .iter()
        .map(|scan| scan.relation.as_str())
        .filter(|name| name.starts_with(relation))
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// Assert that no plan node carries a row-security qualifier.
///
/// H9: the instance is single-tenant, so no policy exists to attach a qual or
/// an `InitPlan` to any scan. The whole plan is searched as text because a policy
/// qual can surface as a `Filter`, an `Index Cond` or a subplan, and any of the
/// three would be a regression.
fn assert_no_policy_qual(plan: &Value, scans: &[Scan]) {
    let text = plan.to_string();
    for token in ["tenant", "current_setting"] {
        assert!(
            !text.contains(token),
            "H9: no row-security qual reaches a hot path's plan, and none of \
             them reads a session setting ({token}):\n{}",
            render(scans)
        );
    }
}

// ── the hot paths ────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_current_version_read_probes_the_head_row_by_primary_key() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let seeded = seed(&svc, &pool).await;

    let mut tx = explain_tx(&pool).await;
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{EXPLAIN}{}",
        read::READ_CURRENT_SQL
    )))
    .bind(seeded.vo)
    .fetch_one(&mut *tx)
    .await
    .expect("explain the current-version read");
    tx.rollback().await.expect("roll the explain back");

    let scans = scans(&plan);
    // H3: "what is current" is one primary-key probe on the head row.
    let head = on(&scans, "vo_head");
    assert_eq!(head.len(), 1, "one vo_head probe:\n{}", render(&scans));
    assert_eq!(
        head[0].index.as_deref(),
        Some("pk_vo_head"),
        "H3: the head row is reached by its primary key:\n{}",
        render(&scans)
    );
    assert_eq!(
        head[0].node_type,
        "Index Scan",
        "H3: the head probe is an index scan:\n{}",
        render(&scans)
    );
    // H3: the version row is reached by an index too, never by a scan of the
    // partition.
    let hot = on(&scans, "version_hot");
    assert_eq!(hot.len(), 1, "one version_hot probe:\n{}", render(&scans));
    assert!(
        hot[0].index.is_some() && hot[0].node_type != "Seq Scan",
        "H3: the version row is reached by an index:\n{}",
        render(&scans)
    );
    assert_no_policy_qual(&plan, &scans);
}

#[tokio::test]
async fn version_at_time_probes_the_descending_commit_index() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let seeded = seed(&svc, &pool).await;

    let mut tx = explain_tx(&pool).await;
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{EXPLAIN}{}",
        read::VERSION_AT_SQL
    )))
    .bind(seeded.vo)
    .bind(seeded.first_committed.to_string())
    .fetch_one(&mut *tx)
    .await
    .expect("explain the version-at-time read");
    tx.rollback().await.expect("roll the explain back");

    let scans = scans(&plan);
    // H3: the trunk row in force at an instant is one descending probe on
    // (vo_id, committed_at DESC) WHERE branch_number = 0.
    assert!(
        scans
            .iter()
            .any(|scan| scan.index.as_deref() == Some("idx_version_hot_trunk_at_time")),
        "H3: version_at_time is served by its own descending commit index:\n{}",
        render(&scans)
    );
    assert_no_policy_qual(&plan, &scans);
}

#[tokio::test]
async fn the_if_match_check_probes_the_head_row_by_primary_key() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let seeded = seed(&svc, &pool).await;

    let mut tx = explain_tx(&pool).await;
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{EXPLAIN}{}",
        meta::CURRENT_VERSION_META_SCOPED_SQL
    )))
    .bind(seeded.vo)
    .bind(seeded.ehr)
    .fetch_one(&mut *tx)
    .await
    .expect("explain the If-Match identity read");
    tx.rollback().await.expect("roll the explain back");

    let scans = scans(&plan);
    // H1: the ETag the If-Match compare needs is answered from the one mutable
    // row per object, by its primary key.
    let head = on(&scans, "vo_head");
    assert_eq!(head.len(), 1, "one vo_head probe:\n{}", render(&scans));
    assert_eq!(
        head[0].index.as_deref(),
        Some("pk_vo_head"),
        "H1: the If-Match check reaches the head row by primary key:\n{}",
        render(&scans)
    );
    assert_no_policy_qual(&plan, &scans);
}

#[tokio::test]
async fn the_revision_history_walks_one_object_by_index() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let seeded = seed(&svc, &pool).await;

    let mut tx = explain_tx(&pool).await;
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{EXPLAIN}{}",
        meta::ALL_VERSION_META_SQL
    )))
    .bind(seeded.vo)
    .fetch_one(&mut *tx)
    .await
    .expect("explain the revision history");
    tx.rollback().await.expect("roll the explain back");

    let scans = scans(&plan);
    // H3: a revision history is the version rows of ONE object, reached by an
    // index on vo_id rather than by a scan of the tier.
    let hot = on(&scans, "version_hot");
    assert_eq!(hot.len(), 1, "one version_hot node:\n{}", render(&scans));
    assert_eq!(
        hot[0].index.as_deref(),
        Some("idx_version_hot_lineage_tip"),
        "H3: the history walks the vo_id-leading index over one object's rows:\n{}",
        render(&scans)
    );
    assert!(
        (hot[0].rows - 3.0).abs() < f64::EPSILON,
        "the seeded container's three versions come back:\n{}",
        render(&scans)
    );
    assert_no_policy_qual(&plan, &scans);
}

#[tokio::test]
async fn a_point_read_on_the_partitioned_parent_probes_both_partitions() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let seeded = seed(&svc, &pool).await;

    let mut tx = explain_tx(&pool).await;
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{EXPLAIN}{}",
        read::READ_VERSION_BY_ORDINAL_SQL
    )))
    .bind(seeded.vo)
    .bind(seeded.sys_version)
    .fetch_one(&mut *tx)
    .await
    .expect("explain the by-ordinal point read");
    tx.rollback().await.expect("roll the explain back");

    let scans = scans(&plan);
    // H5: a point read names no tier, so both partitions are probed and one of
    // them returns nothing — the stated cost of partitioning by tier.
    let hot = on(&scans, "version_hot");
    let cold = on(&scans, "version_cold");
    assert_eq!(hot.len(), 1, "one hot probe:\n{}", render(&scans));
    assert_eq!(cold.len(), 1, "one cold probe:\n{}", render(&scans));
    assert!(
        hot[0].index.is_some() && cold[0].index.is_some(),
        "H5: both probes go through an index, not a partition scan:\n{}",
        render(&scans)
    );
    assert!(
        cold[0].rows.abs() < f64::EPSILON && hot[0].rows > 0.0,
        "H5: the second probe is the empty one — the archived EHR holds no \
         version of this object:\n{}",
        render(&scans)
    );
    assert_no_policy_qual(&plan, &scans);
}

// ── the head-row update ──────────────────────────────────────────────────────

/// `(n_tup_upd, n_tup_hot_upd)` of `clinical.vo_head`.
///
/// The session's statistics snapshot is dropped first: a session reuses the
/// snapshot it first read within a transaction
/// (<https://www.postgresql.org/docs/18/monitoring-stats.html>).
async fn head_update_counters(pool: &PgPool) -> (i64, i64) {
    drop(
        sqlx::query("SELECT pg_stat_clear_snapshot()")
            .execute(pool)
            .await,
    );
    sqlx::query_as(
        "SELECT n_tup_upd, n_tup_hot_upd FROM pg_stat_user_tables \
         WHERE schemaname = 'clinical' AND relname = 'vo_head'",
    )
    .fetch_one(pool)
    .await
    .expect("read the vo_head update counters")
}

/// Head-row updates the heap-only assertion counts.
const SUPERSESSIONS: i64 = 3;

#[tokio::test]
async fn the_head_row_update_is_heap_only() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr = svc.create_ehr(None).await.expect("create an EHR");
    let body = composition("heap-only head");
    let mut committed = svc
        .create_composition(ehr, uv(&body, "249", None))
        .await
        .expect("commit the first version");

    let (upd_before, hot_before) = head_update_counters(&pool).await;
    for _ in 0..SUPERSESSIONS {
        let preceding = committed.version_uid();
        committed = svc
            .update_composition(ehr, committed.vo_id, uv(&body, "251", Some(&preceding)))
            .await
            .expect("supersede the current version");
    }

    // A backend flushes its pending statistics when it next goes idle, so the
    // counters are polled until the supersessions have arrived rather than
    // read once after a guessed delay.
    let mut counters = head_update_counters(&pool).await;
    for _ in 0..100u32 {
        if counters.0 - upd_before >= SUPERSESSIONS {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        counters = head_update_counters(&pool).await;
    }
    let (updated, heap_only) = (counters.0 - upd_before, counters.1 - hot_before);
    assert!(
        updated >= SUPERSESSIONS,
        "the supersessions reached the statistics collector: {updated} updates recorded"
    );
    // H1: the columns a commit writes on the head row are in no index, so every
    // one of those updates is heap-only — no index insert, and the new tuple
    // stays on its own page ("Heap-Only Tuples (HOT)",
    // <https://www.postgresql.org/docs/18/storage-hot.html>).
    assert_eq!(
        heap_only, updated,
        "H1: every commit's head-row update is heap-only ({heap_only} of {updated})"
    );
}

// ── AQL ──────────────────────────────────────────────────────────────────────

/// Lower an AQL statement to the SQL the engine runs, scoped to `ehr_ids`
/// (empty = the population).
fn aql_sql(aql: &str, ehr_ids: Vec<EhrId>) -> ferroehr::aql::sql::PreparedQuery {
    let ast = openehr_query::parser::parse_str(aql).expect("the AQL parses");
    let params = Params::new();
    let ir = ferroehr::aql::plan(&ast, &params, SpecProfile::default()).expect("the AQL plans");
    let ctx = SqlCtx {
        system_id: "plan.shapes.test".to_owned(),
        ehr_ids,
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit: None,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    ferroehr::aql::sql::build(&ir, &params, &ctx).expect("the AQL lowers to SQL")
}

/// `EXPLAIN` a lowered AQL statement inside a rolled-back transaction.
async fn explain_aql(pool: &PgPool, prepared: ferroehr::aql::sql::PreparedQuery) -> Value {
    let mut tx = explain_tx(pool).await;
    let plan: Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("{EXPLAIN}{}", prepared.sql)),
        prepared.values,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("explain the AQL statement");
    tx.rollback().await.expect("roll the explain back");
    plan
}

/// The CONTAINS chain both AQL tests run: every COMPOSITION of the store.
const CONTAINS_AQL: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";

#[tokio::test]
async fn aql_over_one_ehr_reads_the_hot_partition_only() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let seeded = seed(&svc, &pool).await;

    let plan = explain_aql(&pool, aql_sql(CONTAINS_AQL, vec![seeded.ehr])).await;
    let scans = scans(&plan);
    // H4: the emitter writes `tier = 'hot'` as a literal, so exactly one
    // partition of each partitioned relation is left in the plan — the cold
    // ones are pruned at plan time and never appear at all.
    assert_eq!(
        partitions(&scans, "node"),
        vec!["node_hot"],
        "H4: an EHR-scoped query reads the hot node partition and no other:\n{}",
        render(&scans)
    );
    assert_eq!(
        partitions(&scans, "version"),
        vec!["version_hot"],
        "H4: and the hot version partition and no other:\n{}",
        render(&scans)
    );
    assert_head_join_by_key(&scans);
    assert!(
        scans
            .iter()
            .filter(|scan| scan.relation.starts_with("node"))
            .all(|scan| scan.index.is_some()),
        "H4: an EHR-scoped CONTAINS enters through an index, not a partition \
         scan:\n{}",
        render(&scans)
    );
    assert_no_policy_qual(&plan, &scans);
}

#[tokio::test]
async fn aql_over_the_population_reads_the_hot_partition_only() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    seed(&svc, &pool).await;

    let plan = explain_aql(&pool, aql_sql(CONTAINS_AQL, Vec::new())).await;
    let scans = scans(&plan);
    // H4: a population query is unaffected by the archive's size, because the
    // cold partitions are pruned at plan time rather than filtered at run time.
    assert_eq!(
        partitions(&scans, "node"),
        vec!["node_hot"],
        "H4: a population query reads the hot node partition and no other:\n{}",
        render(&scans)
    );
    assert_eq!(
        partitions(&scans, "version"),
        vec!["version_hot"],
        "H4: and the hot version partition and no other:\n{}",
        render(&scans)
    );
    assert_head_join_by_key(&scans);
    // H9: nothing attaches a row-security qual to the population scan.
    assert_no_policy_qual(&plan, &scans);
}

/// The `LATEST_VERSION` binding under `CONTAINS` costs one key probe per
/// object, not a walk of its trunk (#3453).
///
/// The head row is reached by `pk_vo_head`, and the version row through its
/// primary key with `sys_version` bound, never through the at-time index,
/// which is the only index a `vo_id`-alone probe can use and which made the
/// planner read every trunk version of the object per node row (75 601 of
/// 93 255 buffers in the class-`s` population `CONTAINS` recorded on #3350).
/// A probe of a row by a three-column key touches the index's root and leaf
/// and the heap page: a ceiling of eight buffers a loop holds that reading
/// and fails the walk, whose per-loop cost was about twenty-two.
fn assert_head_join_by_key(scans: &[Scan]) {
    let heads = on(scans, "vo_head");
    assert!(
        !heads.is_empty()
            && heads
                .iter()
                .all(|s| s.index.as_deref() == Some("pk_vo_head")),
        "#3453: the head row is reached by its primary key:\n{}",
        render(scans)
    );
    for version in on(scans, "version_hot") {
        assert_ne!(
            version.index.as_deref(),
            Some("idx_version_hot_trunk_at_time"),
            "#3453: LATEST_VERSION must not walk the trunk through the at-time index:\n{}",
            render(scans)
        );
        assert!(
            version.blocks_per_loop() <= 8.0,
            "#3453: a version probe costs a key lookup, not a walk ({} buffers a loop):\n{}",
            version.blocks_per_loop(),
            render(scans)
        );
    }
}

// ── the commit-time range ────────────────────────────────────────────────────

/// A commit-time range over the append-only version store — the shape
/// `idx_version_hot_committed_brin` is declared for.
///
/// The index exists because `committed_at` is physically correlated with
/// insertion order on a table no row is ever updated in, which is the case BRIN
/// is for (<https://www.postgresql.org/docs/18/brin-intro.html>). The statement
/// names no tier, so it reads both partitions: every shipped time-window
/// listing filters the `commit_audit.time_committed` copy of the same instant
/// instead of this column, so what this test pins is that the index still
/// serves the range it was declared for.
const COMMITTED_RANGE_SQL: &str =
    "SELECT count(*) FROM version WHERE committed_at >= $1::timestamptz";

#[tokio::test]
async fn a_commit_time_range_serves_from_the_brin_index() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let seeded = seed(&svc, &pool).await;

    let mut tx = explain_tx(&pool).await;
    let plan: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{EXPLAIN}{COMMITTED_RANGE_SQL}"
    )))
    .bind(seeded.first_committed.to_string())
    .fetch_one(&mut *tx)
    .await
    .expect("explain the commit-time range");
    tx.rollback().await.expect("roll the explain back");

    let scans = scans(&plan);
    // H7: the range is served by a bitmap scan over the BRIN index — a BRIN
    // index can only ever be reached that way (PostgreSQL 18, "BRIN Indexes").
    let hot = on(&scans, "version_hot");
    assert_eq!(hot.len(), 1, "one version_hot node:\n{}", render(&scans));
    assert_eq!(
        hot[0].node_type,
        "Bitmap Heap Scan",
        "H7: the commit-time range is a bitmap heap scan:\n{}",
        render(&scans)
    );
    assert_eq!(
        hot[0].index.as_deref(),
        Some("idx_version_hot_committed_brin"),
        "H7: the bitmap comes from the BRIN index on committed_at:\n{}",
        render(&scans)
    );
    // The cold partition is read sequentially, and deliberately so: it carries
    // the primary key and the tree uniqueness alone, no commit-time index, so
    // no other path exists for this predicate — which is what "cannot suppress
    // sequential scans entirely" means here
    // (<https://www.postgresql.org/docs/18/runtime-config-query.html>).
    let cold = on(&scans, "version_cold");
    assert_eq!(cold.len(), 1, "one version_cold node:\n{}", render(&scans));
    assert!(
        cold[0].index.is_none(),
        "the archived tier carries no commit-time index:\n{}",
        render(&scans)
    );
}
