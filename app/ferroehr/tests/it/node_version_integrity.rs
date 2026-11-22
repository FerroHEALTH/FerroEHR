// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A version's node rows exist exactly when the version does (#5969).
//!
//! The node rows carry no foreign key into `version`: every writer inserts
//! them in the transaction of their version, every delete of a version
//! deletes them in the same statement, and every tier move names them. These
//! tests hold that across commit, update, shared content, logical delete,
//! archive, restore, party erasure, EHR erasure and decommissioning, and show
//! that the storage-parity sweep reports and the rebuild repairs a row set no
//! version reads. No openEHR spec governs storage mechanics: our own design.

use ferroehr::db::DbConfig;
use ferroehr::decommission::Decommission;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::integrity::{StorageDomain, StorageParityDefect};
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::ehr::scope::Unscoped;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::storage::integrity::StorageParityScope;
use ferrohealth_secret::secret_url::SecretUrl;
use sqlx::AssertSqlSafe;
use sqlx::PgPool;

use crate::admin_fixture::{repository, seed_full_ehr};
use crate::fixtures::{composition, uv};
use crate::pseudonymisation_boundary::a_person;
use crate::typed_body::typed;

/// The two schemas that hold a `version` and a `node` relation.
const DOMAINS: [&str; 2] = ["clinical", "party"];

/// The breaches of the invariant in `schema`: node row sets no version of
/// the same tier reads, content versions without their root row, bodiless
/// versions with rows, and rows whose shared members name no earlier holder
/// of their version and tier (#6229).
async fn breaches(pool: &PgPool, schema: &str) -> (i64, i64, i64, i64) {
    let unreached: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT count(*) FROM (SELECT DISTINCT n.tier, n.vo_id, n.sys_version \
         FROM {schema}.node n WHERE NOT EXISTS (SELECT 1 FROM {schema}.version v \
         WHERE v.tier = n.tier AND v.vo_id = n.vo_id AND v.content_key = n.sys_version)) s"
    )))
    .fetch_one(pool)
    .await
    .expect("count the unreached node rows");
    let missing: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT count(*) FROM {schema}.version v \
         WHERE v.body IS NOT NULL AND v.content_key = v.sys_version \
         AND NOT EXISTS (SELECT 1 FROM {schema}.node n WHERE n.tier = v.tier \
         AND n.vo_id = v.vo_id AND n.sys_version = v.sys_version AND n.num = 0)"
    )))
    .fetch_one(pool)
    .await
    .expect("count the versions without rows");
    let bodiless: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT count(*) FROM {schema}.version v \
         WHERE v.body IS NULL AND EXISTS (SELECT 1 FROM {schema}.node n \
         WHERE n.tier = v.tier AND n.vo_id = v.vo_id AND n.sys_version = v.content_key)"
    )))
    .fetch_one(pool)
    .await
    .expect("count the bodiless versions with rows");
    let dangling: i64 = sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT count(*) FROM {schema}.node n WHERE NOT {}",
        ferroehr::storage::shared::references_hold("n")
            .replace(" node h ", &format!(" {schema}.node h "))
    )))
    .fetch_one(pool)
    .await
    .expect("count the rows with a dangling reference");
    (unreached, missing, bodiless, dangling)
}

/// How many node rows of `schema` leave a member to an earlier holder.
async fn sharing_rows(pool: &PgPool, schema: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(format!(
        "SELECT count(*) FROM {schema}.node WHERE shared IS NOT NULL"
    )))
    .fetch_one(pool)
    .await
    .expect("count the sharing rows")
}

/// The invariant holds in both domains after `step`, and the sweep agrees.
async fn holds(pool: &PgPool, svc: &FerroEhrService, step: &str) {
    for schema in DOMAINS {
        assert_eq!(
            breaches(pool, schema).await,
            (0, 0, 0, 0),
            "{step}: {schema}: (unreached row sets, content versions without rows, \
             bodiless versions with rows, dangling shared references)"
        );
    }
    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("the sweep runs");
    assert!(report.is_clean(), "{step}: the sweep is clean: {report:?}");
}

