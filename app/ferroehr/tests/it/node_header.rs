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

use ferroehr::service::FerroEhrService;
use ferroehr::storage::integrity::StorageParityScope;

use crate::node_readers::served;
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
    assert!(
        at_root > 0,
        "a root row keeps its header in its columns too"
    );

    let now = served(&pool, &svc, &QUERIES).await;
    for (query, rows) in QUERIES.iter().zip(&now.aql) {
        assert!(
            !rows.is_empty(),
            "{query} answers nothing, so it proves nothing"
        );
    }

    // The shape the writer stored before the header columns: the keys in
    // `data`, the columns empty; a root's header names the version's template.
    let rewritten = sqlx::query(
        "UPDATE node n SET data = COALESCE(n.data, '{}'::jsonb) \
             || CASE WHEN n.rm_version IS NULL THEN '{}'::jsonb ELSE jsonb_build_object( \
                  'archetype_details', ext.node_archetype_details(n.archetype, n.rm_version, \
                      CASE WHEN n.num = 0 THEN v.template_id END), \
                  'archetype_node_id', ext.node_archetype_node_id(n.archetype, n.rm_version)) END \
             || CASE WHEN n.language IS NULL THEN '{}'::jsonb ELSE jsonb_build_object( \
                  'language', ext.node_code_phrase('ISO_639-1', n.language)) END \
             || CASE WHEN n.encoding IS NULL THEN '{}'::jsonb ELSE jsonb_build_object( \
                  'encoding', ext.node_code_phrase('IANA_character-sets', n.encoding)) END, \
             rm_version = NULL, language = NULL, encoding = NULL \
         FROM version v \
         WHERE v.vo_id = n.vo_id AND v.sys_version = n.sys_version \
           AND (n.rm_version IS NOT NULL OR n.language IS NOT NULL OR n.encoding IS NOT NULL)",
    )
    .execute(&pool)
    .await
    .expect("rewrite the rows into the earlier shape")
    .rows_affected();
    assert!(rewritten > 0, "the rewrite must match the header rows");

    let before = served(&pool, &svc, &QUERIES).await;
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
