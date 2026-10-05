// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The clinical-side data-minimisation pass reaches the two write paths that
//! replay content verbatim (#3237): EHR-Extract import and the admin archive
//! load. Both store a record exactly as received, so a body carrying an
//! identifier can only be REFUSED, never rewritten; these tests prove the
//! refusal, and that nothing of the refused record lands.
//!
//! The source repository runs an explicitly permissive policy (no rules, the
//! identified-party opt-in), so it will hold a FOLDER whose name is a synthetic
//! national identifier; the target runs the configuration default, every
//! shipped rule in `strict` mode. The library default is the refusing posture
//! (#3243), so the permissive side has to say so.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use serde_json::json;
use sqlx::PgPool;

use ferroehr::ids::EhrId;
use ferroehr::privacy::PrivacyPolicy;
use ferroehr::privacy::config::{IdentifierScanConfig, PrivacyConfig, ScanMode};
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::types::ExportSpec;
use ferroehr::service::status::CallStatusType;
use openehr_rm::v1_2::ehr_extract::common::extract::Extract;

use crate::admin_fixture::archive_dir;
use crate::fixtures::uv;

/// A synthetic BSN that passes the eleven-test, so the shipped `nl-bsn` rule
/// matches it.
const SYNTHETIC_BSN: &str = "111222333"; // privacy-allow: synthetic

/// A repository that accepts anything: the source side, which has to hold the
/// identifier the target refuses.
fn permissive(pool: &PgPool) -> FerroEhrService {
    let policy = PrivacyPolicy::compile(&PrivacyConfig {
        allow_identified_parties_in_ehr: true,
        identifier_scan: IdentifierScanConfig {
            mode: ScanMode::Warn,
            rules: Vec::new(),
            patterns: Vec::new(),
        },
        ..PrivacyConfig::default()
    })
    .expect("the permissive policy compiles");
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_privacy(Arc::new(policy))
}

/// A repository under the configuration default: every rule, `strict`.
fn strict(pool: &PgPool) -> FerroEhrService {
    let policy =
        PrivacyPolicy::compile(&PrivacyConfig::default()).expect("the default policy compiles");
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_privacy(Arc::new(policy))
}

/// An EHR whose directory FOLDER is named with the synthetic identifier,
/// seeded through a repository that scans nothing.
async fn seed_ehr_with_identifier_in_folder(source: &FerroEhrService) -> EhrId {
    let ehr = source.create_ehr(None).await.expect("ehr");
    source
        .create_directory(
            ehr,
            uv(
                &json!({
                    "_type": "FOLDER",
                    "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                    "name": { "_type": "DV_TEXT", "value": SYNTHETIC_BSN }
                }),
                "249",
                None,
            ),
        )
        .await
        .expect("directory with the identifier in its name");
    ehr
}

async fn version_rows(pool: &PgPool, ehr: EhrId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM version WHERE ehr_id = $1")
        .bind(uuid::Uuid::from(ehr))
        .fetch_one(pool)
        .await
        .expect("count")
}

/// An EHR-Extract whose FOLDER names a national identifier is refused by the
/// import with the commit path's own `422`, naming the RM path, and the target
/// holds nothing of it.
#[tokio::test]
async fn an_extract_carrying_an_identifier_is_refused_on_import() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = permissive(&source_db.pool());
    let target_db = testkit::db().await.expect("testkit database");
    let target = strict(&target_db.pool());

    let ehr = seed_ehr_with_identifier_in_folder(&source).await;
    let mut extracts = source.extract_ehrs(ehr).await.expect("export");
    let extract: Extract =
        openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("EXTRACT deserializes");

    let refused = target
        .import_ehr(None, extract)
        .await
        .expect_err("the identifier in the FOLDER name refuses the import");
    assert_eq!(refused.status, CallStatusType::ContentInvalid, "{refused}");
    assert!(
        refused.message.contains("FOLDER") && refused.message.contains("name"),
        "the refusal names the RM path: {refused}"
    );
    assert!(
        !refused.message.contains(SYNTHETIC_BSN),
        "the refusal never carries the identifier itself: {refused}"
    );
    assert_eq!(
        version_rows(&target_db.pool(), ehr).await,
        0,
        "nothing of the refused EHR landed"
    );
}

/// An archive whose FOLDER names a national identifier is refused by the load
/// for that EHR, reported rather than fatal, and the target holds nothing of it.
#[tokio::test]
async fn an_archive_carrying_an_identifier_is_refused_on_load() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = permissive(&source_db.pool());
    let target_db = testkit::db().await.expect("testkit database");
    let target = strict(&target_db.pool());

    let ehr = seed_ehr_with_identifier_in_folder(&source).await;
    let dir = archive_dir();
    source
        .export_ehrs(dir.clone(), ExportSpec::canonical_json(1024))
        .await
        .expect("export");

    let reports = target.load_ehrs(dir.clone()).await.expect("the load runs");
    let report = reports
        .iter()
        .find(|r| r.entity_id == ehr.to_string())
        .expect("the refused EHR is reported");
    assert!(
        !report.dump_status,
        "the load of that EHR failed: {report:?}"
    );
    let error = report
        .error
        .as_deref()
        .expect("with the refusal as its error");
    assert!(
        error.contains("FOLDER") && error.contains("name"),
        "the refusal names the RM path: {error}"
    );
    assert!(
        !error.contains(SYNTHETIC_BSN),
        "the report never carries the identifier: {error}"
    );
    assert_eq!(
        version_rows(&target_db.pool(), ehr).await,
        0,
        "nothing of the refused EHR landed"
    );

    std::fs::remove_dir_all(&dir).expect("the archive directory is removed");
}
