// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Persistence integration tests: the greenfield schema applies
//! cleanly on a real `PostgreSQL` 18, the `ext` magnitude functions follow
//! the spec formulas, the temporal versioning model behaves, and the node
//! codec round-trips through the database.
//!
//! Each test takes a fresh, fully-migrated database from the shared `testkit`
//! harness (`tools/testkit`); the returned guard releases the clone on drop.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::path::Path;

use ferroehr::db;
use ferroehr::storage::codec::{decompose, reassemble};
use ferroehr::storage::row::NodeRow;
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

/// Every domain that carries change control holds its archival tier as a
/// PARTITION of the relation it archives, not as a mirror in another schema:
/// three partitioned relations, two partitions each. No openEHR spec governs
/// storage tiering — our own design/extension.
async fn assert_tier_partitions(pool: &PgPool) {
    for schema in ["clinical", "party"] {
        let partitions: Vec<(String, String)> = sqlx::query_as(
            "SELECT p.relname, c.relname FROM pg_inherits i \
             JOIN pg_class p ON p.oid = i.inhparent \
             JOIN pg_class c ON c.oid = i.inhrelid \
             JOIN pg_namespace n ON n.oid = p.relnamespace \
             WHERE n.nspname = $1 AND p.relkind = 'p' AND c.relkind = 'r' \
             ORDER BY 1, 2",
        )
        .bind(schema)
        .fetch_all(pool)
        .await
        .expect("partitions");
        assert_eq!(
            partitions,
            [
                ("node".to_owned(), "node_cold".to_owned()),
                ("node".to_owned(), "node_hot".to_owned()),
                ("version".to_owned(), "version_cold".to_owned()),
                ("version".to_owned(), "version_hot".to_owned()),
                (
                    "vo_attestation".to_owned(),
                    "vo_attestation_cold".to_owned()
                ),
                ("vo_attestation".to_owned(), "vo_attestation_hot".to_owned()),
            ],
            "{schema}"
        );
    }
}

#[tokio::test]
async fn migrations_apply_cleanly_and_idempotently() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    // running again must be a no-op, not an error
    db::run_migrations(&pool)
        .await
        .expect("migrations idempotent");

    let applied = |schema: &'static str| {
        let pool = pool.clone();
        async move {
            // The schema name is one of five literals below, never input.
            sqlx::query_scalar::<_, i64>(sqlx::AssertSqlSafe(format!(
                "SELECT count(*) FROM {schema}._sqlx_migrations"
            )))
            .fetch_one(&pool)
            .await
            .unwrap_or_else(|e| panic!("{schema} bookkeeping: {e}"))
        }
    };
    // One file per concern, numbered per domain with no gaps, so
    // `_sqlx_migrations` reads as the set's table of contents.
    assert_eq!(applied("ext").await, 4);
    assert_eq!(applied("clinical").await, 12);
    assert_eq!(applied("party").await, 8);
    assert_eq!(applied("linkage").await, 4);
    assert_eq!(applied("audit").await, 10);

    let tables = |schema: &'static str| {
        let pool = pool.clone();
        async move {
            sqlx::query_scalar::<_, String>(
                "SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
                 WHERE n.nspname = $1 AND c.relkind IN ('r', 'p') \
                   AND c.relname <> '_sqlx_migrations' \
                   AND NOT EXISTS (SELECT 1 FROM pg_inherits i WHERE i.inhrelid = c.oid) \
                 ORDER BY 1",
            )
            .bind(schema)
            .fetch_all(&pool)
            .await
            .unwrap_or_else(|e| panic!("{schema} tables: {e}"))
        }
    };
    assert_eq!(
        tables("clinical").await,
        [
            "adl2_artefact",
            "archetype_store",
            "blob_ref",
            "category_map",
            "commit_audit",
            "contribution",
            "ehr",
            "ehr_folder",
            "event_outbox",
            "event_outbox_reader",
            "event_subscription",
            "fhir_mapping",
            "item_tag",
            "node",
            "restriction",
            "retention_anchor",
            "retention_policy",
            "stored_query",
            "template_ref",
            "template_store",
            "usage_report_instance",
            "usage_report_window",
            "version",
            "vo_attestation",
            "vo_head",
        ]
    );
    assert_eq!(
        tables("party").await,
        [
            "blob_ref",
            "commit_audit",
            "contribution",
            "event_outbox",
            "event_outbox_reader",
            "identifier_scheme",
            "item_tag",
            "national_identifier",
            "node",
            "party_relationship_target",
            "version",
            "vo_attestation",
            "vo_head",
        ]
    );

    assert_tier_partitions(&pool).await;

    // The mirror schemas and the union views they needed are gone with them.
    let leftovers: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM information_schema.schemata \
         WHERE schema_name IN ('ehr', 'demographic', 'cold', 'cold_demographic')",
    )
    .fetch_one(&pool)
    .await
    .expect("count the first-generation schemas");
    assert_eq!(leftovers, 0, "no first-generation schema may be created");
}

/// None of the four columns a commit updates on `vo_head` is indexed, in either
/// domain.
///
/// That is the STRUCTURAL half of the heap-only-update property: PostgreSQL 18
/// §"Heap-Only Tuples (HOT)" makes an update heap-only when no indexed column
/// changes, so the property is a fact about the index set rather than about a
/// counter that happened to move during one run. Adding an index on any of
/// these four columns would cost every commit an index insert and a new index
/// entry per version, which is what this test exists to notice. No openEHR spec
/// governs storage layout — our own design/extension.
#[tokio::test]
async fn no_commit_updated_column_of_vo_head_is_indexed() {
    // The columns `HeadUpsert`'s ON CONFLICT branch writes.
    const UPDATED: [&str; 4] = [
        "head_sys_version",
        "trunk_head_sys_version",
        "lifecycle_state",
        "committed_at",
    ];
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    for schema in ["clinical", "party"] {
        let indexed: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT a.attname FROM pg_index i \
             JOIN pg_class c ON c.oid = i.indrelid \
             JOIN pg_namespace n ON n.oid = c.relnamespace \
             JOIN pg_attribute a ON a.attrelid = c.oid AND a.attnum = ANY(i.indkey) \
             WHERE n.nspname = $1 AND c.relname = 'vo_head' ORDER BY 1",
        )
        .bind(schema)
        .fetch_all(&pool)
        .await
        .expect("indexed columns of vo_head");
        for column in UPDATED {
            assert!(
                !indexed.iter().any(|c| c == column),
                "{schema}.vo_head.{column} is indexed, so a commit's update of it is no \
                 longer heap-only: {indexed:?}"
            );
        }
    }
}

