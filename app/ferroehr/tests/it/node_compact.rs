// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The node compact markers (#4904) against a real `PostgreSQL` 18 (shared
//! testkit harness): a row stores a plain `DV_DATE_TIME` origin or time as
//! its value string, leaves out a bare `PARTY_SELF` subject, and the root
//! leaves out its uid and keeps its header in the columns, and every reader
//! answers byte for byte what it answered while `data` held them.
//!
//! The proof stores the vendored CKM examples, reads them through every
//! reader (the Rust rebuild, `ext.node_fragment`, AQL), rewrites each row
//! into the shape the writer stored at generation 4 (the keys back in
//! `data`, the markers and the root's header columns `NULL`), and reads
//! again: the two answers must be the same bytes. No openEHR spec governs
//! storage: our own design.

use ferroehr::service::FerroEhrService;
use ferroehr::storage::integrity::StorageParityScope;

use crate::node_readers::served;
use crate::storage_parity::seed_typed_value_compositions;

/// The AQL statements whose answers must not move: every path the markers
/// answer, as a projection, a filter and an order, the root's uid and header
/// through the fragment, the `EHR_STATUS` root an EHR source opens, and the
/// whole entries a subtree reassembles.
const QUERIES: [&str; 9] = [
    "SELECT o/data/origin, o/data/origin/value, o/subject, o/subject/external_ref \
     FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o",
    "SELECT ev/time, ev/time/value, ev/name/value \
     FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o CONTAINS POINT_EVENT ev",
    "SELECT o/data/origin/value \
     FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o \
     WHERE o/data/origin/value > '1000-01-01T00:00:00' \
     ORDER BY o/data/origin/value DESC",
    "SELECT ev/time/value \
     FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o CONTAINS POINT_EVENT ev \
     WHERE ev/time/value > '1000-01-01T00:00:00' \
     ORDER BY ev/time/value",
    "SELECT c/uid/value, c/uid, c/archetype_details, c/archetype_details/template_id/value, \
     c/archetype_details/rm_version, c/archetype_node_id, c/language, \
     c/name/value, c/territory \
     FROM EHR e CONTAINS COMPOSITION c",
    "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c \
     WHERE c/archetype_details/template_id/value = 'Vital signs'",
    "SELECT e/ehr_status/uid/value, e/ehr_status/uid, e/ehr_status/subject, \
     e/ehr_status/archetype_details, e/ehr_status/archetype_node_id FROM EHR e",
    "SELECT o FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o",
    "SELECT a/time/value, a/subject FROM EHR e CONTAINS COMPOSITION c CONTAINS ACTION a",
];

#[tokio::test]
async fn compact_markers_serve_the_bytes_the_whole_fragment_served() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    seed_typed_value_compositions(&svc).await;

    let (origins, times, subjects, uids, root_headers, root_language, roots, compositions): (
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
        i64,
    ) = sqlx::query_as(
        "SELECT count(*) FILTER (WHERE origin_compact), \
                count(*) FILTER (WHERE time_compact), \
                count(*) FILTER (WHERE party_self), \
                count(*) FILTER (WHERE uid_derived), \
                count(*) FILTER (WHERE num = 0 AND rm_version IS NOT NULL), \
                count(*) FILTER (WHERE num = 0 AND language IS NOT NULL), \
                count(*) FILTER (WHERE num = 0), \
                count(*) FILTER (WHERE num = 0 AND rm_type = 'COMPOSITION') \
           FROM node",
    )
    .fetch_one(&pool)
    .await
    .expect("count the markers");
    assert!(
        origins >= 4 && times >= 4 && subjects >= 4 && compositions >= 4,
        "the seed must exercise the markers: {origins} origins, {times} times, \
         {subjects} subjects, {compositions} compositions"
    );
    // Every root (the EHR_STATUS roots too) leaves its uid and header with
    // the version; a COMPOSITION root its language as well.
    assert_eq!(
        (uids, root_headers, root_language),
        (roots, roots, compositions),
        "{roots} roots, {compositions} compositions"
    );

    let now = served(&pool, &svc, &QUERIES).await;
    for (query, rows) in QUERIES.iter().zip(&now.aql) {
        assert!(
            !rows.is_empty(),
            "{query} answers nothing, so it proves nothing"
        );
    }
    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("sweep");
    assert!(report.is_clean(), "{report:?}");

    // The shape the writer stored at generation 4: the keys in `data`, the
    // markers empty, and the root's header in its fragment.
    let rewritten = sqlx::query(
        "UPDATE node n SET data = COALESCE(n.data, '{}'::jsonb) \
             || CASE WHEN n.origin_compact THEN jsonb_build_object( \
                  'origin', ext.node_date_time(n.data -> 'origin')) ELSE '{}'::jsonb END \
             || CASE WHEN n.time_compact THEN jsonb_build_object( \
                  'time', ext.node_date_time(n.data -> 'time')) ELSE '{}'::jsonb END \
             || CASE WHEN n.party_self THEN jsonb_build_object( \
                  'subject', '{\"_type\": \"PARTY_SELF\"}'::jsonb) ELSE '{}'::jsonb END \
             || CASE WHEN n.uid_derived THEN jsonb_build_object('uid', jsonb_build_object( \
                  '_type', 'OBJECT_VERSION_ID', 'value', ext.version_uid(v.vo_id, \
                  v.creating_system_id, v.trunk_version, v.branch_number, v.branch_version))) \
                ELSE '{}'::jsonb END \
             || CASE WHEN n.num = 0 AND n.rm_version IS NOT NULL THEN jsonb_build_object( \
                  'archetype_details', ext.node_archetype_details(n.archetype, n.rm_version, \
                      v.template_id), \
                  'archetype_node_id', ext.node_archetype_node_id(n.archetype, n.rm_version)) \
                ELSE '{}'::jsonb END \
             || CASE WHEN n.num = 0 AND n.language IS NOT NULL THEN jsonb_build_object( \
                  'language', ext.node_code_phrase('ISO_639-1', n.language)) \
                ELSE '{}'::jsonb END \
             || CASE WHEN n.num = 0 AND n.encoding IS NOT NULL THEN jsonb_build_object( \
                  'encoding', ext.node_code_phrase('IANA_character-sets', n.encoding)) \
                ELSE '{}'::jsonb END, \
             origin_compact = NULL, time_compact = NULL, party_self = NULL, \
             uid_derived = NULL, \
             rm_version = CASE WHEN n.num = 0 THEN NULL ELSE n.rm_version END, \
             language = CASE WHEN n.num = 0 THEN NULL ELSE n.language END, \
             encoding = CASE WHEN n.num = 0 THEN NULL ELSE n.encoding END \
         FROM version v \
         WHERE v.vo_id = n.vo_id AND v.sys_version = n.sys_version \
           AND (n.origin_compact OR n.time_compact OR n.party_self OR n.uid_derived \
                OR (n.num = 0 AND (n.rm_version IS NOT NULL OR n.language IS NOT NULL \
                    OR n.encoding IS NOT NULL)))",
    )
    .execute(&pool)
    .await
    .expect("rewrite the rows into the earlier shape")
    .rows_affected();
    assert!(rewritten > 0, "the rewrite must match the marked rows");

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
