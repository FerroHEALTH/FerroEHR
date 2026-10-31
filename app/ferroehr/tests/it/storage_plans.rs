// SPDX-FileCopyrightText: Cadasto B.V.
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

use serde_json::{Value, json};
use sqlx::{Executor as _, PgPool, Postgres, Row as _, Transaction};

use ferroehr::aql::ir::{ParamValue, Params};
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

/// The lookup the foreign-key trigger of `node` and `vo_attestation` runs on
/// the referenced `version` for every inserted row, as `pg_stat_statements`
/// records it (`PostgreSQL` docs § Foreign Keys,
/// <https://www.postgresql.org/docs/18/ddl-constraints.html#DDL-CONSTRAINTS-FK>).
const FK_VERSION_LOOKUP_SQL: &str = "SELECT 1 FROM version x \
     WHERE tier OPERATOR(pg_catalog.=) $1 AND vo_id OPERATOR(pg_catalog.=) $2 \
     AND sys_version OPERATOR(pg_catalog.=) $3 FOR KEY SHARE OF x";

/// Every `Index Cond` a plan tree applies to an index of `version_hot`.
fn version_hot_index_conds(node: &Value, out: &mut Vec<String>) {
    let on_version_hot = node
        .get("Index Name")
        .and_then(Value::as_str)
        .is_some_and(|index| index.contains("version_hot"));
    if on_version_hot && let Some(cond) = node.get("Index Cond").and_then(Value::as_str) {
        out.push(cond.to_owned());
    }
    for child in node
        .get("Plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        version_hot_index_conds(child, out);
    }
}

#[tokio::test]
async fn the_foreign_key_check_of_a_node_row_probes_one_version() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // Mostly single-version objects beside one long history: the statistics
    // under which the planner rated a scan of all of an object's versions as
    // cheap as the primary key (#4340).
    let ehr = svc.create_ehr(None).await.expect("create an EHR");
    let body = composition("foreign-key lookup");
    for _ in 0..300 {
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("commit a single-version composition");
    }
    let mut head = svc
        .create_composition(ehr, uv(&body, "249", None))
        .await
        .expect("commit the long history's first version");
    for _ in 0..40 {
        let preceding = head.version_uid();
        head = svc
            .update_composition(ehr, head.vo_id, uv(&body, "251", Some(&preceding)))
            .await
            .expect("extend the long history");
    }
    pool.execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded corpus");

    let mut tx = explain_tx(&pool).await;
    // A session switches to the generic plan from its sixth check on when it
    // costs no more than the custom plans did (`PostgreSQL` docs § PREPARE).
    // Sent as a simple query: an extended-protocol bind would have to supply
    // the three parameters the generic plan leaves open.
    let generic: Value = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
        "EXPLAIN (GENERIC_PLAN, FORMAT JSON) {FK_VERSION_LOOKUP_SQL}"
    )))
    .fetch_one(&mut *tx)
    .await
    .expect("explain the generic foreign-key lookup")
    .try_get(0)
    .expect("EXPLAIN FORMAT JSON returns one json column");
    let custom: Value = sqlx::query_scalar(sqlx::AssertSqlSafe(format!(
        "{EXPLAIN}{FK_VERSION_LOOKUP_SQL}"
    )))
    .bind("hot")
    .bind(head.vo_id)
    .bind(head.sys_version)
    .fetch_one(&mut *tx)
    .await
    .expect("explain the foreign-key lookup of the long history's head");
    tx.rollback().await.expect("roll the explain back");

    for (shape, plan) in [("generic", &generic), ("custom", &custom)] {
        let root = plan
            .get(0)
            .and_then(|first| first.get("Plan"))
            .expect("EXPLAIN FORMAT JSON returns a one-element array carrying a Plan");
        let mut conds = Vec::new();
        version_hot_index_conds(root, &mut conds);
        assert!(
            !conds.is_empty() && conds.iter().all(|cond| cond.contains("sys_version")),
            "the {shape} plan of the foreign-key lookup bounds its version_hot index \
             scan by sys_version, so it reads one version however long the history: \
             {conds:?}\n{}",
            render(&scans(plan))
        );
    }
}

/// Every `version_hot` node of a plan tree: its `Index Cond`, if any, and the
/// rows its `Filter` dropped.
fn version_hot_probes(node: &Value, out: &mut Vec<(Option<String>, f64)>) {
    if node
        .get("Relation Name")
        .and_then(Value::as_str)
        .is_some_and(|relation| relation == "version_hot")
    {
        out.push((
            node.get("Index Cond")
                .and_then(Value::as_str)
                .map(ToOwned::to_owned),
            node.get("Rows Removed by Filter")
                .and_then(Value::as_f64)
                .unwrap_or_default(),
        ));
    }
    for child in node
        .get("Plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        version_hot_probes(child, out);
    }
}