/// The instance is single-tenant: no relation in any domain carries a tenant
/// column, and no row policy exists anywhere.
///
/// openEHR places multi-tenancy at the layer that HOSTS several logical EHR
/// systems rather than inside one — BASE
/// `architecture_overview/master06-design_of_the_ehr.adoc` §The EHR System: a
/// system is "a distinct logical repository corresponding to an organisational
/// entity that is legally responsible" for the data, and is "distinct from any
/// underlying virtualisation infrastructure or cloud computing facility, which
/// may house multiple logical EHR systems in a multi-tenant fashion". Isolation
/// between organisations is therefore a deployment property here.
#[tokio::test]
async fn no_relation_carries_a_tenant_column_and_no_row_policy_exists() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let tenant_columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_schema, table_name FROM information_schema.columns \
         WHERE table_schema IN ('clinical', 'party', 'linkage', 'audit', 'ext') \
           AND column_name = 'tenant_id' ORDER BY 1, 2",
    )
    .fetch_all(&pool)
    .await
    .expect("scan for tenant columns");
    assert!(
        tenant_columns.is_empty(),
        "no relation may carry a tenant column: {tenant_columns:?}"
    );

    let policies: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT schemaname, tablename, policyname FROM pg_policies \
         WHERE schemaname IN ('clinical', 'party', 'linkage', 'audit', 'ext') ORDER BY 1, 2, 3",
    )
    .fetch_all(&pool)
    .await
    .expect("scan for row policies");
    assert!(policies.is_empty(), "no row policy may exist: {policies:?}");
}

/// The clinical domain holds exactly ONE subject identifier: the promoted
/// `ehr.subject_id`/`subject_namespace` pair, and only under the pseudonym
/// guard.
///
/// The pair has to stay there, because the wire binds it to `EHR_STATUS` content:
/// `ehr_get_by_subject` matches `EHR_STATUS.subject.external_ref.id.value` and
/// `.namespace` (ITS-REST `ehr_get_by_subject.yaml`) and a second EHR for the
/// same subject is a `409` (`409_EHR.yaml`). Everything else that names a
/// subject — the EHR Index associations — belongs to the cross-reference
/// domain, because a clinical column no trigger guards is free to hold a
/// national identifier, and the separation GDPR Art. 4(5) asks for
/// (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>) is only as good as its
/// narrowest hole.
#[tokio::test]
async fn only_the_guarded_ehr_columns_name_a_subject_in_the_clinical_domain() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let columns: Vec<(String, String)> = sqlx::query_as(
        "SELECT table_name, column_name FROM information_schema.columns \
         WHERE table_schema = 'clinical' AND column_name LIKE 'subject%' \
         ORDER BY 1, 2",
    )
    .fetch_all(&pool)
    .await
    .expect("scan the clinical domain for subject columns");
    assert_eq!(
        columns,
        vec![
            ("ehr".to_owned(), "subject_id".to_owned()),
            ("ehr".to_owned(), "subject_namespace".to_owned()),
        ],
        "only the guarded ehr pair may name a subject in the clinical domain"
    );

    // And the pair the wire needs is guarded: the trigger is what refuses a
    // national identifier under `privacy.subject_namespaces`.
    let guarded: Vec<String> = sqlx::query_scalar(
        "SELECT t.tgname FROM pg_trigger t \
         JOIN pg_class c ON c.oid = t.tgrelid \
         JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'clinical' AND c.relname = 'ehr' AND NOT t.tgisinternal \
         ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .expect("read the ehr triggers");
    assert!(
        guarded
            .iter()
            .any(|name| name == "ehr_subject_pseudonym_guard"),
        "the promoted subject pair must stay under the pseudonym guard: {guarded:?}"
    );
}

/// The cross-reference lives in the linkage domain, and the erasure function is
/// the only way its rows are ever removed.
///
/// The linkage role holds `SELECT`, `INSERT` and `UPDATE` and no `DELETE`, so a
/// merge corrects forward and nothing quietly drops history; erasure still has
/// to reach the map, because a row naming an erased EHR is the additional
/// information of GDPR Art. 4(5) outliving the data it was additional to
/// (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>). A `SECURITY DEFINER`
/// function is what squares the two.
#[tokio::test]
async fn the_linkage_role_erases_only_through_the_definer_function() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let relations: Vec<String> = sqlx::query_scalar(
        "SELECT c.relname FROM pg_class c JOIN pg_namespace n ON n.oid = c.relnamespace \
         WHERE n.nspname = 'linkage' AND c.relkind IN ('r', 'p') \
           AND c.relname <> '_sqlx_migrations' ORDER BY 1",
    )
    .fetch_all(&pool)
    .await
    .expect("linkage relations");
    assert_eq!(relations, ["subject_ehr"], "linkage holds one relation");

    let definer: (bool, String) = sqlx::query_as(
        "SELECT p.prosecdef, pg_get_function_identity_arguments(p.oid) \
         FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = 'linkage' AND p.proname = 'erase_ehr'",
    )
    .fetch_one(&pool)
    .await
    .expect("the erase function exists");
    assert_eq!(
        definer,
        (true, "an_ehr_id uuid".to_owned()),
        "erase_ehr is SECURITY DEFINER over one EHR id"
    );

    // PUBLIC must not hold EXECUTE on a definer function: that would hand every
    // role the owner's reach, including the DELETE this schema grants nobody.
    let public_execute: bool = sqlx::query_scalar(
        "SELECT has_function_privilege('public', p.oid, 'EXECUTE') \
         FROM pg_proc p JOIN pg_namespace n ON n.oid = p.pronamespace \
         WHERE n.nspname = 'linkage' AND p.proname = 'erase_ehr'",
    )
    .fetch_one(&pool)
    .await
    .expect("read PUBLIC's privilege on the erase function");
    assert!(!public_execute, "PUBLIC may not execute linkage.erase_ehr");
}

