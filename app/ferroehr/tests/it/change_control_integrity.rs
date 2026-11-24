// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every version names a stored CONTRIBUTION and commit audit, and every
//! CONTRIBUTION a stored commit audit (#6394).
//!
//! The three references carry no foreign key: the commit statements insert
//! the rows they name in the same statement, a version written into an
//! existing CONTRIBUTION, an imported version and an archive load check that
//! the rows they name are stored, and the EHR erasure checks that nothing
//! still names an audit it deletes. These tests hold that across every path
//! that writes or deletes those rows, and show each check refusing a row that
//! would dangle. No openEHR spec governs storage mechanics: our own design.

use ferroehr::service::admin::types::ExportSpec;
use ferroehr::storage::error::StorageError;
use ferroehr::storage::version_repo::commit::{AuditRow, Folded, FoldedVersion, commit_folded};
use ferroehr::storage::version_repo::import::{
    ImportedVersionRow, VerbatimVersionRow, insert_imported_vo_version, insert_versions_verbatim,
};
use serde_json::json;
use sqlx::{AssertSqlSafe, Connection as _, PgPool};
use uuid::Uuid;

use crate::admin_fixture::{archive_dir, repository};
use crate::fixtures::{change_type, commit_audit, committer, composition, uv};

/// The two schemas that hold the change-control relations.
const DOMAINS: [&str; 2] = ["clinical", "party"];

/// The rows of `schema` that name a row it does not hold: versions without
/// their CONTRIBUTION, versions without their commit audit, and
/// CONTRIBUTIONs without their commit audit.
async fn dangling(pool: &PgPool, schema: &str) -> (i64, i64, i64) {
    sqlx::query_as(AssertSqlSafe(format!(
        "SELECT \
           (SELECT count(*) FROM {schema}.version v WHERE NOT EXISTS \
              (SELECT 1 FROM {schema}.contribution c WHERE c.id = v.contribution_id)), \
           (SELECT count(*) FROM {schema}.version v WHERE NOT EXISTS \
              (SELECT 1 FROM {schema}.commit_audit a WHERE a.id = v.commit_audit_id)), \
           (SELECT count(*) FROM {schema}.contribution c WHERE NOT EXISTS \
              (SELECT 1 FROM {schema}.commit_audit a WHERE a.id = c.commit_audit_id))"
    )))
    .fetch_one(pool)
    .await
    .expect("count the dangling references")
}

/// No row of either domain dangles after `step`.
async fn holds(pool: &PgPool, step: &str) {
    for schema in DOMAINS {
        assert_eq!(
            dangling(pool, schema).await,
            (0, 0, 0),
            "{step}: {schema}: (versions without their CONTRIBUTION, versions without \
             their audit, CONTRIBUTIONs without their audit)"
        );
    }
}

/// How many rows `table` holds.
async fn rows(pool: &PgPool, table: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {table}")))
        .fetch_one(pool)
        .await
        .expect("count the rows")
}

/// A CONTRIBUTION member creating a COMPOSITION named `name`.
fn creation(name: &str) -> serde_json::Value {
    json!({
        "_type": "ORIGINAL_VERSION",
        "commit_audit": {
            "change_type": change_type("249", "creation"),
            "committer": committer("author")
        },
        "lifecycle_state": change_type("532", "complete"),
        "data": composition(name)
    })
}

