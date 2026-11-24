// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A judged AQL statement whose served versions are not read after it
//! carries the policy and definition revisions on the first row it serves
//! alone, under any offset, with one `WindowAgg` over the order the statement
//! already has and no other sort (#6320). An ordered statement with a
//! `LIMIT`, a `DISTINCT` and an aggregate carry them on every row. No openEHR
//! spec governs this: our own design.

use std::sync::Arc;

use ferroehr::access_eval::condition::care::Care;
use ferroehr::access_eval::condition::{AccessCondition, CallerKeys, ConditionGroups, VerdictPair};
use ferroehr::access_eval::procedure::Verdict;
use ferroehr::aql::ir::Params;
use ferroehr::aql::lineage::ArchetypeLineage;
use ferroehr::aql::sql::{EhrScopeKind, PreparedQuery, SqlCtx};
use ferroehr::config::profile::SpecProfile;
use serde_json::Value;
use sqlx::Row as _;

use crate::fixtures::{create_ehr, service};
use crate::storage_plans::explain_aql;

/// The EHRs the population holds.
const EHRS: usize = 7;

/// A condition with access groups that admits every row.
fn admitting() -> AccessCondition {
    let admit = || {
        Arc::new(VerdictPair {
            cared: Verdict::admit(),
            uncared: Verdict::admit(),
        })
    };
    AccessCondition::from(ConditionGroups {
        groups: Vec::new(),
        closed: admit(),
        default: Some(admit()),
        cared: Care::Census(Arc::from([])),
        subject_namespace: "local".into(),
        facts: CallerKeys {
            now: jiff::Timestamp::now(),
            parties: Arc::from([]),
            keeper: None,
        },
        confined: None,
        conflicts: Arc::from([]),
        episodic: false,
    })
}

/// `aql` lowered for a caller with access groups, paged by `limit` and
/// `offset`.
fn judged(aql: &str, limit: Option<i64>, offset: Option<i64>) -> PreparedQuery {
    let ast = openehr_query::parser::parse_str(aql).expect("the AQL parses");
    let params = Params::default();
    let ir = ferroehr::aql::plan(&ast, &params, SpecProfile::default()).expect("the AQL plans");
    let ctx = SqlCtx {
        system_id: "first.row.test".to_owned(),
        ehr_ids: Vec::new(),
        ehr_scope_kind: EhrScopeKind::Named,
        ehr_access: admitting(),
        limit,
        offset,
        archetype_lineage: Arc::new(ArchetypeLineage::default()),
        policy_parties: ferroehr::aql::sql::readers::PolicyParties::Served,
        access_settings: ferroehr::aql::sql::readers::EhrAccessReaders::Every,
        own_records: None,
        caseload: None,
    };
    let prepared = ferroehr::aql::sql::build(&ir, &params, &ctx).expect("the AQL lowers");
    assert!(prepared.layout.revisions, "{aql} selects the revisions");
    prepared
}

/// Each served row's first cell and whether it carries both revisions.
async fn served(pool: &sqlx::PgPool, prepared: PreparedQuery) -> Vec<(String, bool)> {
    let rows = sqlx::query_with(sqlx::AssertSqlSafe(prepared.sql), prepared.values)
        .fetch_all(pool)
        .await
        .expect("the statement runs");
    rows.iter()
        .map(|row| {
            let policy: Option<i64> = row.get("policy_revision");
            let definitions: Option<i64> = row.get("definition_revision");
            assert_eq!(policy.is_some(), definitions.is_some(), "both or neither");
            let cell = row
                .try_get::<String, _>(0)
                .or_else(|_| row.try_get::<i64, _>(0).map(|n| n.to_string()))
                .or_else(|_| row.try_get::<Value, _>(0).map(|v| v.to_string()))
                .expect("a text, count or jsonb cell");
            (cell, policy.is_some())
        })
        .collect()
}