/// A database created by a release older than the storage rewrite is refused at
/// boot, with the remedy, rather than served as an empty repository beside the
/// operator's unreachable content.
///
/// The rewrite is greenfield: the migration sets replace the first
/// generation's outright, so there is nothing to upgrade in place. The
/// signature is a first-generation schema that carries its own migration
/// bookkeeping — a bare schema of that name is not evidence of anything.
#[tokio::test]
async fn a_database_from_before_the_storage_rewrite_is_refused_with_the_remedy() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    // The shape a pre-rewrite database has: the old schema, carrying the
    // bookkeeping table only a migrator that ran there would have written.
    sqlx::query("CREATE SCHEMA ehr")
        .execute(&pool)
        .await
        .expect("create the first-generation schema");
    sqlx::query("CREATE TABLE ehr._sqlx_migrations (version bigint PRIMARY KEY)")
        .execute(&pool)
        .await
        .expect("create the first-generation bookkeeping");

    let error = db::run_migrations(&pool)
        .await
        .expect_err("a pre-rewrite database must be refused");
    assert!(
        matches!(
            error,
            db::DbError::FirstGenerationDatabase { schema: "ehr" }
        ),
        "the refusal must be the typed one, not a bare relation-exists error: {error}"
    );
    let message = error.to_string();
    assert!(
        message.contains("recreate the database"),
        "the refusal must name the remedy: {message}"
    );

    // A bare schema of the same name is NOT the signature: someone may have
    // made one, and refusing on it would strand a healthy deployment.
    sqlx::query("DROP TABLE ehr._sqlx_migrations")
        .execute(&pool)
        .await
        .expect("drop the bookkeeping");
    db::run_migrations(&pool)
        .await
        .expect("an empty schema of that name is not a pre-rewrite database");
}

/// A first-generation `ext`, `linkage` or `audit` set is refused by the SAME
/// typed error, on the description of its version 1.
///
/// Those three schema NAMES survive the rewrite, so their bookkeeping exists in
/// both generations and its mere presence proves nothing; which migration ran
/// first does. Without this the old set would reach its own migrator and fail
/// on a checksum mismatch — an error about a hash where the operator needs the
/// remedy.
#[tokio::test]
async fn a_first_generation_set_in_a_surviving_schema_is_refused_by_its_first_migration() {
    // (schema, the first generation's version-1 description, this build's).
    const SIGNATURES: [(&str, &str, &str); 3] = [
        ("ext", "openehr functions", "schema and roles"),
        ("linkage", "baseline", "schema and role"),
        ("audit", "baseline", "schema and roles"),
    ];
    for (schema, first_generation, second_generation) in SIGNATURES {
        let db = testkit::db().await.expect("testkit database");
        let pool = db.pool();
        set_first_description(&pool, schema, first_generation).await;
        let error = db::run_migrations(&pool)
            .await
            .expect_err("a first-generation set must be refused");
        assert!(
            matches!(&error, db::DbError::FirstGenerationDatabase { schema: s } if *s == schema),
            "{schema}: the refusal must be the typed one, not a checksum error: {error}"
        );
        assert!(
            error.to_string().contains("recreate the database"),
            "{schema}: the refusal must name the remedy: {error}"
        );

        // The discrimination is real: the same bookkeeping carrying THIS
        // build's version-1 description is a generation-2 set, and migrating it
        // again is the no-op it should be.
        let db = testkit::db().await.expect("testkit database");
        let pool = db.pool();
        set_first_description(&pool, schema, second_generation).await;
        db::run_migrations(&pool)
            .await
            .unwrap_or_else(|e| panic!("{schema}: a generation-2 set must pass the guard: {e}"));
    }
}

/// Rewrite the version-1 description of `schema`'s existing bookkeeping.
///
/// The harness hands out a MIGRATED database, so the three surviving schemas
/// already carry this build's own bookkeeping: re-describing its first row is
/// exactly the state a first-generation database is in, and nothing else about
/// the database is disturbed.
async fn set_first_description(pool: &PgPool, schema: &'static str, description: &str) {
    // `schema` is one of the literals at the call sites, never input.
    let updated = sqlx::query(sqlx::AssertSqlSafe(format!(
        "UPDATE {schema}._sqlx_migrations SET description = $1 WHERE version = 1"
    )))
    .bind(description)
    .execute(pool)
    .await
    .expect("re-describe version 1")
    .rows_affected();
    assert_eq!(updated, 1, "{schema} must carry exactly one version-1 row");
}