#[tokio::test]
async fn a_read_by_version_tree_id_probes_one_version() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // Single-version objects beside one long history, as in the foreign-key
    // probe above; the read addresses a trunk version in the middle of it.
    let ehr = svc.create_ehr(None).await.expect("create an EHR");
    let body = composition("tree id read");
    for _ in 0..300 {
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("commit a single-version composition");
    }
    let mut head = svc
        .create_composition(ehr, uv(&body, "249", None))
        .await
        .expect("commit the long history's first version");
    for _ in 0..60 {
        let preceding = head.version_uid();
        head = svc
            .update_composition(ehr, head.vo_id, uv(&body, "251", Some(&preceding)))
            .await
            .expect("extend the long history");
    }

    // A session switches to the generic plan from its sixth execution on when
    // it costs no more than the custom plans did (`PostgreSQL` docs § PREPARE),
    // so the generic plan is the one a pooled connection holds, with and
    // without statistics. Before any ANALYZE every candidate index of a custom
    // plan estimates one row at the same cost, which no index layout decides.
    let mut unbounded = Vec::new();
    for (statistics, modes) in [
        ("without ANALYZE", &["force_generic_plan"][..]),
        (
            "after ANALYZE",
            &["force_custom_plan", "force_generic_plan"][..],
        ),
    ] {
        if statistics == "after ANALYZE" {
            pool.execute("ANALYZE")
                .await
                .expect("give the planner statistics over the seeded corpus");
        }
        for mode in modes {
            let mut tx = explain_tx(&pool).await;
            tx.execute(sqlx::AssertSqlSafe(format!(
                "SET LOCAL plan_cache_mode = {mode}"
            )))
            .await
            .expect("set the plan cache mode");
            tx.execute(sqlx::AssertSqlSafe(format!(
                "PREPARE tree_read (uuid, int, int, int) AS {}",
                read::READ_VERSION_SQL
            )))
            .await
            .expect("prepare the by-tree read");
            let plan: Value = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "{EXPLAIN}EXECUTE tree_read ('{}', 30, 0, 0)",
                head.vo_id.0
            )))
            .fetch_one(&mut *tx)
            .await
            .expect("explain the by-tree read")
            .try_get(0)
            .expect("EXPLAIN FORMAT JSON returns one json column");
            // A prepared statement outlives the transaction it was made in.
            tx.execute("DEALLOCATE tree_read")
                .await
                .expect("drop the prepared by-tree read");
            tx.rollback().await.expect("roll the explain back");

            let root = plan
                .get(0)
                .and_then(|first| first.get("Plan"))
                .expect("EXPLAIN FORMAT JSON returns a one-element array carrying a Plan");
            let mut probes = Vec::new();
            version_hot_probes(root, &mut probes);
            let bounded = matches!(
                probes.as_slice(),
                [(Some(cond), removed)]
                    if ["vo_id", "trunk_version"].iter().all(|column| cond.contains(column))
                        && removed.abs() < f64::EPSILON
            );
            if !bounded {
                unbounded.push(format!(
                    "{statistics}, {mode}: {probes:?}\n{}",
                    render(&scans(&plan))
                ));
            }
        }
    }
    assert!(
        unbounded.is_empty(),
        "the by-tree read reaches its version through an index on the tree id columns \
         and filters out no other version of the object:\n{}",
        unbounded.join("\n")
    );
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
    aql_sql_limited(aql, ehr_ids, None)
}

/// [`aql_sql`] with a row limit, as REST `fetch` or AQL `LIMIT` sets it.
fn aql_sql_limited(
    aql: &str,
    ehr_ids: Vec<EhrId>,
    limit: Option<i64>,
) -> ferroehr::aql::sql::PreparedQuery {
    aql_sql_with(aql, &Params::new(), ehr_ids, limit)
}

/// [`aql_sql_limited`] with the query parameters bound.
fn aql_sql_with(
    aql: &str,
    params: &Params,
    ehr_ids: Vec<EhrId>,
    limit: Option<i64>,
) -> ferroehr::aql::sql::PreparedQuery {
    let ast = openehr_query::parser::parse_str(aql).expect("the AQL parses");
    let ir = ferroehr::aql::plan(&ast, params, SpecProfile::default()).expect("the AQL plans");
    let ctx = SqlCtx {
        system_id: "plan.shapes.test".to_owned(),
        ehr_ids,
        ehr_scope_kind: ferroehr::aql::sql::EhrScopeKind::Named,
        subject_scope: None,
        ehr_access: ferroehr::aql::sql::EhrAccessScope::Unrestricted,
        limit,
        offset: None,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
    };
    ferroehr::aql::sql::build(&ir, params, &ctx).expect("the AQL lowers to SQL")
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

/// The archetype columns carry one statistics object in each store, so the
/// planner sees that an archetype fixes its RM type, entity and major version
/// instead of multiplying their selectivities (PostgreSQL 18, "Extended
/// Statistics"; clinical #4399, party #4437).
#[tokio::test]
async fn the_archetype_columns_carry_dependency_statistics() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    for schema in ["clinical", "party"] {
        let (columns, kinds): (Vec<String>, Vec<String>) = sqlx::query_as(
            "SELECT ARRAY(SELECT a.attname::text FROM unnest(s.stxkeys) AS k(n) \
                      JOIN pg_attribute a ON a.attrelid = s.stxrelid AND a.attnum = k.n \
                      ORDER BY a.attnum), \
                s.stxkind::text[] \
         FROM pg_statistic_ext s \
         WHERE s.stxname = 'st_node_hot_archetype' \
           AND s.stxrelid = $1::regclass",
        )
        .bind(format!("{schema}.node_hot"))
        .fetch_one(&pool)
        .await
        .expect("the archetype statistics object of each store exists");
        assert_eq!(
            columns,
            ["rm_type", "arch_entity", "arch_concept", "arch_major"],
            "{schema}"
        );
        assert!(
            kinds.contains(&"f".to_owned()),
            "{schema} dependencies: {kinds:?}"
        );
        assert!(
            kinds.contains(&"m".to_owned()),
            "{schema} most common values: {kinds:?}"
        );
    }
}

/// A population count of a whole object reads no node row (#4286).
///
/// `COUNT(c)` and `COUNT(*)` ask only whether each COMPOSITION is there
/// (QUERY master03 §COUNT), which the current version row of an object that
/// is not logically deleted answers; probing `node` for every root row cost
/// 25 s at 500 000 compositions.
#[tokio::test]
async fn a_population_count_of_whole_objects_reads_no_node() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    seed(&svc, &pool).await;

    for aql in [
        "SELECT count(c) FROM EHR e CONTAINS COMPOSITION c",
        "SELECT count(*) FROM EHR e CONTAINS COMPOSITION c",
    ] {
        let plan = explain_aql(&pool, aql_sql(aql, Vec::new())).await;
        let scans = scans(&plan);
        assert!(
            partitions(&scans, "node").is_empty(),
            "{aql} reads no node partition:\n{}",
            render(&scans)
        );
        assert_eq!(
            partitions(&scans, "version"),
            vec!["version_hot"],
            "{aql} reads the hot version partition:\n{}",
            render(&scans)
        );
    }
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

/// The newest compositions of the population, the first page: the ordering
/// key is the promoted start time of every COMPOSITION root.
const NEWEST_AQL: &str = "SELECT c/uid/value, c/context/start_time/value \
     FROM EHR e CONTAINS COMPOSITION c \
     ORDER BY c/context/start_time/value DESC, c/uid/value ASC";