#[tokio::test]
async fn every_version_and_contribution_names_stored_rows() {
    let (_db, pool, svc) = repository().await;
    let (ehr, other, full, person) = Box::pin(crate::node_version_integrity::seeded(&svc)).await;
    holds(&pool, "commit, update, logical delete, a party commit").await;

    for members in [1, 2] {
        let body = json!({
            "versions": (0..members).map(|n| creation(&format!("member {n}"))).collect::<Vec<_>>(),
            "audit": commit_audit("249", "creation", "author")
        });
        Box::pin(svc.create_ehr_contribution(ehr, body))
            .await
            .expect("the CONTRIBUTION commits");
    }
    holds(&pool, "CONTRIBUTIONs of one and two members").await;

    let (_source_db, source) = crate::fixtures::service().await;
    let extracted = crate::fixtures::seed_extract_ehr(&source).await.0;
    let mut extracts = source.extract_ehrs(extracted).await.expect("extract");
    svc.import_ehr(
        None,
        openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("an EXTRACT"),
    )
    .await
    .expect("import the EHR");
    holds(&pool, "import").await;

    svc.archive_ehrs(&[ehr, full]).await.expect("archive");
    holds(&pool, "archive").await;
    svc.restore_archived_ehrs(&[ehr, full])
        .await
        .expect("restore");
    holds(&pool, "restore").await;

    let dir = archive_dir();
    svc.export_ehrs(dir.clone(), ExportSpec::canonical_json(1))
        .await
        .expect("export");
    let (_target_db, target_pool, target) = repository().await;
    target
        .template_adl14_upload(&crate::node_version_integrity::ips_file(
            "../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt",
        ))
        .await
        .expect("upload the IPS template");
    let failed = target.load_ehrs(dir.clone()).await.expect("load");
    assert!(failed.is_empty(), "the load reports no failure: {failed:?}");
    assert!(
        rows(&target_pool, "clinical.version").await > 0,
        "the load wrote versions"
    );
    holds(&target_pool, "archive load").await;
    let _ = std::fs::remove_dir_all(&dir);

    svc.admin_ehr_delete(other).await.expect("erase an EHR");
    holds(&pool, "erasure of an EHR").await;
    svc.admin_ehr_delete_all(&[ehr, full])
        .await
        .expect("erase two EHRs");
    holds(&pool, "erasure of two EHRs").await;
    svc.physical_party_delete(person)
        .await
        .expect("erase the person");
    holds(&pool, "erasure of a party").await;
}

/// The audit a version written into a CONTRIBUTION carries.
fn audit() -> AuditRow<'static> {
    AuditRow {
        system_id: "integrity.test",
        change_type: "249",
        description: None,
        committer: committer("author").to_string(),
        attestation: None,
    }
}

/// A content-less first version of a new COMPOSITION in `ehr`.
fn version(ehr: ferroehr::ids::EhrId) -> FoldedVersion<'static> {
    FoldedVersion {
        vo_id: ferroehr::ids::VoId(Uuid::now_v7()),
        kind: "COMPOSITION",
        ehr_id: Some(ehr),
        sys_version: 1,
        trunk_version: 1,
        branch_number: 0,
        branch_version: 0,
        lifecycle_state: "532",
        creating_system_id: "integrity.test",
        preceding_version_uid: None,
        other_input_version_uids: &[],
        template_id: None,
        signature: None,
        signature_client_supplied: false,
        stable_compatible: true,
        origins: &[],
        body: None,
        time_committed: jiff::Timestamp::now(),
        rows: &[],
        blob_uris: &[],
        content_digest: None,
        content_sys_version: None,
        root_archetype: None,
        episodes: &[],
    }
}

#[tokio::test]
async fn a_version_into_a_contribution_not_stored_is_refused() {
    let (_db, pool, svc) = repository().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let versions = rows(&pool, "clinical.version").await;
    let mut conn = ferroehr::db::acquire::acquire(&pool)
        .await
        .expect("a connection");
    let mut tx = conn.begin().await.expect("begin");
    let refused = commit_folded(
        &mut tx,
        &audit(),
        Folded::Into {
            id: Uuid::now_v7(),
            version: version(ehr),
        },
    )
    .await;
    assert!(
        matches!(refused, Err(StorageError::DanglingReference(_))),
        "{refused:?}"
    );
    let written: i64 = sqlx::query_scalar("SELECT count(*) FROM clinical.version")
        .fetch_one(&mut *tx)
        .await
        .expect("count the versions");
    assert_eq!(written, versions, "the statement wrote no version");
}

#[tokio::test]
async fn an_imported_version_naming_rows_not_stored_is_refused() {
    let (_db, pool, svc) = repository().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let versions = rows(&pool, "clinical.version").await;
    let wrapped = json!({});
    let mut conn = ferroehr::db::acquire::acquire(&pool)
        .await
        .expect("a connection");
    let refused = insert_imported_vo_version(
        &mut conn,
        &ImportedVersionRow {
            vo_id: ferroehr::ids::VoId(Uuid::now_v7()),
            kind: "COMPOSITION",
            ehr_id: Some(ehr),
            sys_version: 1,
            trunk_version: 1,
            branch_number: 0,
            branch_version: 0,
            lifecycle_state: "532",
            creating_system_id: "integrity.test",
            preceding_version_uid: None,
            other_input_version_uids: &[],
            contribution_id: Uuid::now_v7(),
            commit_audit_id: Uuid::now_v7(),
            signature: None,
            wrapped_original: &wrapped,
            committed_at: jiff::Timestamp::now(),
            body: None,
        },
    )
    .await;
    assert!(
        matches!(refused, Err(StorageError::DanglingReference(_))),
        "{refused:?}"
    );
    assert_eq!(rows(&pool, "clinical.version").await, versions);
}