#[tokio::test]
async fn ext_magnitude_function_follows_the_spec_formulas() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let cases: &[(&str, f64)] = &[
        (
            r#"{"_type":"DV_QUANTITY","magnitude":117.0,"units":"mm[Hg]"}"#,
            117.0,
        ),
        (r#"{"_type":"DV_COUNT","magnitude":3}"#, 3.0),
        (r#"{"_type":"DV_ORDINAL","value":2}"#, 2.0),
        (
            r#"{"_type":"DV_PROPORTION","numerator":60.0,"denominator":100.0,"type":2}"#,
            0.6,
        ),
        // days since 0001-01-01: 1970-01-01 => 719162
        (r#"{"_type":"DV_DATE","value":"1970-01-01"}"#, 719_162.0),
        (r#"{"_type":"DV_DATE","value":"1970"}"#, 719_162.0),
        // seconds since 0001-01-01T00:00Z
        (
            r#"{"_type":"DV_DATE_TIME","value":"1970-01-01T00:00:00Z"}"#,
            62_135_596_800.0,
        ),
        (
            r#"{"_type":"DV_DATE_TIME","value":"1970-01-01T01:00:00+01:00"}"#,
            62_135_596_800.0,
        ),
        (r#"{"_type":"DV_TIME","value":"10:55:41"}"#, 39_341.0),
        (r#"{"_type":"DV_DURATION","value":"PT42M"}"#, 2_520.0),
        (
            r#"{"_type":"DV_DURATION","value":"P1Y"}"#,
            365.24 * 86_400.0,
        ),
        (r#"{"_type":"DV_DURATION","value":"-PT30S"}"#, -30.0),
    ];
    for (dv, expected) in cases {
        let got: Option<f64> = sqlx::query_scalar("SELECT openehr_magnitude($1::jsonb)::float8")
            .bind(dv)
            .fetch_one(&pool)
            .await
            .expect("magnitude call");
        let got = got.unwrap_or_else(|| panic!("NULL magnitude for {dv}"));
        assert!(
            (got - expected).abs() < 1e-6,
            "magnitude({dv}) = {got}, expected {expected}"
        );
    }
    // unknown/unparseable values yield NULL, never an error
    let none: Option<f64> =
        sqlx::query_scalar("SELECT openehr_magnitude('{\"_type\":\"DV_TEXT\"}'::jsonb)::float8")
            .fetch_one(&pool)
            .await
            .expect("null magnitude");
    assert!(none.is_none());
}

/// The store is append-only: a supersession is an insert, the trunk position is
/// what a second version at the same position collides on, and the head row is
/// what says which version is current (RM common master06 §The 'Virtual Version
/// Tree').
#[tokio::test]
async fn the_append_only_versioning_model_behaves() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (vo, ehr_id) = seed_version(&pool).await;

    // A second version at an occupied TRUNK POSITION is impossible at the
    // database: the trunk line is one global sequence per container.
    let duplicate = sqlx::query(
        "INSERT INTO version (vo_id, kind, ehr_id, sys_version, trunk_version, committed_at, contribution_id, commit_audit_id, creating_system_id)
         SELECT $1, 'COMPOSITION', $2, 2, 1, now(), contribution_id, commit_audit_id, 'other.system'
         FROM version WHERE vo_id = $1 AND sys_version = 1",
    )
    .bind(vo)
    .bind(ehr_id)
    .execute(&pool)
    .await;
    assert!(
        duplicate.is_err(),
        "a second version at trunk position 1 must be refused"
    );

    // The supersession is ONE insert: nothing is updated, and the previous
    // version keeps every column it was committed with.
    sqlx::query(
        "INSERT INTO version (vo_id, kind, ehr_id, sys_version, trunk_version, committed_at, contribution_id, commit_audit_id, creating_system_id)
         SELECT $1, 'COMPOSITION', $2, 2, 2, now(), contribution_id, commit_audit_id, creating_system_id
         FROM version WHERE vo_id = $1 AND sys_version = 1",
    )
    .bind(vo)
    .bind(ehr_id)
    .execute(&pool)
    .await
    .expect("commit v2");
    sqlx::query(
        "UPDATE vo_head SET head_sys_version = 2, trunk_head_sys_version = 2 WHERE vo_id = $1",
    )
    .bind(vo)
    .execute(&pool)
    .await
    .expect("advance the head");

    // LATEST_VERSION = the head row's answer; ALL_VERSIONS = unfiltered.
    let current: i32 = sqlx::query_scalar(
        "SELECT v.sys_version FROM version v JOIN vo_head h ON h.vo_id = v.vo_id \
         AND h.trunk_head_sys_version = v.sys_version WHERE v.vo_id = $1",
    )
    .bind(vo)
    .fetch_one(&pool)
    .await
    .expect("current");
    assert_eq!(current, 2);
    let all: i64 = sqlx::query_scalar("SELECT count(*) FROM version WHERE vo_id = $1")
        .bind(vo)
        .fetch_one(&pool)
        .await
        .expect("all versions");
    assert_eq!(all, 2);
}

/// A TRUNK position is unique across creating systems; a BRANCH id is not.
///
/// RM common `master06-change_control_package.adoc` §Version Identification
/// §Distributed Versioning identifies a version globally by the tuple
/// `{object_id, creating_system_id, version_tree_id}` — but that tuple alone
/// would admit two versions of ONE container both claiming trunk position 2,
/// one per system. The model forbids it: §Copying §Subsequent Local
/// Modifications makes a second system BRANCH rather than extend the trunk
/// ("the local system id is recorded in the `uid.creating_system_id()`
/// attribute, while branching numbering is used in the
/// `uid.version_tree_id()`"), and §Moving Version Containers has the trunk
/// CONTINUE its increment under the new system's id. Either way the trunk is
/// one global sequence.
///
/// BRANCH ids, by contrast, legitimately collide across systems — each system
/// allocates its branch numbers locally, which is precisely what the 3-part
/// identifier exists to disambiguate ("Two places are indicated on the diagram
/// where identification clashes could have occurred, but are prevented due to
/// the use of the 3-part unique Version identifier scheme"). So the tuple
/// constraint keeps `creating_system_id` and the trunk-position index does not.
///
/// The two live write paths derive the tree position from the container's own
/// tip and cannot produce a duplicate; the archive load replays arbitrary file
/// input, so it is guarded — and this pins both the typed refusal and the
/// database backstop behind it.
#[tokio::test]
async fn a_trunk_position_is_unique_across_creating_systems_but_a_branch_id_is_not() {
    use ferroehr::ids::{EhrId, VoId};
    use ferroehr::storage::error::StorageError;
    use ferroehr::storage::version_repo::import::{VerbatimVersionRow, insert_version_verbatim};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (vo, ehr_id) = seed_version(&pool).await;
    // The seeded row is trunk 1 created by `ferroehr.test`.
    let (contribution_id, commit_audit_id): (Uuid, Uuid) =
        sqlx::query_as("SELECT contribution_id, commit_audit_id FROM version WHERE vo_id = $1")
            .bind(vo)
            .fetch_one(&pool)
            .await
            .expect("seeded provenance");

    let row = |trunk: i32, branch: i32, branch_version: i32, system: &'static str, ord: i32| {
        VerbatimVersionRow {
            vo_id: VoId(vo),
            kind: "COMPOSITION",
            ehr_id: Some(EhrId(ehr_id)),
            sys_version: ord,
            trunk_version: trunk,
            branch_number: branch,
            branch_version,
            preceding_version_uid: None,
            other_input_version_uids: None,
            committed_at: Some("2026-01-01T00:00:00Z"),
            lifecycle_state: "532",
            contribution_id,
            commit_audit_id,
            template_id: None,
            signature: None,
            signature_client_supplied: false,
            creating_system_id: system,
            wrapped_original: None,
            body: None,
        }
    };

    let mut conn = pool.acquire().await.expect("connection");

    // A second creating system claiming the SAME trunk position is refused,
    // and the refusal names the container, the position and the holder.
    let clash = insert_version_verbatim(&mut conn, &row(1, 0, 0, "sysB.example.org", 2)).await;
    match clash {
        Err(StorageError::TrunkPositionInUse {
            vo_id,
            trunk_version,
            held_by,
        }) => {
            assert_eq!(vo_id, vo);
            assert_eq!(trunk_version, 1);
            assert_eq!(held_by, "ferroehr.test");
        }
        other => panic!("expected a typed trunk-position conflict, got {other:?}"),
    }

    // The database is the backstop behind the guard: the same row written past
    // the repository layer still cannot land.
    let raw = sqlx::query(
        "INSERT INTO version (vo_id, kind, ehr_id, sys_version, trunk_version, committed_at, \
         contribution_id, commit_audit_id, creating_system_id) \
         VALUES ($1, 'COMPOSITION', $2, 2, 1, '2026-01-01T00:00:00Z'::timestamptz, \
                 $3, $4, 'sysB.example.org')",
    )
    .bind(vo)
    .bind(ehr_id)
    .bind(contribution_id)
    .bind(commit_audit_id)
    .execute(&pool)
    .await;
    assert!(
        raw.is_err(),
        "the trunk-position unique index must reject a second trunk row at one position"
    );

    // A BRANCH id, however, may repeat across creating systems: `1.1.1` minted
    // by two different systems off trunk node 1 are two distinct versions.
    insert_version_verbatim(&mut conn, &row(1, 1, 1, "sysB.example.org", 3))
        .await
        .expect("a foreign branch off trunk 1");
    insert_version_verbatim(&mut conn, &row(1, 1, 1, "sysC.example.org", 4))
        .await
        .expect("another system's branch with the SAME branch id is a distinct version");
    let branches: i64 =
        sqlx::query_scalar("SELECT count(*) FROM version WHERE vo_id = $1 AND branch_number = 1")
            .bind(vo)
            .fetch_one(&pool)
            .await
            .expect("branch rows");
    assert_eq!(
        branches, 2,
        "cross-system branch-id collisions stay admitted (the 3-part identifier disambiguates)"
    );
}

/// An as-of read resolves along the TRUNK.
///
/// `VERSIONED_OBJECT.version_at_time (a_time): VERSION[1]`
/// (`UML/classes/org.openehr.rm.common.versioned_object.adoc` §Functions)
/// returns exactly one version, yet a container may have several valid tips at
/// one instant — the trunk tip plus one per open branch — so only the trunk
/// makes the answer unique. The class draws the same line elsewhere:
/// `latest_version` is "the most recently added version (i.e. on trunk or any
/// branch)" while `latest_trunk_version` and `trunk_lifecycle_state` read the
/// trunk alone, the latter being how the spec says to decide "if the version
/// container is logically deleted".
///
/// The corollary this pins on the other side: a container holding branches but
/// no trunk version has no as-of answer — and that container is a state RM
/// common `master06-change_control_package.adoc` §Copying §Subsequent Local
/// Modifications rules out, since "branch versions … cannot be copied without
/// their corresponding preceding versions on the same branch (if any) and trunk
/// versions also being copied".
#[tokio::test]
async fn an_as_of_read_resolves_along_the_trunk() {
    use ferroehr::ids::{EhrId, VoId};
    use ferroehr::storage::version_repo::import::{VerbatimVersionRow, insert_version_verbatim};
    use ferroehr::storage::version_repo::read::version_at;

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (vo, ehr_id) = seed_version(&pool).await;
    let (contribution_id, commit_audit_id): (Uuid, Uuid) =
        sqlx::query_as("SELECT contribution_id, commit_audit_id FROM version WHERE vo_id = $1")
            .bind(vo)
            .fetch_one(&pool)
            .await
            .expect("seeded provenance");

    let branch_row = |target: VoId, ord: i32| VerbatimVersionRow {
        vo_id: target,
        kind: "COMPOSITION",
        ehr_id: Some(EhrId(ehr_id)),
        sys_version: ord,
        trunk_version: 1,
        branch_number: 1,
        branch_version: 1,
        preceding_version_uid: None,
        other_input_version_uids: None,
        committed_at: Some("2020-01-01T00:00:00Z"),
        lifecycle_state: "532",
        contribution_id,
        commit_audit_id,
        template_id: None,
        signature: None,
        signature_client_supplied: false,
        creating_system_id: "sysB.example.org",
        wrapped_original: None,
        body: None,
    };
    let mut conn = pool.acquire().await.expect("connection");

    // The seeded container's trunk version 1 was committed at `now()`; a branch
    // tip live at the same instant does not displace it.
    insert_version_verbatim(&mut conn, &branch_row(VoId(vo), 2))
        .await
        .expect("branch beside the trunk");
    // The server clock, not the test process's: `committed_at` is stamped by
    // the database, and a client/DB skew under parallel load races the at-time
    // read against the instants it is comparing. Same reasoning as
    // `service_demographic::db_now`.
    let at: jiff::Timestamp = sqlx::query_scalar::<_, jiff_sqlx::Timestamp>("SELECT now()")
        .fetch_one(&pool)
        .await
        .expect("db now()")
        .to_jiff();
    let read = version_at(&pool, VoId(vo), at)
        .await
        .expect("as-of read")
        .expect("the trunk version is current at this instant");
    assert_eq!(
        (read.trunk_version, read.branch_number, read.branch_version),
        (1, 0, 0),
        "an as-of read returns the TRUNK version, never a branch tip"
    );

    // A container with branches and no trunk version has no as-of answer — the
    // copy-closure rule says such a container should not exist.
    let branch_only = VoId(Uuid::now_v7());
    insert_version_verbatim(&mut conn, &branch_row(branch_only, 1))
        .await
        .expect("a branch-only container");
    assert!(
        version_at(&pool, branch_only, at)
            .await
            .expect("as-of read")
            .is_none(),
        "a container with no trunk version has no as-of answer to give"
    );
}

#[tokio::test]
async fn node_codec_round_trips_through_the_database() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    // m5: round-trip EVERY corpus COMPOSITION through the real jsonb `node`
    // store (decompose → INSERT → SELECT → reassemble), not just one sample.
    // One container / one DB for speed; each composition gets its own vo.
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json");
    let mut checked = 0usize;
    for entry in std::fs::read_dir(&dir).expect("corpus dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let text = std::fs::read_to_string(&path).expect("read corpus file");
        let Ok(composition) = serde_json::from_str::<Value>(&text) else {
            continue; // deliberately-invalid corpus files
        };
        if composition.get("_type").and_then(Value::as_str) != Some("COMPOSITION") {
            continue;
        }
        let (vo, ehr_id) = seed_version(&pool).await;
        let rows =
            decompose(composition.clone()).unwrap_or_else(|e| panic!("decompose {path:?}: {e}"));
        insert_nodes(&pool, vo, 1, ehr_id, &rows).await;

        let read: Vec<NodeRow> = sqlx::query(
            "SELECT num, num_cap, parent_num, rm_type, archetype, name, name_code,
                    name_terminology, path, data
             FROM node WHERE vo_id = $1 AND sys_version = 1 ORDER BY num",
        )
        .bind(vo)
        .fetch_all(&pool)
        .await
        .expect("read nodes")
        .into_iter()
        .map(|r| NodeRow {
            num: r.get("num"),
            num_cap: r.get("num_cap"),
            parent_num: r.get("parent_num"),
            rm_type: r.get("rm_type"),
            archetype: r.get("archetype"),
            // arch_* are query-only promoted columns, unused by `reassemble`.
            arch_entity: None,
            arch_concept: None,
            arch_major: None,
            name: r.get("name"),
            name_code: r.get("name_code"),
            name_terminology: r.get("name_terminology"),
            path: r.get("path"),
            data: r.get("data"),
            // Promoted-leaf columns are query-only and unused by `reassemble`.
            promoted: Vec::new(),
        })
        .collect();

        assert_eq!(read.len(), rows.len(), "node count for {path:?}");
        let reassembled = reassemble(&read).expect("reassemble");
        assert_eq!(
            reassembled, composition,
            "DB round-trip must be lossless for {path:?}"
        );
        checked += 1;
    }
    assert!(checked >= 50, "expected the full corpus, got {checked}");

    // The CONTAINS shape works against real rows (IPS sample).
    let (vo, ehr_id) = seed_version(&pool).await;
    let composition = corpus_sample();
    let rows = decompose(composition).expect("decompose");
    insert_nodes(&pool, vo, 1, ehr_id, &rows).await;
    let contains: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM node c
         JOIN node o ON o.vo_id = c.vo_id AND o.sys_version = c.sys_version
                    AND o.num BETWEEN c.num AND c.num_cap
         WHERE c.vo_id = $1 AND c.sys_version = 1 AND c.num = 0
           AND o.rm_type = 'OBSERVATION'",
    )
    .bind(vo)
    .fetch_one(&pool)
    .await
    .expect("contains query");
    let expected = i64::try_from(rows.iter().filter(|r| r.rm_type == "OBSERVATION").count())
        .expect("count fits");
    assert_eq!(contains, expected);
}

/// The stored `version.template_id` is read back through the version
/// read-back and surfaced by `FerroEhrService::template_of_version` (the ABAC
/// template attribute).
#[tokio::test]
async fn template_id_is_read_back_from_version() {
    use ferroehr::service::FerroEhrService;

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (vo, ehr_id) = seed_version(&pool).await;
    // Production sets this on commit (service/vobject.rs); set it directly here.
    // version.template_id has an FK into template_ref — seed the template.
    seed_template(&pool, "org.openehr::vital_signs.v1").await;
    sqlx::query("UPDATE version SET template_id = $2 WHERE vo_id = $1")
        .bind(vo)
        .bind("org.openehr::vital_signs.v1")
        .execute(&pool)
        .await
        .expect("set template_id");
    // Nodes so the read-back can reassemble the current version.
    let rows = decompose(corpus_sample()).expect("decompose");
    insert_nodes(&pool, vo, 1, ehr_id, &rows).await;

    let service = FerroEhrService::new(&db::domain::DomainPools::from_shared(&pool));
    // Current version.
    assert_eq!(
        service
            .template_of_version(ferroehr::ids::VoId(vo), None)
            .await
            .expect("read template")
            .as_deref(),
        Some("org.openehr::vital_signs.v1")
    );
    // Explicit version 1.
    assert_eq!(
        service
            .template_of_version(ferroehr::ids::VoId(vo), Some("1"))
            .await
            .expect("read template v1")
            .as_deref(),
        Some("org.openehr::vital_signs.v1")
    );
    // Unknown object → None (not an error).
    assert_eq!(
        service
            .template_of_version(ferroehr::ids::VoId(Uuid::now_v7()), None)
            .await
            .expect("unknown ok"),
        None
    );
}

/// Projection-independence regression (EHRbase v1 read these attributes off the
/// SELECT columns): the ABAC query subject-scope pre-filter restricts rows to
/// the caller's patient EHRs, and the executor collects the touched
/// EHR/template sets, **even when the query projects neither `ehr_id`/`value`
/// nor a template path**.
#[tokio::test]
async fn query_subject_scope_filters_and_collects_projection_independently() {
    use ferroehr::service::FerroEhrService;
    use ferroehr::service::query::request::AqlQueryRequest;

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    // Two EHRs with distinct subjects, each holding one composition (same corpus
    // body) under a distinct template id.
    let (vo_a, ehr_a) = seed_version(&pool).await;
    let (vo_b, ehr_b) = seed_version(&pool).await;
    for (ehr, vo, subject, template) in [
        (ehr_a, vo_a, "SUBJ-A", "org.openehr::t_a.v1"),
        (ehr_b, vo_b, "SUBJ-B", "org.openehr::t_b.v1"),
    ] {
        sqlx::query("UPDATE ehr SET subject_id = $2 WHERE id = $1")
            .bind(ehr)
            .bind(subject)
            .execute(&pool)
            .await
            .expect("set subject");
        // version.template_id has an FK into template_store — seed first.
        seed_template(&pool, template).await;
        sqlx::query("UPDATE version SET template_id = $2 WHERE vo_id = $1")
            .bind(vo)
            .bind(template)
            .execute(&pool)
            .await
            .expect("set template");
        let rows = decompose(corpus_sample()).expect("decompose");
        insert_nodes(&pool, vo, 1, ehr, &rows).await;
    }

    let service = FerroEhrService::new(&db::domain::DomainPools::from_shared(&pool));
    // The projection is `c/name/value` — neither ehr_id nor a template path.
    let aql = "SELECT c/name/value FROM COMPOSITION c";

    // Unscoped: both compositions are visible (control).
    let all = service
        .execute_ad_hoc_query(aql.to_owned(), AqlQueryRequest::default())
        .await
        .expect("unscoped query");
    assert_eq!(row_count(&all.result_set), 2, "both compositions visible");

    // Scoped to SUBJ-A + collection on: only A's row is fetched, and the touched
    // EHR/template sets are collected despite the projection.
    let scoped = service
        .execute_ad_hoc_query(
            aql.to_owned(),
            AqlQueryRequest {
                subject_scope: Some("SUBJ-A".to_owned()),
                collect_attributes: true,
                ..Default::default()
            },
        )
        .await
        .expect("scoped query");
    assert_eq!(row_count(&scoped.result_set), 1, "only SUBJ-A row fetched");
    assert_eq!(scoped.ehr_ids, vec![ehr_a.to_string()]);
    assert_eq!(scoped.template_ids, vec!["org.openehr::t_a.v1".to_owned()]);
}

/// The number of `rows` in an ITS-REST `RESULT_SET`.
fn row_count(result_set: &Value) -> usize {
    result_set
        .get("rows")
        .and_then(Value::as_array)
        .map_or(0, Vec::len)
}

// ─── helpers ────────────────────────────────────────────────────────────────

/// Creates ehr + audit + contribution + an open v1 `version`; returns
/// `(vo_id, ehr_id)`.
/// Seed a `template_store` row so `version.template_id` (FK) can reference
/// it — production ingests the OPT before any commit can cite it.
async fn seed_template(pool: &PgPool, template_id: &str) {
    sqlx::query(
        "INSERT INTO template_store (template_id, content) VALUES ($1, '<test/>')
         ON CONFLICT (template_id) DO NOTHING",
    )
    .bind(template_id)
    .execute(pool)
    .await
    .expect("seed template_store");
    // Register the wire address exactly as `store_template` does — the
    // version.template_id FK targets the template_ref registry.
    sqlx::query("INSERT INTO template_ref (template_id) VALUES ($1) ON CONFLICT DO NOTHING")
        .bind(template_id)
        .execute(pool)
        .await
        .expect("seed template_ref");
}

async fn seed_version(pool: &PgPool) -> (Uuid, Uuid) {
    let ehr_id = Uuid::now_v7();
    let vo = Uuid::now_v7();
    // ehr.system_id is NOT NULL.
    sqlx::query("INSERT INTO ehr (id, system_id) VALUES ($1, 'ferroehr.test')")
        .bind(ehr_id)
        .execute(pool)
        .await
        .expect("ehr row");
    // audit.change_type is a coded audit_change_type value ('249' creation),
    // enforced by ck_audit_change_type — not the rubric.
    let commit_audit_id: Uuid = sqlx::query_scalar(
        "INSERT INTO commit_audit (system_id, change_type, committer)
         VALUES ('test.system', '249', '{\"_type\":\"PARTY_SELF\"}'::jsonb)
         RETURNING id",
    )
    .fetch_one(pool)
    .await
    .expect("audit row");
    let contribution_id: Uuid = sqlx::query_scalar(
        "INSERT INTO contribution (ehr_id, commit_audit_id) VALUES ($1, $2) RETURNING id",
    )
    .bind(ehr_id)
    .bind(commit_audit_id)
    .fetch_one(pool)
    .await
    .expect("contribution row");
    // creating_system_id is NOT NULL.
    sqlx::query(
        "INSERT INTO version (vo_id, kind, ehr_id, sys_version, trunk_version, committed_at, contribution_id, commit_audit_id, creating_system_id)
         VALUES ($1, 'COMPOSITION', $2, 1, 1, now(), $3, $4, 'ferroehr.test')",
    )
    .bind(vo)
    .bind(ehr_id)
    .bind(contribution_id)
    .bind(commit_audit_id)
    .execute(pool)
    .await
    .expect("version row");
    seed_head(pool, vo).await;
    // Every EHR has an EHR_STATUS from creation (RM ehr §"EHR Creation");
    // the AQL population gate keys off its `is_queryable` flag
    // (`i_query_service.adoc`), so a spec-realistic fixture must seed one —
    // a bare `ehr` row without a status is not a state the service can
    // produce.
    let status_vo = Uuid::now_v7();
    sqlx::query(
        "INSERT INTO version (vo_id, kind, ehr_id, sys_version, trunk_version, committed_at, contribution_id, commit_audit_id, creating_system_id)
         VALUES ($1, 'EHR_STATUS', $2, 1, 1, now(), $3, $4, 'ferroehr.test')",
    )
    .bind(status_vo)
    .bind(ehr_id)
    .bind(contribution_id)
    .bind(commit_audit_id)
    .execute(pool)
    .await
    .expect("ehr_status version row");
    seed_head(pool, status_vo).await;
    sqlx::query(
        "INSERT INTO node (vo_id, sys_version, num, num_cap, parent_num, rm_type, ehr_id, path, data)
         VALUES ($1, 1, 0, 0, 0, 'EHR_STATUS', $2, '',
                 '{\"_type\":\"EHR_STATUS\",\"is_queryable\":true,\"is_modifiable\":true}'::jsonb)",
    )
    .bind(status_vo)
    .bind(ehr_id)
    .execute(pool)
    .await
    .expect("ehr_status root node");
    (vo, ehr_id)
}

/// Write the head row a seeded version needs, from the version rows themselves.
///
/// The commit path writes it in the same statement as the version; a fixture
/// that inserts version rows directly has to write it too, or every read that
/// asks "what is current" finds no answer.
async fn seed_head(pool: &PgPool, vo: Uuid) {
    sqlx::query(
        "INSERT INTO vo_head (vo_id, kind, ehr_id, head_sys_version, trunk_head_sys_version, \
             lifecycle_state, committed_at) \
         SELECT a.vo_id, t.kind, t.ehr_id, a.head, t.sys_version, t.lifecycle_state, \
                t.committed_at \
         FROM (SELECT vo_id, max(sys_version) AS head FROM version \
               WHERE vo_id = $1 GROUP BY vo_id) a \
         JOIN LATERAL (SELECT kind, ehr_id, sys_version, lifecycle_state, committed_at \
                       FROM version WHERE vo_id = a.vo_id AND branch_number = 0 \
                       ORDER BY sys_version DESC LIMIT 1) t ON true \
         ON CONFLICT (vo_id) DO UPDATE SET \
             head_sys_version = EXCLUDED.head_sys_version, \
             trunk_head_sys_version = EXCLUDED.trunk_head_sys_version, \
             committed_at = EXCLUDED.committed_at",
    )
    .bind(vo)
    .execute(pool)
    .await
    .expect("head row");
}

async fn insert_nodes(pool: &PgPool, vo: Uuid, sys_version: i32, ehr_id: Uuid, rows: &[NodeRow]) {
    for row in rows {
        sqlx::query(
            "INSERT INTO node (vo_id, sys_version, num, num_cap, parent_num,
                               ehr_id, rm_type, archetype, name, path, data)
             VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
        )
        .bind(vo)
        .bind(sys_version)
        .bind(row.num)
        .bind(row.num_cap)
        .bind(row.parent_num)
        .bind(ehr_id)
        .bind(&row.rm_type)
        .bind(&row.archetype)
        .bind(&row.name)
        .bind(&row.path)
        .bind(&row.data)
        .execute(pool)
        .await
        .expect("insert node");
    }
}

/// A real corpus composition (the IPS — the largest one).
fn corpus_sample() -> Value {
    let path = Path::new(env!("CARGO_MANIFEST_DIR")).join(
        "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
    );
    serde_json::from_str(&std::fs::read_to_string(path).expect("read ips_canonical.json"))
        .expect("parse composition")
}

/// The materialized `version.body` is byte-identical to the node-row
/// reassembly on a REAL service commit — the parity the body column's whole
/// design rests on (reads serve `body`; AQL reads the nodes; both must be the
/// same canonical value, RM common master06 §Copying: a stored version is
/// served verbatim).
#[tokio::test]
async fn materialized_body_matches_node_reassembly_on_a_real_commit() {
    use ferroehr::service::FerroEhrService;
    use ferroehr::storage::node_repo::read_version_canonical;

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let service = FerroEhrService::new(&db::domain::DomainPools::from_shared(&pool));
    let ehr_id = service.create_ehr(None).await.expect("ehr create");

    // The EHR create commits an EHR_STATUS through the full commit path.
    let (vo, body): (Uuid, Option<String>) =
        sqlx::query_as("SELECT vo_id, body FROM version WHERE ehr_id = $1 AND kind = 'EHR_STATUS'")
            .bind(ehr_id.0)
            .fetch_one(&pool)
            .await
            .expect("status version row");
    let body: Value =
        serde_json::from_str(&body.expect("a content-bearing version materializes its body"))
            .expect("the stored body text parses");
    let reassembled = read_version_canonical(&pool, ferroehr::ids::VoId(vo), 1)
        .await
        .expect("node reassembly");
    assert_eq!(
        body, reassembled,
        "version.body must equal the node-row reassembly"
    );
    assert_eq!(
        body.get("_type").and_then(Value::as_str),
        Some("EHR_STATUS")
    );
}

/// The fixed-text `unnest` node insert has no per-row parameter cost, so a
/// composition decomposing to more than 4,095 node rows — past the 65,535
/// extended-protocol parameter cap the old per-row shape hit — commits in one
/// statement (#2668).
#[tokio::test]
async fn write_nodes_survives_more_than_4095_rows() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (vo, ehr_id) = seed_version(&pool).await;

    let n = 5_000;
    let mut rows = Vec::with_capacity(n);
    for i in 0..n {
        let num = i32::try_from(i).expect("row ordinal fits i32");
        rows.push(NodeRow {
            num,
            num_cap: num,
            parent_num: 0,
            rm_type: "ELEMENT".to_owned(),
            archetype: None,
            arch_entity: None,
            arch_concept: None,
            arch_major: None,
            name: None,
            name_code: None,
            name_terminology: None,
            path: if i == 0 {
                String::new()
            } else {
                format!("items{i}.")
            },
            data: serde_json::json!({"_type": "ELEMENT", "archetype_node_id": "at0001"}),
            promoted: Vec::new(),
        });
    }
    let mut tx = pool.begin().await.expect("begin");
    ferroehr::storage::node_repo::write_nodes(
        &mut tx,
        ferroehr::ids::VoId(vo),
        1,
        Some(ferroehr::ids::EhrId(ehr_id)),
        &rows,
    )
    .await
    .expect("a 5,000-row version must write in one statement");
    tx.commit().await.expect("commit");

    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM node WHERE vo_id = $1")
        .bind(vo)
        .fetch_one(&pool)
        .await
        .expect("count");
    assert_eq!(count, 5_000);
}