/// A population-wide ordered page reads the compositions in start-time order
/// from `idx_node_hot_composition_start` and stops at the limit, so its cost
/// does not grow with the number of compositions (#4090).
#[tokio::test]
async fn the_newest_compositions_page_reads_the_start_time_index() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    seed(&svc, &pool).await;

    let plan = explain_aql(&pool, aql_sql_limited(NEWEST_AQL, Vec::new(), Some(10))).await;
    let scans = scans(&plan);
    assert!(
        on(&scans, "node_hot")
            .iter()
            .any(|s| s.index.as_deref() == Some("idx_node_hot_composition_start")),
        "#4090: the ordered page enters through the start-time index:\n{}",
        render(&scans)
    );
}

/// Every node type anywhere in a plan tree, outermost first.
fn node_types(node: &Value, out: &mut Vec<String>) {
    if let Some(node_type) = node.get("Node Type").and_then(Value::as_str) {
        out.push(node_type.to_owned());
    }
    for child in node
        .get("Plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        node_types(child, out);
    }
}

/// `EXPLAIN` an ordered population page under the server's own settings, plus
/// `extra` (a `SET LOCAL`), inside a rolled-back transaction.
async fn explain_ordered_page(pool: &PgPool, aql: &str, extra: Option<&str>) -> Value {
    let prepared = aql_sql_limited(aql, Vec::new(), Some(5));
    let mut tx = pool.begin().await.expect("begin the explain transaction");
    if let Some(setting) = extra {
        tx.execute(sqlx::AssertSqlSafe(setting.to_owned()))
            .await
            .expect("apply the planner setting");
    }
    let plan: Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("{EXPLAIN}{}", prepared.sql)),
        prepared.values,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("explain the ordered page");
    tx.rollback().await.expect("roll the explain back");
    plan
}

/// A population page ordered by the version uid runs in one backend, and
/// `idx_version_hot_uid` hands the versions over in that order (#4429).
///
/// Without the index the only plan that sorted the version rows before the
/// joins was a `Gather Merge`, which a pruned partition is offered however
/// small it is, so such a query started a parallel worker on every call or,
/// when the statistics tipped the other way, sorted the whole join. Over this
/// small seed a sort of a sequential scan is as good a serial plan, so the
/// index is read with sorting disabled, which only an index matching the
/// engine's uid expression can satisfy.
#[tokio::test]
async fn a_page_ordered_by_the_version_uid_reads_the_uid_index_serially() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    seed(&svc, &pool).await;

    for direction in ["ASC", "DESC"] {
        let aql = format!(
            "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
             ORDER BY c/uid/value {direction}"
        );
        let plan = explain_ordered_page(&pool, &aql, None).await;
        let mut types = Vec::new();
        node_types(
            plan.get(0)
                .and_then(|first| first.get("Plan"))
                .unwrap_or(&Value::Null),
            &mut types,
        );
        assert!(
            !types.iter().any(|t| t.starts_with("Gather")),
            "#4429: {aql} starts no parallel worker: {types:?}"
        );

        let plan = explain_ordered_page(&pool, &aql, Some("SET LOCAL enable_sort = off")).await;
        let scans = scans(&plan);
        assert!(
            on(&scans, "version_hot")
                .iter()
                .any(|s| s.index.as_deref() == Some("idx_version_hot_uid")),
            "#4429: {aql} can read the versions in uid order from the uid index:\n{}",
            render(&scans)
        );
        let mut types = Vec::new();
        node_types(
            plan.get(0)
                .and_then(|first| first.get("Plan"))
                .unwrap_or(&Value::Null),
            &mut types,
        );
        assert!(
            !types.iter().any(|t| t == "Sort" || t.starts_with("Gather")),
            "#4429: {aql} reads its order off the index, with no sort: {types:?}"
        );
    }
}

/// A template predicate on a COMPOSITION reads `idx_version_hot_template`
/// instead of extracting the template id from every root (#4090).
#[tokio::test]
async fn a_template_predicate_reads_the_template_index() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    seed(&svc, &pool).await;

    let plan = explain_aql(
        &pool,
        aql_sql_limited(
            "SELECT c FROM EHR e CONTAINS COMPOSITION c \
             WHERE c/archetype_details/template_id/value = 'no.such.template'",
            Vec::new(),
            Some(500),
        ),
    )
    .await;
    let scans = scans(&plan);
    assert!(
        on(&scans, "version_hot")
            .iter()
            .any(|s| s.index.as_deref() == Some("idx_version_hot_template")),
        "#4090: the template predicate enters through the template index:\n{}",
        render(&scans)
    );
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

// ── the row estimate of a chain of path bindings ─────────────────────────────

/// The archetype of the OBSERVATION the binding-chain tests commit.
const WIDE_ARCHETYPE: &str = "openEHR-EHR-OBSERVATION.plan_shapes.v1";

/// The at-codes of the ELEMENTs under that OBSERVATION's one event.
const WIDE_ITEMS: [&str; 8] = [
    "at0004", "at0005", "at0006", "at0007", "at0008", "at0009", "at0010", "at0011",
];

/// The path from the OBSERVATION to the magnitude of the ELEMENT `item`.
fn wide_leaf(item: &str) -> String {
    format!("o/data[at0001]/events[at0002]/data[at0003]/items[{item}]/value/magnitude")
}

/// A COMPOSITION carrying one OBSERVATION whose one event holds a
/// `DV_QUANTITY` ELEMENT per [`WIDE_ITEMS`] entry, the shape of the storage
/// bench's wide projection.
fn wide_observation_composition(name: &str, magnitude: f64) -> Value {
    let items: Vec<Value> = WIDE_ITEMS
        .iter()
        .zip(0_u8..)
        .map(|(item, offset)| {
            json!({
                "_type": "ELEMENT",
                "name": { "_type": "DV_TEXT", "value": format!("reading {item}") },
                "archetype_node_id": item,
                "value": {
                    "_type": "DV_QUANTITY",
                    "magnitude": magnitude + f64::from(offset),
                    "units": "mm[Hg]"
                }
            })
        })
        .collect();
    let mut body = composition(name);
    let content = json!([{
        "_type": "OBSERVATION",
        "name": { "_type": "DV_TEXT", "value": "Readings" },
        "archetype_node_id": WIDE_ARCHETYPE,
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": WIDE_ARCHETYPE },
            "rm_version": "1.1.0"
        },
        "language": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "en"
        },
        "encoding": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "IANA_character-sets" },
            "code_string": "UTF-8"
        },
        "subject": { "_type": "PARTY_SELF" },
        "data": {
            "_type": "HISTORY",
            "name": { "_type": "DV_TEXT", "value": "History" },
            "archetype_node_id": "at0001",
            "origin": { "_type": "DV_DATE_TIME", "value": "2026-01-01T10:00:00Z" },
            "events": [{
                "_type": "POINT_EVENT",
                "name": { "_type": "DV_TEXT", "value": "Any event" },
                "archetype_node_id": "at0002",
                "time": { "_type": "DV_DATE_TIME", "value": "2026-01-01T10:00:00Z" },
                "data": {
                    "_type": "ITEM_TREE",
                    "name": { "_type": "DV_TEXT", "value": "Tree" },
                    "archetype_node_id": "at0003",
                    "items": items
                }
            }]
        }
    }]);
    body.as_object_mut()
        .expect("the composition fixture is a JSON object")
        .insert("content".to_owned(), content);
    body
}

