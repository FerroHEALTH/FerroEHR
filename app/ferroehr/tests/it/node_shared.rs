// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A member repeated across a version's node rows is stored once and read
//! back whole by every reader (#6229, `ferroehr::storage::shared`).
//!
//! The IPS composition repeats one `other_participations` list on its 38
//! entries; it is stored on the first and referenced from the others. These
//! tests hold the rule at commit, the reads that put the member back (the
//! version read, a subtree whose interval leaves the holder out, an AQL
//! projection and predicate into the member), the sweep's verdict on a broken
//! reference, and every index on `node` against the members that are never
//! shared. No openEHR spec governs storage: our own design.

use serde_json::Value;
use sqlx::PgPool;

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::integrity::StorageParityDefect;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::storage::integrity::StorageParityScope;
use ferroehr::storage::node_repo::SubtreeAnchor;
use ferroehr::storage::shared::{UNSHARED, references_hold};

use crate::admin_fixture::repository;
use crate::fixtures::{composition, create_ehr, uv};

/// A file of the workspace, relative to this crate.
fn fixture(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

/// The vendored IPS composition, whose entries repeat one participation list.
fn ips() -> Value {
    serde_json::from_str(&fixture(
        "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
    ))
    .expect("the IPS composition parses")
}

/// The participation list the IPS entries repeat: the first
/// `other_participations` in the composition.
fn the_list() -> Value {
    fn find(value: &Value) -> Option<Value> {
        match value {
            Value::Object(map) => map
                .get("other_participations")
                .cloned()
                .or_else(|| map.values().find_map(find)),
            Value::Array(items) => items.iter().find_map(find),
            _ => None,
        }
    }
    find(&ips()).expect("the IPS entries carry the list")
}

/// An EHR with the IPS composition committed, and the composition's object id.
async fn ips_committed(svc: &FerroEhrService) -> (EhrId, VoId) {
    svc.template_adl14_upload(&fixture(
        "../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt",
    ))
    .await
    .expect("upload the IPS template");
    let ehr = create_ehr(svc).await;
    let created = svc
        .create_composition(ehr, uv(&ips(), "249", None))
        .await
        .expect("the IPS composition commits");
    (ehr, created.vo_id)
}

/// Each row of `vo` with its `shared` column and whether its references hold.
async fn shared_rows(pool: &PgPool, vo: VoId) -> Vec<(i32, Option<Value>, bool)> {
    sqlx::query_as(sqlx::AssertSqlSafe(format!(
        "SELECT n.num, n.shared, {} FROM node n WHERE n.vo_id = $1 ORDER BY n.num",
        references_hold("n")
    )))
    .bind(vo.0)
    .fetch_all(pool)
    .await
    .expect("read the node rows")
}

#[tokio::test]
async fn an_ips_commit_stores_the_repeated_list_once_and_reads_back_whole() {
    let (_db, pool, svc) = repository().await;
    let (_ehr, vo) = Box::pin(ips_committed(&svc)).await;
    let rows = shared_rows(&pool, vo).await;
    let sharing: Vec<&(i32, Option<Value>, bool)> = rows
        .iter()
        .filter(|(_, shared, _)| shared.is_some())
        .collect();
    let lists = sharing
        .iter()
        .filter(|(_, shared, _)| {
            shared
                .as_ref()
                .is_some_and(|s| s.get("other_participations").is_some())
        })
        .count();
    assert_eq!(lists, 37, "37 of the 38 entries reference the first's list");
    let mut holders: std::collections::BTreeMap<String, Vec<i64>> =
        std::collections::BTreeMap::new();
    for (num, shared, holds) in &sharing {
        assert!(holds, "row {num}'s references hold");
        for (key, holder) in shared
            .as_ref()
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            assert!(!UNSHARED.contains(&key.as_str()), "row {num} shares {key}");
            let holder = holder.as_i64().expect("a num");
            assert!(
                holder < i64::from(*num),
                "row {num} names an earlier holder"
            );
            holders.entry(key.clone()).or_default().push(holder);
        }
    }
    assert_eq!(
        holders.keys().collect::<Vec<_>>(),
        ["other_participations"],
        "only the list is long enough to share; an ism_transition is not"
    );
    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("the sweep runs");
    assert!(
        report.is_clean(),
        "the rows read back as the body: {report:?}"
    );
}

#[tokio::test]
async fn a_version_with_no_repeated_member_writes_no_shared_column() {
    let (_db, pool, svc) = repository().await;
    let ehr = create_ehr(&svc).await;
    let created = svc
        .create_composition(ehr, uv(&composition("plain"), "249", None))
        .await
        .expect("commit");
    let rows = shared_rows(&pool, created.vo_id).await;
    assert_ne!(rows.len(), 0, "the version has node rows");
    assert!(
        rows.iter().all(|(_, shared, _)| shared.is_none()),
        "{rows:?}"
    );
}

#[tokio::test]
async fn a_subtree_whose_interval_leaves_the_holder_out_reads_the_list_back() {
    let (_db, pool, svc) = repository().await;
    let (_ehr, vo) = Box::pin(ips_committed(&svc)).await;
    let anchors: Vec<SubtreeAnchor> = sqlx::query_as::<_, (i32, i32)>(
        "SELECT num, num_cap FROM node WHERE vo_id = $1 AND shared IS NOT NULL ORDER BY num",
    )
    .bind(vo.0)
    .fetch_all(&pool)
    .await
    .expect("read the referencing rows")
    .into_iter()
    .map(|(num, num_cap)| SubtreeAnchor {
        vo_id: vo,
        sys_version: 1,
        num,
        num_cap,
    })
    .collect();
    let list = the_list();
    let subtrees = ferroehr::storage::node_repo::read_subtrees_canonical(&pool, &anchors)
        .await
        .expect("the subtrees read");
    assert_eq!(subtrees.len(), anchors.len());
    for anchor in &anchors {
        assert_eq!(
            subtrees[anchor].get("other_participations"),
            Some(&list),
            "the subtree at {} carries the list its holder stores",
            anchor.num
        );
    }
}