/// How many node rows `schema` holds.
async fn node_rows(pool: &PgPool, schema: &str) -> i64 {
    sqlx::query_scalar(AssertSqlSafe(format!("SELECT count(*) FROM {schema}.node")))
        .fetch_one(pool)
        .await
        .expect("count the node rows")
}

/// Two EHRs, each with a composition committed, updated, updated to the
/// same content and logically deleted; an EHR of every other object kind; and
/// a person.
async fn seeded(svc: &FerroEhrService) -> (EhrId, EhrId, EhrId, ferroehr::ids::VoId) {
    let ehr = crate::fixtures::create_ehr(svc).await;
    let other = crate::fixtures::create_ehr(svc).await;
    let full = seed_full_ehr(svc).await;
    for target in [ehr, other] {
        let created = svc
            .create_composition(target, uv(&composition("first"), "249", None))
            .await
            .expect("commit");
        let changed = svc
            .update_composition(
                target,
                created.vo_id,
                uv(&composition("second"), "251", Some(&created.version_uid())),
            )
            .await
            .expect("update to new content");
        let same = svc
            .update_composition(
                target,
                created.vo_id,
                uv(&composition("second"), "251", Some(&changed.version_uid())),
            )
            .await
            .expect("update to the same content");
        let doomed = svc
            .create_composition(target, uv(&composition("doomed"), "249", None))
            .await
            .expect("commit the composition to delete");
        svc.delete_composition(
            target,
            &doomed.version_uid().parse().expect("a version uid"),
            WriteRequest::new(()),
            &Unscoped,
        )
        .await
        .expect("logical delete");
        assert!(
            same.sys_version > changed.sys_version,
            "the unchanged update is a version of its own"
        );
    }
    // The IPS composition repeats one participation list on its entries, so
    // its rows share it (#6229) through every step that follows.
    svc.template_adl14_upload(&ips_file(
        "../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt",
    ))
    .await
    .expect("upload the IPS template");
    let ips: serde_json::Value = serde_json::from_str(&ips_file(
        "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
    ))
    .expect("the IPS composition parses");
    svc.create_composition(ehr, uv(&ips, "249", None))
        .await
        .expect("commit the IPS composition");
    let person = svc
        .party_create(PartyKind::Person, WriteRequest::new(typed(&a_person())))
        .await
        .expect("create a person");
    (ehr, other, full, crate::fixtures::written_vo(&person))
}