/// Every `Plan Rows` estimate of a plan tree, its subplans included.
fn plan_row_estimates(node: &Value, out: &mut Vec<f64>) {
    if let Some(rows) = node.get("Plan Rows").and_then(Value::as_f64) {
        out.push(rows);
    }
    for child in node
        .get("Plans")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        plan_row_estimates(child, out);
    }
}

/// A chain of path bindings is estimated at the rows it can produce, so its
/// cost neither picks the join order of a statement many times larger nor
/// switches on JIT compilation (#4430, #4431).
///
/// Each binding of a path step probes the children of one node row. Lowered
/// as a set-returning function over an array the planner cannot see into, a
/// binding was estimated at ten rows (PostgreSQL 18 `estimate_array_length`'s
/// default), so a chain of eleven multiplied the estimate by 10^11: the
/// storage bench's wide projection over one EHR spent a second in LLVM
/// compilation on every execution, because the planner JIT-compiles a
/// statement whose estimated cost passes `jit_above_cost` (PostgreSQL 18
/// §30.2 "When to JIT?"). The flat shape's bindings are probed in the same
/// order and are planned from their own subquery plans. No estimate of a
/// binding chain can exceed the node rows the store holds: each binding
/// yields the children of one row, or one all-`NULL` row.
#[tokio::test]
async fn a_chain_of_path_bindings_is_estimated_at_the_rows_it_reads() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let mut ehrs = Vec::new();
    for index in 0..4_u8 {
        let ehr = svc.create_ehr(None).await.expect("create an EHR");
        for reading in 0..10_u8 {
            let body = wide_observation_composition(
                &format!("readings {index}.{reading}"),
                f64::from(reading) * 10.0,
            );
            svc.create_composition(ehr, uv(&body, "249", None))
                .await
                .expect("commit an observation");
        }
        ehrs.push(ehr);
    }
    pool.execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded corpus");
    let node_rows: i64 = sqlx::query_scalar("SELECT count(*) FROM node")
        .fetch_one(&pool)
        .await
        .expect("count the node rows");

    let magnitude = wide_leaf(WIDE_ITEMS[0]);
    let ordered = format!(
        "SELECT c/uid/value, {magnitude} FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[{WIDE_ARCHETYPE}] \
         WHERE {magnitude} >= 0 ORDER BY {magnitude} DESC"
    );
    let wide = format!(
        "SELECT c/uid/value, {} FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[{WIDE_ARCHETYPE}]",
        WIDE_ITEMS.map(wide_leaf).join(", ")
    );
    let one_ehr = ehrs.first().copied().expect("a seeded EHR");
    for (name, prepared, expected) in [
        (
            "the ordered population page",
            aql_sql_limited(&ordered, Vec::new(), Some(50)),
            40,
        ),
        (
            "the wide projection over one EHR",
            aql_sql(&wide, vec![one_ehr]),
            10,
        ),
    ] {
        let plan = explain_aql(&pool, prepared).await;
        let root = plan
            .get(0)
            .expect("EXPLAIN FORMAT JSON returns a one-element array");
        let returned = root
            .pointer("/Plan/Actual Rows")
            .and_then(Value::as_f64)
            .expect("the root plan node reports its rows");
        assert!(
            (returned - f64::from(expected)).abs() < f64::EPSILON,
            "{name} returns {expected} rows, so the statement read what was seeded: {returned}"
        );
        let mut estimates = Vec::new();
        plan_row_estimates(root.get("Plan").expect("a Plan"), &mut estimates);
        let widest = estimates.iter().copied().fold(0.0, f64::max);
        let ceiling = f64::from(
            i32::try_from(node_rows).expect("a test seed holds fewer than 2^31 node rows"),
        );
        assert!(
            widest <= ceiling,
            "#4430, #4431: no plan node of {name} is estimated at more rows than the {node_rows} \
             node rows the store holds ({widest}):\n{}",
            render(&scans(&plan))
        );
        assert!(
            root.get("JIT").is_none(),
            "#4430, #4431: {name} is not JIT-compiled: {}",
            root.get("JIT").map(Value::to_string).unwrap_or_default()
        );
    }
}

// ── a projection over named EHRs whatever the archetype statistics (#4487) ──

