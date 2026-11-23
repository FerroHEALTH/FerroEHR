// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The clinical-side data-minimisation pass reaches the two write paths that
//! replay content verbatim (#3237): EHR-Extract import and the admin archive
//! load. Both store a record exactly as received, so a body identifying its
//! subject can only be REFUSED, never rewritten; these tests prove the
//! refusal, and that nothing of the refused record lands.

use ferroehr::error::ServiceError;
use ferroehr::error::refusal::Invalid;
use std::sync::Arc;

use serde_json::json;
use sqlx::PgPool;

use ferroehr::ids::EhrId;
use ferroehr::privacy::PrivacyPolicy;
use ferroehr::privacy::config::PrivacyConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::types::ExportSpec;
use openehr_rm::v1_2::ehr_extract::common::extract::Extract;

use crate::admin_fixture::archive_dir;
use crate::fixtures::uv;

/// The subject's name, synthetic, which the refusal must never echo.
const SUBJECT_NAME: &str = "Sybrand Veenstra";

/// A repository that accepts identified parties: the source side, which has to
/// hold the content the target refuses.
fn permissive(pool: &PgPool) -> FerroEhrService {
    let policy = PrivacyPolicy::from(&PrivacyConfig {
        allow_identified_parties_in_ehr: true,
        ..PrivacyConfig::default()
    });
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_privacy(Arc::new(policy))
}

/// A repository under the configuration default, refusing identified parties.
fn strict(pool: &PgPool) -> FerroEhrService {
    let policy = PrivacyPolicy::from(&PrivacyConfig::default());
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_privacy(Arc::new(policy))
}

/// An EHR holding a COMPOSITION whose composer is the record's subject by
/// name, a `PARTY_RELATED` coded `self`, seeded through the opt-in.
async fn seed_ehr_with_named_subject(source: &FerroEhrService) -> EhrId {
    let ehr = crate::fixtures::create_ehr(source).await;
    let mut composition = crate::fixtures::composition("subject named");
    composition["composer"] = json!({
        "_type": "PARTY_RELATED",
        "name": SUBJECT_NAME,
        "relationship": {
            "_type": "DV_CODED_TEXT",
            "value": "self",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "0"
            }
        }
    });
    source
        .create_composition(ehr, uv(&composition, "249", None))
        .await
        .expect("the opt-in stores the named subject");
    ehr
}

async fn version_rows(pool: &PgPool, ehr: EhrId) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM version WHERE ehr_id = $1")
        .bind(uuid::Uuid::from(ehr))
        .fetch_one(pool)
        .await
        .expect("count")
}

/// An EHR-Extract whose COMPOSITION names its subject is refused by the
/// import with the commit path's own `422`, naming the RM path, and the target
/// holds nothing of it.
#[tokio::test]
async fn an_extract_naming_its_subject_is_refused_on_import() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = permissive(&source_db.pool());
    let target_db = testkit::db().await.expect("testkit database");
    let target = strict(&target_db.pool());

    let ehr = seed_ehr_with_named_subject(&source).await;
    let mut extracts = source.extract_ehrs(ehr).await.expect("export");
    let extract: Extract =
        openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("EXTRACT deserializes");

    let refused = target
        .import_ehr(None, extract)
        .await
        .expect_err("the named subject refuses the import");
    assert!(
        matches!(
            &refused,
            ServiceError::Unprocessable {
                kind: Invalid::Content,
                ..
            }
        ),
        "{refused}"
    );
    assert!(
        refused.message().contains("composer/name"),
        "the refusal names the RM path: {refused}"
    );
    assert!(
        !refused.message().contains(SUBJECT_NAME),
        "the refusal never carries the name itself: {refused}"
    );
    assert_eq!(
        version_rows(&target_db.pool(), ehr).await,
        0,
        "nothing of the refused EHR landed"
    );
}

/// An archive whose COMPOSITION names its subject is refused by the load
/// for that EHR, reported rather than fatal, and the target holds nothing of it.
#[tokio::test]
async fn an_archive_naming_its_subject_is_refused_on_load() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = permissive(&source_db.pool());
    let target_db = testkit::db().await.expect("testkit database");
    let target = strict(&target_db.pool());

    let ehr = seed_ehr_with_named_subject(&source).await;
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
        error.contains("composer/name"),
        "the refusal names the RM path: {error}"
    );
    assert!(
        !error.contains(SUBJECT_NAME),
        "the report never carries the name: {error}"
    );
    assert_eq!(
        version_rows(&target_db.pool(), ehr).await,
        0,
        "nothing of the refused EHR landed"
    );

    std::fs::remove_dir_all(&dir).expect("the archive directory is removed");
}