/// Every pooled connection carries the configured `statement_timeout`
/// (#2669): the DB-side runaway-query guard is the one setting a pool variant
/// must never silently drop.
#[tokio::test]
async fn a_pooled_connection_applies_the_configured_statement_timeout() {
    let db = testkit::db().await.expect("testkit database");
    let settings = db::DbConfig {
        url: ferroehr::config::secret::SecretUrl::new(db.url()),
        statement_timeout_ms: 12_345,
        max_connections: 2,
        min_connections: 0,
        ..db::DbConfig::default()
    };

    let pool = db::connect(&settings).await.expect("clinical pool");
    let timeout: String = sqlx::query_scalar("SHOW statement_timeout")
        .fetch_one(&pool)
        .await
        .expect("read timeout");
    assert_eq!(
        timeout, "12345ms",
        "a pooled connection must carry the configured statement_timeout"
    );
}

/// A partially wiped database comes back whole (#3298): the sandbox reset drops
/// the clinical, audit and ext schemas and leaves the party and linkage sets
/// applied, so the clinical set has to rebuild everything the write path needs
/// or every update fails on the placement read with 42P01.
#[tokio::test]
async fn a_wiped_clinical_schema_is_rebuilt_and_serves_writes_again() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    for schema in ["clinical", "audit", "ext"] {
        sqlx::query(sqlx::AssertSqlSafe(format!(
            "DROP SCHEMA IF EXISTS {schema} CASCADE"
        )))
        .execute(&pool)
        .await
        .expect("drop the schema the sandbox wipe drops");
    }
    db::run_migrations(&pool)
        .await
        .expect("the clinical set rebuilds its schema on a database whose party set is complete");

    let svc = ferroehr::service::FerroEhrService::new(&db::domain::DomainPools::from_shared(&pool));
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr");
    let body = crate::fixtures::composition("after the wipe");
    let created = svc
        .create_composition(ehr_id, crate::fixtures::uv(&body, "249", None))
        .await
        .expect("a create works on the rebuilt schema");
    let first = created.version_uid();
    svc.update_composition(
        ehr_id,
        created.vo_id,
        crate::fixtures::uv(&body, "251", Some(&first)),
    )
    .await
    .expect("an update works too: the placement read reaches the cold alias view");
}