/// `EXPLAIN` `prepared` inside a rolled-back transaction after `statistics`
/// re-collected the planner statistics of `node_hot` in that transaction.
async fn explain_after(
    pool: &PgPool,
    statistics: &str,
    prepared: ferroehr::aql::sql::PreparedQuery,
) -> Value {
    let mut tx = explain_tx(pool).await;
    sqlx::raw_sql(sqlx::AssertSqlSafe(statistics))
        .execute(&mut *tx)
        .await
        .expect("re-collect the statistics for this transaction");
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

/// A projection over the EHRs its caller named reads those EHRs' objects
/// first and each object's own nodes after them, whatever the archetype
/// statistics say (#4487).
///
/// When an archetype's rows fall out of the most-common-value list of
/// `st_node_hot_archetype`, as the storage bench's commit group made them do
/// in some rounds by diluting the sample, the planner estimates them at one
/// row. A contained node that was a plain FROM item then led the join through
/// `idx_node_hot_arch_subsume` over the whole population and ran every path
/// binding of the projection for each of those rows before the EHR filter:
/// the bench's wide projection over one EHR read about 10 000 pages per call
/// in those rounds and 1 200 in the others. The statement now fences the
/// contained node inside its parent, so the plan is the same with the list
/// cut to one entry and with it whole.
#[tokio::test]
async fn a_projection_over_named_ehrs_holds_its_plan_whatever_the_archetype_statistics() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let mut ehrs = Vec::new();
    for index in 0..8_u8 {
        let ehr = svc.create_ehr(None).await.expect("create an EHR");
        for reading in 0..5_u8 {
            let body = wide_observation_composition(
                &format!("readings {index}.{reading}"),
                f64::from(reading) * 10.0,
            );
            svc.create_composition(ehr, uv(&body, "249", None))
                .await
                .expect("commit an observation");
        }
        ehrs.push(ehr);
    }
    analyze_as_autovacuum(&pool).await;
    ferroehr::db::parent_statistics::refresh_pool(&pool)
        .await
        .expect("refresh the partitioned parents' statistics");

    let wide = format!(
        "SELECT c/uid/value, {} FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[{WIDE_ARCHETYPE}]",
        WIDE_ITEMS.map(wide_leaf).join(", ")
    );
    let one_ehr = ehrs.first().copied().expect("a seeded EHR");
    let mut shapes = Vec::new();
    for (state, statistics) in [
        (
            "the whole most-common-value list",
            "ANALYZE clinical.node_hot",
        ),
        (
            "the list cut to one entry",
            "ALTER STATISTICS clinical.st_node_hot_archetype SET STATISTICS 1; \
             ANALYZE clinical.node_hot",
        ),
    ] {
        let plan = explain_after(&pool, statistics, aql_sql(&wide, vec![one_ehr])).await;
        let returned = plan
            .pointer("/0/Plan/Actual Rows")
            .and_then(Value::as_f64)
            .expect("the root plan node reports its rows");
        assert!(
            (returned - 5.0).abs() < f64::EPSILON,
            "{state}: the projection returns the named EHR's five observations: {returned}"
        );
        let scans = scans(&plan);
        assert!(
            on(&scans, "node_hot")
                .iter()
                .all(|scan| scan.index.as_deref() != Some("idx_node_hot_arch_subsume")),
            "#4487, {state}: a projection over a named EHR never enters through the \
             population's archetype index:\n{}",
            render(&scans)
        );
        let mut indexes: Vec<String> = scans
            .iter()
            .map(|scan| {
                format!(
                    "{} via {}",
                    scan.relation,
                    scan.index.as_deref().unwrap_or("-")
                )
            })
            .collect();
        indexes.sort_unstable();
        indexes.dedup();
        shapes.push((state, indexes, render(&scans)));
    }
    let (_, whole, whole_plan) = shapes.first().expect("the whole list was explained");
    let (_, cut, cut_plan) = shapes.get(1).expect("the cut list was explained");
    assert_eq!(
        whole, cut,
        "#4487: the projection reads the same relations through the same indexes \
         whatever the archetype statistics:\nwhole list:\n{whole_plan}\ncut list:\n{cut_plan}"
    );
}

// ── a lookup by an ELEMENT value (#4437) ─────────────────────────────────────

/// The `person_identifier` CLUSTER archetype the lookup walks.
const PERSON_IDENTIFIER_CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2";

/// A partner's lookup of a person by an identifier value (#4437).
const PERSON_LOOKUP_AQL: &str = "SELECT p/uid/value, id/items[at0001]/value/type, \
     id/items[at0001]/value/issuer, id/items[at0001]/value/assigner \
     FROM PERSON p CONTAINS CLUSTER id[openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2] \
     WHERE id/items[at0001]/value/id = $v";

/// The identifier of the `index`th seeded person.
fn person_identifier(index: u32) -> String {
    format!("MRN-{index:07}")
}

/// A PERSON carrying one `person_identifier` CLUSTER whose ELEMENT holds a
/// `DV_IDENTIFIER` with its issuer, assigner and type, the shape whose value
/// stays in the stored fragment (`crate::storage::slim` moves a bare id to
/// the typed columns and nothing wider).
fn person_with_identifier(index: u32) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": format!("Person {index}") },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0003",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0004",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0005",
                    "name": { "_type": "DV_TEXT", "value": "name" },
                    "value": { "_type": "DV_TEXT", "value": format!("Person {index}") }
                }]
            }
        }],
        "details": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "details" },
            "items": [{
                "_type": "CLUSTER",
                "archetype_node_id": PERSON_IDENTIFIER_CLUSTER,
                "archetype_details": { "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": PERSON_IDENTIFIER_CLUSTER },
                    "rm_version": "1.1.0" },
                "name": { "_type": "DV_TEXT", "value": "person identifier" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0001",
                    "name": { "_type": "DV_TEXT", "value": "identifier" },
                    "value": { "_type": "DV_IDENTIFIER", "issuer": "urn:issuer:hospital",
                        "assigner": "urn:issuer:hospital", "type": "MRN",
                        "id": person_identifier(index) }
                }]
            }]
        }
    })
}

/// The at-code of the one `DV_TEXT` ELEMENT of [`text_observation_composition`].
const TEXT_ITEM: &str = "at0004";

/// The path from the OBSERVATION to the text of that ELEMENT.
fn text_leaf() -> String {
    format!("o/data[at0001]/events[at0002]/data[at0003]/items[{TEXT_ITEM}]/value/value")
}

/// A COMPOSITION carrying one OBSERVATION whose one event holds one `DV_TEXT`
/// ELEMENT reading `text`, the clinical shape of a lookup by a value.
fn text_observation_composition(name: &str, text: &str) -> Value {
    let mut body = wide_observation_composition(name, 0.0);
    let items = json!([{
        "_type": "ELEMENT",
        "name": { "_type": "DV_TEXT", "value": "note" },
        "archetype_node_id": TEXT_ITEM,
        "value": { "_type": "DV_TEXT", "value": text }
    }]);
    *body
        .pointer_mut("/content/0/data/events/0/data/items")
        .expect("the observation fixture holds its items") = items;
    body
}