/// A file of the workspace, relative to this crate.
fn ips_file(rel: &str) -> String {
    let path = format!("{}/{rel}", env!("CARGO_MANIFEST_DIR"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[tokio::test]
async fn a_versions_nodes_exist_exactly_when_the_version_does() {
    let (db, pool, svc) = repository().await;
    let (ehr, other, full, person) = Box::pin(seeded(&svc)).await;
    assert!(
        sharing_rows(&pool, "clinical").await > 0,
        "the IPS rows share their list"
    );
    holds(
        &pool,
        &svc,
        "commit, update, shared content and logical delete",
    )
    .await;

    svc.archive_ehrs(&[ehr, full]).await.expect("archive");
    holds(&pool, &svc, "archive").await;
    let cold: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM clinical.node WHERE tier = 'cold' AND shared IS NOT NULL",
    )
    .fetch_one(&pool)
    .await
    .expect("count the archived sharing rows");
    assert_eq!(
        cold,
        sharing_rows(&pool, "clinical").await,
        "the sharing rows move to the cold tier with their holders"
    );
    svc.restore_archived_ehrs(&[ehr, full])
        .await
        .expect("restore");
    holds(&pool, &svc, "restore").await;

    svc.archive_ehrs(&[other])
        .await
        .expect("archive the other EHR");
    svc.admin_ehr_delete(other)
        .await
        .expect("erase the archived EHR");
    holds(&pool, &svc, "erasure of an archived EHR").await;
    svc.admin_ehr_delete(ehr).await.expect("erase the EHR");
    holds(&pool, &svc, "erasure of an EHR").await;
    svc.admin_ehr_delete(full)
        .await
        .expect("erase the full EHR");
    assert_eq!(node_rows(&pool, "clinical").await, 0, "no EHR is left");

    svc.physical_party_delete(person)
        .await
        .expect("erase the person");
    holds(&pool, &svc, "erasure of a party").await;

    ferroehr::usage_report::store::ensure_instance(&pool)
        .await
        .expect("store an instance id");
    let decommission = Decommission::new(DbConfig {
        url: SecretUrl::new(db.url().to_owned()),
        ..DbConfig::default()
    });
    let confirmation = decommission
        .inventory()
        .await
        .expect("the dry run reads")
        .confirmation()
        .expect("the instance id confirms the erase");
    decommission
        .erase(&confirmation)
        .await
        .expect("decommission");
    for schema in DOMAINS {
        let (version, node): (Option<String>, Option<String>) =
            sqlx::query_as(AssertSqlSafe(format!(
                "SELECT to_regclass('{schema}.version')::text, to_regclass('{schema}.node')::text"
            )))
            .fetch_one(&pool)
            .await
            .expect("look the relations up");
        assert_eq!(
            (version, node),
            (None, None),
            "decommissioning drops {schema}'s node rows with its versions"
        );
    }
}

/// Node rows left behind by a deleted version, or stranded in the other tier,
/// are reported `orphan_nodes` and deleted by the rebuild.
#[tokio::test]
async fn the_sweep_reports_and_the_rebuild_deletes_node_rows_no_version_reads() {
    let (_db, pool, svc) = repository().await;
    let ehr = crate::fixtures::create_ehr(&svc).await;
    let left = svc
        .create_composition(ehr, uv(&composition("left behind"), "249", None))
        .await
        .expect("commit");
    let stranded = svc
        .create_composition(ehr, uv(&composition("stranded"), "249", None))
        .await
        .expect("commit");
    for statement in [
        "DELETE FROM version WHERE vo_id = $1",
        "DELETE FROM vo_head WHERE vo_id = $1",
    ] {
        sqlx::query(statement)
            .bind(left.vo_id)
            .execute(&pool)
            .await
            .expect("drop the version without its rows");
    }
    sqlx::query("UPDATE node SET tier = 'cold' WHERE vo_id = $1")
        .bind(stranded.vo_id)
        .execute(&pool)
        .await
        .expect("strand the rows in the cold tier");

    let report = svc
        .verify_storage_parity(StorageParityScope::default())
        .await
        .expect("the sweep runs");
    let mut found: Vec<_> = report
        .mismatches
        .iter()
        .map(|m| (m.vo_id, m.domain, m.kind.as_str(), m.defect))
        .collect();
    found.sort_by_key(|(vo, _, _, defect)| (*vo, defect.as_str()));
    let mut expected = vec![
        (
            left.vo_id.0,
            StorageDomain::Clinical,
            "COMPOSITION",
            StorageParityDefect::OrphanNodes,
        ),
        (
            stranded.vo_id.0,
            StorageDomain::Clinical,
            "COMPOSITION",
            StorageParityDefect::NodesMissing,
        ),
        (
            stranded.vo_id.0,
            StorageDomain::Clinical,
            "COMPOSITION",
            StorageParityDefect::OrphanNodes,
        ),
    ];
    expected.sort_by_key(|(vo, _, _, defect)| (*vo, defect.as_str()));
    assert_eq!(found, expected, "{report:?}");

    let scoped = svc
        .verify_storage_parity(StorageParityScope {
            vo_id: Some(left.vo_id.0),
            ..StorageParityScope::default()
        })
        .await
        .expect("the scoped sweep runs");
    assert_eq!(scoped.mismatch_count, 1, "{scoped:?}");

    let rebuilt = svc
        .rebuild_version_nodes(StorageParityScope::default())
        .await
        .expect("the rebuild runs");
    assert!(rebuilt.is_complete(), "{rebuilt:?}");
    holds(&pool, &svc, "the rebuild").await;
}
