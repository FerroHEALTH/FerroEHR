// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The node header columns (#4525) against a real `PostgreSQL` 18 (shared
//! testkit harness): a row below the root keeps its archetype header,
//! language and encoding in `rm_version`, `language` and `encoding`, and every
//! reader answers byte for byte what it answered while `data` held them.
//!
//! The proof stores the vendored CKM examples, reads them through every
//! reader (the Rust rebuild, `ext.node_fragment`, AQL), rewrites each row
//! into the shape the writer stored before the header columns (the keys back
//! in `data`, the columns `NULL`), and reads again: the two answers must be
//! the same bytes. No openEHR spec governs storage: our own design.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              fixture helpers; a failing fixture must panic at the fixture (the \
              Rust Book ch11)"
)]

use serde_json::Value;
use sqlx::PgPool;
use uuid::Uuid;

use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::integrity::StorageParityScope;
use ferroehr::service::query::request::AqlQueryRequest;

use crate::storage_parity::seed_typed_value_compositions;

/// The AQL statements whose answers must not move: every header path the
/// emitter reads from a column, the paths it rebuilds a member for, a whole
/// entry (a subtree reassembled in Rust) and the filters over them.
const QUERIES: [&str; 6] = [
    "SELECT o/archetype_node_id, o/archetype_details, o/archetype_details/rm_version, \
     o/archetype_details/archetype_id/value, o/language, o/language/code_string, \
     o/language/terminology_id/value, o/encoding, o/encoding/code_string \
     FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o",
    "SELECT o FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o",
    "SELECT cl/archetype_node_id, cl/archetype_details, cl \
     FROM EHR e CONTAINS COMPOSITION c CONTAINS CLUSTER cl",
    "SELECT c/archetype_node_id, c/archetype_details, c/language \
     FROM EHR e CONTAINS COMPOSITION c",
    "SELECT o/archetype_node_id FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
     WHERE o/language/code_string = 'en' AND o/encoding/code_string = 'UTF-8'",
    "SELECT o/archetype_node_id FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
     WHERE o/archetype_details/rm_version = '1.2.0' \
     AND o/archetype_node_id = 'openEHR-EHR-OBSERVATION.blood_pressure.v2'",
];

/// Everything a reader serves of the stored content, serialized.
#[derive(Debug, PartialEq, Eq)]
struct Served {
    /// Each row's fragment as the Rust rebuild serves it, in node order.
    rust: Vec<String>,
    /// Each row's `ext.node_fragment`, as jsonb text, in node order.
    sql: Vec<String>,
    /// Each query's rows, sorted, so a plan that returns them in another
    /// order is no difference.
    aql: Vec<Vec<String>>,
}

async fn served(pool: &PgPool, svc: &FerroEhrService) -> Served {
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
        "SELECT ext.node_fragment(data, rm_type, archetype, name, name_code, name_terminology, \
                value_type, value_num, value_text, value_code, value_terminology, rm_version, \
                language, encoding)::text \
           FROM node ORDER BY vo_id, sys_version, num",
    )
    .fetch_all(pool)
    .await
    .expect("read the rebuilt fragments");
    let mut aql = Vec::new();
    for query in QUERIES {
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
    Served { rust, sql, aql }
}

#[tokio::test]
async fn header_columns_serve_the_bytes_the_whole_fragment_served() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    seed_typed_value_compositions(&svc).await;

    let (archetyped, languages, encodings, at_root): (i64, i64, i64, i64) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE rm_version IS NOT NULL), \
                count(*) FILTER (WHERE language IS NOT NULL), \
                count(*) FILTER (WHERE encoding IS NOT NULL), \
                count(*) FILTER (WHERE num = 0 AND (rm_version IS NOT NULL \
                    OR language IS NOT NULL OR encoding IS NOT NULL)) \
           FROM node",
    )
    .fetch_one(&pool)
    .await
    .expect("count the header columns");
    assert!(
        archetyped >= 10 && languages >= 5 && encodings >= 5,
        "the seed must exercise the header columns: {archetyped} archetype headers, \
         {languages} languages, {encodings} encodings"
    );
    assert_eq!(at_root, 0, "a root row keeps its header in its fragment");

    let now = served(&pool, &svc).await;
    for (query, rows) in QUERIES.iter().zip(&now.aql) {
        assert!(
            !rows.is_empty(),
            "{query} answers nothing, so it proves nothing"
        );
    }

    // The shape the writer stored before the header columns: the keys in
    // `data`, the columns empty.
    let rewritten = sqlx::query(
        "UPDATE node SET data = COALESCE(data, '{}'::jsonb) \
             || CASE WHEN rm_version IS NULL THEN '{}'::jsonb ELSE jsonb_build_object( \
                  'archetype_details', ext.node_archetype_details(archetype, rm_version), \
                  'archetype_node_id', ext.node_archetype_node_id(archetype, rm_version)) END \
             || CASE WHEN language IS NULL THEN '{}'::jsonb ELSE jsonb_build_object( \
                  'language', ext.node_code_phrase('ISO_639-1', language)) END \
             || CASE WHEN encoding IS NULL THEN '{}'::jsonb ELSE jsonb_build_object( \
                  'encoding', ext.node_code_phrase('IANA_character-sets', encoding)) END, \
             rm_version = NULL, language = NULL, encoding = NULL \
         WHERE rm_version IS NOT NULL OR language IS NOT NULL OR encoding IS NOT NULL",
    )
    .execute(&pool)
    .await
    .expect("rewrite the rows into the earlier shape")
    .rows_affected();
    assert!(rewritten > 0, "the rewrite must match the header rows");

    let before = served(&pool, &svc).await;
    assert_eq!(now.rust.len(), before.rust.len());
    for (i, (now, before)) in now.rust.iter().zip(&before.rust).enumerate() {
        assert_eq!(now, before, "the Rust rebuild of row {i}");
    }
    assert_eq!(now.sql, before.sql, "ext.node_fragment");
    for ((query, now), before) in QUERIES.iter().zip(&now.aql).zip(&before.aql) {
        assert_eq!(now, before, "{query}");
    }

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(report.is_clean(), "{report:?}");
}