/// Asserts that the plan of a lookup by an ELEMENT value found its one row
/// through `idx_node_hot_value_search` and read no scan of the population.
fn assert_value_lookup(name: &str, plan: &Value, population: u32) {
    let root = plan
        .get(0)
        .expect("EXPLAIN FORMAT JSON returns a one-element array");
    let returned = root
        .pointer("/Plan/Actual Rows")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its rows");
    assert!(
        (returned - 1.0).abs() < f64::EPSILON,
        "{name} finds the one object carrying the value: {returned} rows"
    );
    let scans = scans(plan);
    assert!(
        scans
            .iter()
            .any(|scan| scan.index.as_deref() == Some("idx_node_hot_value_search")),
        "#4437: {name} enters through the value index:\n{}",
        render(&scans)
    );
    let population = f64::from(population);
    for scan in on(&scans, "node_hot") {
        assert!(
            scan.rows * scan.loops < population,
            "#4437: no node scan of {name} walks the {population} objects of the store, \
             {} read {} rows over {} loops:\n{}",
            scan.node_type,
            scan.rows,
            scan.loops,
            render(&scans)
        );
    }
    let buffers = root
        .pointer("/Plan/Shared Hit Blocks")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its buffers");
    assert!(
        buffers < population,
        "#4437: {name} reads fewer buffers than the store holds objects ({buffers}):\n{}",
        render(&scans)
    );
}

/// A lookup of a person by an identifier value reads the one person (#4437).
///
/// A path binding is a correlated subquery, so the lookup read every
/// `person_identifier` CLUSTER of the store, probed its version, head and root
/// and tested the value last: 16 buffers a person, 350 ms at 50 000 persons.
/// The engine now anchors such an equality on the ELEMENT rows of the same
/// version by their search text (`ext.node_search_text`), which
/// `idx_node_hot_value_search` serves, on the flat shape and on the
/// streaming one (a LIMIT without an ORDER BY) alike.
#[tokio::test]
async fn a_lookup_by_a_person_identifier_reads_the_one_person() {
    const PERSONS: u32 = 200;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let pools = ferroehr::db::domain::DomainPools::from_shared(&pool);
    let svc = FerroEhrService::new(&pools);
    for index in 0..PERSONS {
        let body = person_with_identifier(index);
        svc.party_create(
            ferroehr::service::demographic::types::PartyKind::Person,
            openehr_its::json::from_canonical_value(&body).expect("the fixture is a PERSON"),
            None,
        )
        .await
        .expect("create a person");
    }
    pools
        .party
        .execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded parties");

    let params = Params::new().with(
        "v",
        ParamValue::Str(person_identifier(PERSONS.div_euclid(2))),
    );
    for (name, limit) in [("the lookup", None), ("the lookup with a limit", Some(10))] {
        let prepared = aql_sql_with(PERSON_LOOKUP_AQL, &params, Vec::new(), limit);
        let plan = explain_aql(&pools.party, prepared).await;
        assert_value_lookup(name, &plan, PERSONS);
    }
}

/// A lookup of a composition by a text value reads the one composition
/// (#4437): the clinical store serves an equality on an ELEMENT value the
/// same way.
#[tokio::test]
async fn a_lookup_by_an_element_text_reads_the_one_composition() {
    const COMPOSITIONS: u32 = 400;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    for index in 0..COMPOSITIONS {
        let ehr = svc.create_ehr(None).await.expect("create an EHR");
        let body =
            text_observation_composition(&format!("note {index}"), &format!("needle {index}"));
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("commit an observation");
    }
    pool.execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded corpus");

    let text = text_leaf();
    let aql = format!(
        "SELECT c/uid/value, {text} FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[{WIDE_ARCHETYPE}] WHERE {text} = $needle"
    );
    let params = Params::new().with(
        "needle",
        ParamValue::Str(format!("needle {}", COMPOSITIONS.div_euclid(2))),
    );
    for (name, limit) in [("the lookup", None), ("the lookup with a limit", Some(10))] {
        let prepared = aql_sql_with(&aql, &params, Vec::new(), limit);
        let plan = explain_aql(&pool, prepared).await;
        assert_value_lookup(name, &plan, COMPOSITIONS);
    }
}

// ── a lookup of an EHR by its subject (#4442) ────────────────────────────────

/// The `EHR_STATUS` of the `index`th seeded EHR: its subject is a `PARTY_REF`
/// to a PERSON by its uid, in the `local` namespace.
fn ehr_status_with_subject(index: u32) -> openehr_rm::prelude::EhrStatus {
    openehr_its::json::from_canonical_value(&json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": "local",
                "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": subject_uid(index) }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    }))
    .expect("the EHR_STATUS fixture decodes")
}

/// The subject uid of the `index`th seeded EHR.
fn subject_uid(index: u32) -> String {
    format!("00000000-0000-4000-8000-{index:012}")
}

/// A lookup of an EHR by the subject of its `EHR_STATUS` enters through
/// `uq_ehr_subject` and reads no `EHR_STATUS` version (#4442).
#[tokio::test]
async fn an_ehr_lookup_by_subject_enters_through_the_subject_index() {
    const EHRS: u32 = 200;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    for index in 0..EHRS {
        svc.create_ehr(Some(ehr_status_with_subject(index)))
            .await
            .expect("create an EHR with a subject");
    }
    pool.execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded EHRs");

    let params = Params::new()
        .with("u", ParamValue::Str(subject_uid(EHRS.div_euclid(2))))
        .with("n", ParamValue::Str("local".to_owned()));
    let prepared = aql_sql_with(
        "SELECT e/ehr_id/value FROM EHR e \
         WHERE e/ehr_status/subject/external_ref/id/value = $u \
         AND e/ehr_status/subject/external_ref/namespace = $n",
        &params,
        Vec::new(),
        None,
    );
    let plan = explain_aql(&pool, prepared).await;
    let root = plan
        .get(0)
        .expect("EXPLAIN FORMAT JSON returns a one-element array");
    let returned = root
        .pointer("/Plan/Actual Rows")
        .and_then(Value::as_f64)
        .expect("the root plan node reports its rows");
    assert!(
        (returned - 1.0).abs() < f64::EPSILON,
        "the lookup finds the one EHR of the subject: {returned} rows"
    );
    let scans = scans(&plan);
    assert!(
        scans
            .iter()
            .any(|scan| scan.index.as_deref() == Some("uq_ehr_subject")),
        "#4442: the lookup enters through the subject index:\n{}",
        render(&scans)
    );
    assert!(
        scans
            .iter()
            .all(|scan| !scan.relation.starts_with("version") && !scan.relation.starts_with("node")),
        "#4442: the lookup reads no EHR_STATUS version or node:\n{}",
        render(&scans)
    );
}