#[tokio::test]
async fn an_archive_load_naming_rows_not_stored_writes_nothing() {
    let (_db, pool, svc) = repository().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let stored = svc
        .create_composition(ehr, uv(&composition("stored"), "249", None))
        .await
        .expect("commit");
    let (contribution, audit): (Uuid, Uuid) = sqlx::query_as(
        "SELECT contribution_id, commit_audit_id FROM clinical.version WHERE vo_id = $1",
    )
    .bind(stored.vo_id)
    .fetch_one(&pool)
    .await
    .expect("the stored version's references");
    let versions = rows(&pool, "clinical.version").await;
    let mut conn = ferroehr::db::acquire::acquire(&pool)
        .await
        .expect("a connection");
    let row = |contribution_id, commit_audit_id| VerbatimVersionRow {
        vo_id: ferroehr::ids::VoId(Uuid::now_v7()),
        kind: "COMPOSITION",
        ehr_id: Some(ehr),
        sys_version: 1,
        trunk_version: 1,
        branch_number: 0,
        branch_version: 0,
        preceding_version_uid: None,
        other_input_version_uids: None,
        committed_at: None,
        lifecycle_state: "532",
        contribution_id,
        commit_audit_id,
        template_id: None,
        signature: None,
        signature_client_supplied: false,
        creating_system_id: "integrity.test",
        wrapped_original: None,
        body: None,
    };
    for (what, batch) in [
        (
            "a CONTRIBUTION not stored",
            [row(contribution, audit), row(Uuid::now_v7(), audit)],
        ),
        (
            "an audit not stored",
            [row(contribution, audit), row(contribution, Uuid::now_v7())],
        ),
    ] {
        let refused = insert_versions_verbatim(&mut conn, &batch).await;
        assert!(
            matches!(refused, Err(StorageError::DanglingReference(_))),
            "{what}: {refused:?}"
        );
        assert_eq!(
            rows(&pool, "clinical.version").await,
            versions,
            "{what}: no row of the batch is written"
        );
    }

    let contributions = rows(&pool, "clinical.contribution").await;
    for ehr_id in [Some(ehr), None] {
        let refused = ferroehr::storage::dump_repo::write::insert_contributions(
            &mut conn,
            ehr_id,
            &[
                ferroehr::storage::dump_repo::ContributionRow {
                    id: Uuid::now_v7(),
                    commit_audit_id: audit,
                },
                ferroehr::storage::dump_repo::ContributionRow {
                    id: Uuid::now_v7(),
                    commit_audit_id: Uuid::now_v7(),
                },
            ],
        )
        .await;
        assert!(
            matches!(refused, Err(StorageError::DanglingReference(_))),
            "{ehr_id:?}: {refused:?}"
        );
        assert_eq!(
            rows(&pool, "clinical.contribution").await,
            contributions,
            "{ehr_id:?}: no CONTRIBUTION of the batch is written"
        );
    }
}

#[tokio::test]
async fn erasure_keeps_an_audit_a_stored_row_names() {
    let (_db, pool, svc) = repository().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let stored = svc
        .create_composition(ehr, uv(&composition("stored"), "249", None))
        .await
        .expect("commit");
    let audit: Uuid =
        sqlx::query_scalar("SELECT commit_audit_id FROM clinical.version WHERE vo_id = $1")
            .bind(stored.vo_id)
            .fetch_one(&pool)
            .await
            .expect("the stored version's audit");
    let audits = rows(&pool, "clinical.commit_audit").await;
    let mut conn = ferroehr::db::acquire::acquire(&pool)
        .await
        .expect("a connection");
    let refused =
        ferroehr::storage::erasure::delete_commit_audits(&mut conn, &[Uuid::now_v7(), audit]).await;
    assert!(
        matches!(refused, Err(StorageError::DanglingReference(_))),
        "{refused:?}"
    );
    assert_eq!(
        rows(&pool, "clinical.commit_audit").await,
        audits,
        "no audit of the set is deleted"
    );
    holds(&pool, "a refused audit delete").await;
}