/// The node types on the path from the plan's root down to its
/// `WindowAgg`, the `WindowAgg` included; empty when it has none.
fn above_window(plan: &Value) -> Vec<String> {
    fn walk(node: &Value, path: &mut Vec<String>) -> bool {
        let kind = node["Node Type"].as_str().unwrap_or_default().to_owned();
        path.push(kind.clone());
        if kind == "WindowAgg" {
            return true;
        }
        let found = node["Plans"]
            .as_array()
            .into_iter()
            .flatten()
            .any(|child| walk(child, path));
        if !found {
            path.pop();
        }
        found
    }
    let mut path = Vec::new();
    walk(&plan[0]["Plan"], &mut path);
    path
}

/// How many nodes of `kind` the plan holds.
fn count(plan: &Value, kind: &str) -> usize {
    fn walk(node: &Value, kind: &str) -> usize {
        usize::from(node["Node Type"] == kind)
            + node["Plans"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|child| walk(child, kind))
                .sum::<usize>()
    }
    walk(&plan[0]["Plan"], kind)
}

#[tokio::test]
async fn only_the_first_served_row_carries_the_revisions() {
    let (db, svc) = service().await;
    for _ in 0..EHRS {
        create_ehr(&svc).await;
    }
    let pool = db.pool();
    let all = "SELECT e/ehr_id/value FROM EHR e ORDER BY e/ehr_id/value";
    let ordered: Vec<String> = served(&pool, judged(all, None, None))
        .await
        .into_iter()
        .map(|(id, _)| id)
        .collect();
    assert_eq!(ordered.len(), EHRS);
    let unordered = "SELECT e/ehr_id/value FROM EHR e";
    for (aql, limit, offset) in [
        (unordered, None, None),
        (unordered, Some(3), None),
        (unordered, Some(3), Some(2)),
        (all, None, None),
        (all, None, Some(2)),
        (
            "SELECT e/ehr_id/value FROM EHR e ORDER BY e/time_created/value DESC, e/ehr_id/value",
            None,
            Some(1),
        ),
    ] {
        let rows = served(&pool, judged(aql, limit, offset)).await;
        assert!(!rows.is_empty(), "{aql} {limit:?} {offset:?} serves rows");
        let carried: Vec<bool> = rows.iter().map(|(_, carries)| *carries).collect();
        let mut first = vec![false; rows.len()];
        first[0] = true;
        assert_eq!(carried, first, "{aql} {limit:?} {offset:?}");
        if aql == all {
            let skip = usize::try_from(offset.unwrap_or(0)).expect("an offset");
            let take = usize::try_from(limit.unwrap_or(100)).expect("a limit");
            let ids: Vec<&String> = rows.iter().map(|(id, _)| id).collect();
            let expected: Vec<&String> = ordered.iter().skip(skip).take(take).collect();
            assert_eq!(ids, expected, "the page the order serves");
        }

        let plan = explain_aql(&pool, judged(aql, limit, offset)).await;
        assert_eq!(count(&plan, "WindowAgg"), 1, "{aql}: one window {plan}");
        let sorts = count(&plan, "Sort") + count(&plan, "Incremental Sort");
        let ordered = aql.contains("ORDER BY");
        assert!(
            sorts <= usize::from(ordered),
            "{aql}: the order's own sort alone {plan}"
        );
        let above = above_window(&plan);
        assert!(
            !above.iter().any(|kind| kind.contains("Sort")),
            "{aql}: no sort above the window {plan}"
        );
    }

    let past = served(
        &pool,
        judged(
            unordered,
            Some(5),
            Some(i64::try_from(EHRS).expect("a count")),
        ),
    )
    .await;
    assert!(
        past.is_empty(),
        "a page past the end serves no row, so carries none"
    );

    for (aql, limit, offset) in [
        (all, Some(3), Some(1)),
        ("SELECT DISTINCT e/ehr_id/value FROM EHR e", None, Some(1)),
        ("SELECT COUNT(e/ehr_id/value) FROM EHR e", None, None),
    ] {
        let rows = served(&pool, judged(aql, limit, offset)).await;
        assert!(!rows.is_empty(), "{aql} serves rows");
        assert!(rows.iter().all(|(_, carries)| *carries), "{aql}: every row");
        let plan = explain_aql(&pool, judged(aql, limit, offset)).await;
        assert_eq!(count(&plan, "WindowAgg"), 0, "{aql}: no window {plan}");
    }
}