// ── the partitioned parents' statistics ─────────────────────────────────────

/// Analyze every partition and plain table of both stores and no partitioned
/// parent: the statistics autovacuum leaves, since "partitioned tables are not
/// processed by autovacuum" (`PostgreSQL` 18 docs § 24.1.3).
async fn analyze_as_autovacuum(pool: &PgPool) {
    let leaves: Vec<String> = sqlx::query_scalar(
        "SELECT format('%I.%I', n.nspname, c.relname) \
         FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname IN ('clinical', 'party') AND c.relkind = 'r' ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("list the stores' tables");
    for leaf in leaves {
        pool.execute(sqlx::AssertSqlSafe(format!("ANALYZE {leaf}")))
            .await
            .expect("analyze one table as autovacuum would");
    }
}

/// The statements that read one object's versions by `vo_id` and join each
/// version row to its commit audit, under the names the failure reports.
const READS_BY_VO_ID: [(&str, &str); 2] = [
    ("the revision history", meta::ALL_VERSION_META_SQL),
    ("the current version read", read::READ_CURRENT_SQL),
];

/// A store whose partitioned `version` table still holds the statistics of
/// its seed, beside one four-version object committed since: the state in
/// which the storage bench's reads walked `pk_commit_audit` (#4438). Returns
/// that object.
///
/// The partitions carry the statistics autovacuum takes of them; `version`
/// itself keeps the seed's highest commit audit id, which the planner reads as
/// the highest there is, so a merge join over the audit index looks as if it
/// stops after the seed's rows.
async fn seed_stale_parent_statistics(svc: &FerroEhrService, pool: &PgPool) -> VoId {
    // The storage bench's seed, analyzed whole once seeded.
    let body = composition("parent statistics");
    let mut ehrs = Vec::new();
    for _ in 0..20 {
        ehrs.push(svc.create_ehr(None).await.expect("create an EHR"));
    }
    for index in 0..300 {
        let ehr = *ehrs.get(index % ehrs.len()).expect("the seed holds EHRs");
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("commit a single-version composition");
    }
    pool.execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seed");

    // The commits that follow, with the statistics autovacuum takes of them.
    let ehr = *ehrs.first().expect("the seed holds EHRs");
    for _ in 0..1000 {
        svc.create_composition(ehr, uv(&body, "249", None))
            .await
            .expect("commit a first version");
    }
    let mut head = svc
        .create_composition(ehr, uv(&body, "249", None))
        .await
        .expect("commit the read object's first version");
    for _ in 0..3 {
        head = svc
            .update_composition(ehr, head.vo_id, uv(&body, "251", None))
            .await
            .expect("supersede the read object");
    }
    analyze_as_autovacuum(pool).await;
    head.vo_id
}

/// The reads of [`READS_BY_VO_ID`] whose custom or generic plan for `vo`
/// reaches a commit audit row other than by one `pk_commit_audit` probe, each
/// with its plan.
async fn reads_walking_the_commit_audit(pool: &PgPool, vo: VoId) -> Vec<String> {
    let mut walked = Vec::new();
    for (read, sql) in READS_BY_VO_ID {
        for mode in ["force_custom_plan", "force_generic_plan"] {
            let mut tx = pool.begin().await.expect("begin the explain transaction");
            tx.execute(sqlx::AssertSqlSafe(format!(
                "SET LOCAL plan_cache_mode = {mode}"
            )))
            .await
            .expect("set the plan cache mode");
            tx.execute(sqlx::AssertSqlSafe(format!(
                "PREPARE by_vo (uuid) AS {sql}"
            )))
            .await
            .expect("prepare the read");
            let plan: Value = sqlx::raw_sql(sqlx::AssertSqlSafe(format!(
                "{EXPLAIN}EXECUTE by_vo ('{}')",
                vo.0
            )))
            .fetch_one(&mut *tx)
            .await
            .expect("explain the read")
            .try_get(0)
            .expect("EXPLAIN FORMAT JSON returns one json column");
            // A prepared statement outlives the transaction it was made in.
            tx.execute("DEALLOCATE by_vo")
                .await
                .expect("drop the prepared read");
            tx.rollback().await.expect("roll the explain back");

            let scans = scans(&plan);
            let audits = on(&scans, "commit_audit");
            let probed = !audits.is_empty()
                && audits.iter().all(|scan| {
                    scan.index.as_deref() == Some("pk_commit_audit") && scan.rows <= 1.0
                });
            if !probed {
                walked.push(format!("{read}, {mode}:\n{}", render(&scans)));
            }
        }
    }
    walked
}

/// A read by `vo_id` probes each version's commit audit by key once the
/// maintenance task has refreshed the parents' statistics (#4438, #4453).
#[tokio::test]
async fn a_read_by_vo_id_probes_each_commit_audit_by_key() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let vo = seed_stale_parent_statistics(&svc, &pool).await;
    ferroehr::db::parent_statistics::refresh_pool(&pool)
        .await
        .expect("refresh the partitioned parents' statistics");

    let walked = reads_walking_the_commit_audit(&pool, vo).await;
    assert!(
        walked.is_empty(),
        "a read by vo_id reaches each version's commit audit by one primary-key \
         probe, never by a walk over the commit audit index:\n{}",
        walked.join("\n")
    );
}

/// A read by `vo_id` probes each version's commit audit by key while the
/// parents' statistics are stale, as they are between two refreshes (#4438).
///
/// With a plain join this state walked the whole audit index in a merge join:
/// 1 318 buffers for the history of a four-version object beside 1 300
/// others, against 21. The lateral audit probe leaves the planner no join to
/// choose.
#[tokio::test]
async fn a_read_by_vo_id_probes_each_commit_audit_by_key_over_stale_parent_statistics() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let vo = seed_stale_parent_statistics(&svc, &pool).await;

    let walked = reads_walking_the_commit_audit(&pool, vo).await;
    assert!(
        walked.is_empty(),
        "over stale parent statistics, a read by vo_id still reaches each version's \
         commit audit by one primary-key probe:\n{}",
        walked.join("\n")
    );
}