#[tokio::test]
async fn aql_reads_and_filters_a_shared_member_as_written() {
    let (_db, _pool, svc) = repository().await;
    let (ehr, _vo) = Box::pin(ips_committed(&svc)).await;
    let rows = |aql: String| {
        let svc = &svc;
        async move {
            let outcome = Box::pin(svc.execute_aql(&aql, None, &AqlQueryRequest::default()))
                .await
                .expect("the query runs");
            outcome
                .result_set
                .to_value()
                .expect("the result set renders")["rows"]
                .as_array()
                .cloned()
                .unwrap_or_default()
        }
    };
    let pinned =
        format!("EHR e[ehr_id/value='{ehr}'] CONTAINS COMPOSITION c CONTAINS EVALUATION v");
    let all = rows(format!("SELECT v/other_participations FROM {pinned}")).await;
    let named = rows(format!(
        "SELECT v/other_participations FROM {pinned} \
         WHERE v/other_participations/performer/name = 'Lara Markham'"
    ))
    .await;
    let evaluations = rows(format!("SELECT COUNT(v/uid/value) FROM {pinned}")).await;
    assert_eq!(all.len(), 21, "every EVALUATION projects its list");
    let list = the_list();
    assert!(
        all.iter().all(|row| row[0] == list),
        "each projected list is the list as written: {all:?}"
    );
    assert_eq!(
        named, all,
        "the predicate keeps every EVALUATION: {evaluations:?}"
    );
}

#[tokio::test]
async fn the_sweep_reports_a_reference_to_a_row_without_the_member() {
    let (_db, pool, svc) = repository().await;
    let (_ehr, vo) = Box::pin(ips_committed(&svc)).await;
    sqlx::query(
        "UPDATE node SET shared = jsonb_build_object('other_participations', 0) \
         WHERE vo_id = $1 AND num = (SELECT max(num) FROM node WHERE vo_id = $1 \
         AND shared IS NOT NULL)",
    )
    .bind(vo.0)
    .execute(&pool)
    .await
    .expect("point a reference at the root");
    let holds = shared_rows(&pool, vo).await;
    assert!(
        holds.iter().any(|(_, _, holds)| !holds),
        "the reference breaks"
    );
    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("the sweep runs");
    assert!(
        report
            .mismatches
            .iter()
            .any(|m| m.vo_id == vo.0 && m.defect == StorageParityDefect::ContentDiffers),
        "{report:?}"
    );
}

/// The `data` members an SQL text reads: `data -> 'm'`, `data ->> 'm'` and
/// the first member of a `data #> '{m,…}'` or `data #>> '{m,…}'` path.
fn members_read(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(at) = rest.find("data") {
        rest = &rest[at + 4..];
        let after = rest.trim_start();
        let Some(path) = ["#>>", "#>", "->>", "->"]
            .iter()
            .find_map(|op| after.strip_prefix(op))
        else {
            continue;
        };
        let path = path.trim_start().trim_start_matches('(');
        let Some(quoted) = path.strip_prefix('\'') else {
            continue;
        };
        let member: String = quoted
            .trim_start_matches('{')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
            .collect();
        if !member.is_empty() {
            out.push(member);
        }
    }
    out
}

/// Every index on `node`, in both domains and both tiers, reads only members
/// that are never shared, in its expression, its predicate and every function
/// it calls; a future index that read another member would lose the rows that
/// share it.
#[tokio::test]
async fn every_node_index_reads_only_members_that_are_never_shared() {
    let (_db, pool, _svc) = repository().await;
    let indexes: Vec<(String, String)> = sqlx::query_as(
        "SELECT schemaname || '.' || indexname, indexdef FROM pg_indexes \
         WHERE schemaname IN ('clinical', 'party') AND tablename IN ('node_hot', 'node_cold')",
    )
    .fetch_all(&pool)
    .await
    .expect("read the index definitions");
    assert!(indexes.len() >= 20, "{indexes:?}");
    let functions: Vec<(String, String)> = sqlx::query_as(
        "SELECT p.proname, pg_get_functiondef(p.oid) FROM pg_proc p JOIN pg_namespace s ON s.oid = p.pronamespace \
         WHERE s.nspname = 'ext' AND p.prokind = 'f'",
    )
    .fetch_all(&pool)
    .await
    .expect("read the ext functions");
    let mut read: Vec<String> = Vec::new();
    for (name, definition) in &indexes {
        let mut texts = vec![definition.clone()];
        for (function, source) in &functions {
            // pg_indexes prints a function unqualified when the search path reaches it.
            if definition.contains(&format!("{function}(")) {
                texts.push(source.clone());
            }
        }
        for member in texts.iter().flat_map(|t| members_read(t)) {
            assert!(
                UNSHARED.contains(&member.as_str()),
                "{name} reads the member {member}, which may be shared: {definition}"
            );
            read.push(member);
        }
    }
    assert!(
        read.iter().any(|m| m == "value"),
        "the value indexes read `value` through their functions: {read:?}"
    );
}
