// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What every reader of the stored node rows serves (the Rust rebuild,
//! `ext.node_fragment` and AQL), read once for the storage-shape proofs that
//! compare two layouts of the same content byte for byte. No openEHR spec
//! governs storage: our own design.

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;

/// Everything a reader serves of the stored content, serialized.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Served {
    /// Each row's fragment as the Rust rebuild serves it, in node order.
    pub(crate) rust: Vec<String>,
    /// Each row's `ext.node_fragment`, as jsonb text, in node order.
    pub(crate) sql: Vec<String>,
    /// Each query's rows, sorted, so a plan that returns them in another
    /// order is no difference.
    pub(crate) aql: Vec<Vec<String>>,
}

/// Reads every stored version's node rows through the Rust rebuild and
/// `ext.node_fragment`, and answers each of `queries` through AQL.
pub(crate) async fn served(pool: &PgPool, svc: &FerroEhrService, queries: &[&str]) -> Served {
    let keys: Vec<(Uuid, i32)> = sqlx::query_as(
        "SELECT vo_id, sys_version FROM version WHERE body IS NOT NULL \
         ORDER BY vo_id, sys_version",
    )
    .fetch_all(pool)
    .await
    .expect("read the stored versions");
    let keys: Vec<(VoId, i32)> = keys.into_iter().map(|(vo, sys)| (VoId(vo), sys)).collect();
    let rows = ferroehr::storage::node_repo::read_version_rows_all(pool, &keys)
        .await
        .expect("read the node rows");
    let rust = rows
        .values()
        .flatten()
        .map(|row| serde_json::to_string(&row.data).expect("serializable"))
        .collect();
    let sql = sqlx::query_scalar(
        "SELECT ext.node_fragment(n.data, n.rm_type, n.archetype, n.name, n.name_code, \
                n.name_terminology, n.value_type, n.value_num, n.value_text, n.value_code, \
                n.value_terminology, n.rm_version, n.language, n.encoding, n.origin_compact, \
                n.time_compact, n.party_self, n.uid_derived, \
                ext.version_uid(v.vo_id, v.creating_system_id, v.trunk_version, \
                v.branch_number, v.branch_version), \
                CASE WHEN n.num = 0 THEN v.template_id END)::text \
           FROM node n JOIN version v ON v.vo_id = n.vo_id AND v.sys_version = n.sys_version \
           ORDER BY n.vo_id, n.sys_version, n.num",
    )
    .fetch_all(pool)
    .await
    .expect("read the rebuilt fragments");
    let aql = aql_answers(svc, queries).await;
    Served { rust, sql, aql }
}

/// Answers each of `queries` through AQL, each answer's rows serialized and
/// sorted, so a plan that returns them in another order is no difference.
pub(crate) async fn aql_answers(svc: &FerroEhrService, queries: &[&str]) -> Vec<Vec<String>> {
    let mut aql = Vec::new();
    for query in queries {
        let result = svc
            .execute_aql(query, None, &AqlQueryRequest::default())
            .await
            .map_err(|e| format!("{query}: {e:?}"))
            .expect("the AQL statement runs")
            .result_set;
        let mut rows: Vec<String> = result
            .get("rows")
            .and_then(Value::as_array)
            .expect("rows array")
            .iter()
            .map(|row| serde_json::to_string(row).expect("serializable"))
            .collect();
        rows.sort();
        aql.push(rows);
    }
    aql
}