/// Upload one vendored CKM template and generate its committable example, as
/// the storage bench seeds it.
async fn ckm_example(svc: &FerroEhrService, template: &str) -> Value {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../corpus/templates/ckm/{template}.opt"));
    let xml = std::fs::read_to_string(path).expect("the vendored OPT is readable");
    let template_id = openehr_its::opt14::from_xml(&xml)
        .expect("the vendored OPT parses")
        .template_id
        .value;
    svc.upload_opt(xml).await.expect("upload the template");
    svc.template_adl14_example(template_id, Some("medium".to_owned()), None)
        .await
        .expect("the template generates an example composition")
}

/// The population `CONTAINS` of the storage bench's `contains_exists` enters
/// through the archetype index once the parents carry statistics (#4453).
///
/// With the partitions analyzed and the parents not, the planner estimated
/// the head join from no statistics at all and drove from every COMPOSITION
/// root, reading all of its nodes (3 703 buffers on this seed, against 1 745).
#[tokio::test]
async fn a_population_contains_enters_through_the_archetype_index() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));

    // The storage bench's seed: its two templates alternating over 20 EHRs.
    let vital_signs = ckm_example(&svc, "vital-signs").await;
    let gp_data_set = ckm_example(&svc, "gp-data-set").await;
    let mut ehrs = Vec::new();
    for _ in 0..20 {
        ehrs.push(svc.create_ehr(None).await.expect("create an EHR"));
    }
    for index in 0..300_usize {
        let body = if index.div_euclid(ehrs.len()).is_multiple_of(2) {
            &vital_signs
        } else {
            &gp_data_set
        };
        svc.create_composition(ehrs[index % ehrs.len()], uv(body, "249", None))
            .await
            .expect("seed a composition");
    }
    analyze_as_autovacuum(&pool).await;
    ferroehr::db::parent_statistics::refresh_pool(&pool)
        .await
        .expect("refresh the partitioned parents' statistics");

    let prepared = aql_sql(
        "SELECT count(c) FROM EHR e CONTAINS COMPOSITION c \
         CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.blood_pressure.v2]",
        Vec::new(),
    );
    let mut tx = pool.begin().await.expect("begin the explain transaction");
    let plan: Value = sqlx::query_scalar_with(
        sqlx::AssertSqlSafe(format!("{EXPLAIN}{}", prepared.sql)),
        prepared.values,
    )
    .fetch_one(&mut *tx)
    .await
    .expect("explain the population CONTAINS");
    tx.rollback().await.expect("roll the explain back");

    let scans = scans(&plan);
    assert!(
        on(&scans, "node_hot")
            .iter()
            .any(|scan| scan.index.as_deref() == Some("idx_node_hot_arch_subsume")),
        "the population CONTAINS finds the OBSERVATIONs through \
         idx_node_hot_arch_subsume rather than every COMPOSITION's nodes:\n{}",
        render(&scans)
    );
}

/// The maintenance task analyzes the partitioned parents of both stores, which
/// autovacuum never does, from its first tick (#4453).
#[tokio::test]
async fn the_maintenance_task_analyzes_the_partitioned_parents() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let pools = ferroehr::db::domain::DomainPools::from_shared(&pool);
    let svc = FerroEhrService::new(&pools);
    let ehr = svc.create_ehr(None).await.expect("create an EHR");
    for _ in 0..5 {
        svc.create_composition(ehr, uv(&composition("maintenance"), "249", None))
            .await
            .expect("commit a composition");
    }
    // `inherited` marks the statistics of a parent over all its partitions
    // (`PostgreSQL` 18 docs, `pg_stats`).
    let parent_stats = || async {
        sqlx::query_scalar::<_, i64>(
            "SELECT count(*) FROM pg_stats \
             WHERE schemaname = 'clinical' AND tablename IN ('node', 'version') AND inherited",
        )
        .fetch_one(&pool)
        .await
        .expect("count the parents' statistics")
    };
    assert_eq!(
        parent_stats().await,
        0,
        "no ANALYZE has reached the parents"
    );

    let task = tokio::spawn(ferroehr::db::parent_statistics::scheduled(pools.clone()));
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(60);
    while parent_stats().await == 0 && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    task.abort();
    assert!(
        parent_stats().await > 0,
        "the maintenance task's first tick analyzed clinical.node and clinical.version"
    );

    let refreshed = ferroehr::db::parent_statistics::refresh_pools(&pools)
        .await
        .expect("refresh both stores");
    for parent in [
        "clinical.node",
        "clinical.version",
        "clinical.vo_attestation",
        "party.node",
        "party.version",
        "party.vo_attestation",
    ] {
        assert!(
            refreshed.analyzed.iter().any(|table| table == parent),
            "{parent} is analyzed: {refreshed:?}"
        );
    }
    assert!(refreshed.refused.is_empty(), "{refreshed:?}");
}

/// Each serving role may analyze its store's partitioned parents and `ehr`,
/// so the maintenance task works under the role split (#4453).
#[tokio::test]
async fn the_serving_roles_may_analyze_the_partitioned_parents() {
    let db = testkit::db().await.expect("testkit database");
    for (store, role, tables) in [
        (
            ferroehr::db::domain::Domain::Clinical,
            "ferroehr_clinical",
            &[
                "clinical.ehr",
                "clinical.node",
                "clinical.version",
                "clinical.vo_attestation",
            ][..],
        ),
        (
            ferroehr::db::domain::Domain::Party,
            "ferroehr_party",
            &["party.node", "party.version", "party.vo_attestation"][..],
        ),
    ] {
        let dsn = crate::fixtures::dsn_as(&db, &format!("{role}_stats"), role).await;
        let pool = PgPool::connect(&dsn)
            .await
            .expect("connect as the serving role");
        for table in tables {
            let allowed: bool = sqlx::query_scalar("SELECT has_table_privilege($1, 'MAINTAIN')")
                .bind(*table)
                .fetch_one(&pool)
                .await
                .expect("read the role's privilege");
            assert!(allowed, "{role} holds MAINTAIN on {table}");
            pool.execute(sqlx::AssertSqlSafe(format!("ANALYZE ONLY {table}")))
                .await
                .expect("the serving role analyzes the table");
        }
        let refreshed = ferroehr::db::parent_statistics::refresh_store(&pool, store)
            .await
            .expect("refresh the store as its serving role");
        assert!(
            refreshed.refused.is_empty() && refreshed.analyzed.len() >= 3,
            "{role}: {refreshed:?}"
        );
        pool.close().await;
    }
}
