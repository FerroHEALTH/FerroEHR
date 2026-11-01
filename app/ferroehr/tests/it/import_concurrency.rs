// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Two concurrent imports into one version container (#3658).
//!
//! Each import numbers the versions it lands from the container's stored state,
//! so two imports racing on one container must be serialised: the second one
//! sees the first one's versions and answers the re-import conflict RM common
//! `docs/specs/openehr/RM/docs/common/master06-change_control_package.adoc`
//! §Copying implies for a version already held, never a bare unique-index
//! violation.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use ferroehr::service::error::ServiceError;
use ferroehr::service::refusal::Clash;
use serde_json::{Value, json};

use openehr_rm::v1_2::ehr_extract::common::extract::Extract;
use openehr_rm::v1_2::ehr_extract::common::extract_spec::ExtractSpec;

use ferroehr::db::domain::DomainPools;
use ferroehr::service::FerroEhrService;

use crate::fixtures::uv;

/// The `X_VERSIONED_FOLDER` content item of an exported extract.
fn folder_item(extract: &Value) -> &Value {
    extract["chapters"][0]["items"]
        .as_array()
        .expect("chapter items")
        .iter()
        .find(|it| it["item"]["_type"] == json!("X_VERSIONED_FOLDER"))
        .expect("a FOLDER in the export")
}

/// An extract carrying only the directory `FOLDER` of a freshly seeded source
/// EHR, so importing it into another EHR is a first receipt of one container.
async fn folder_only_extract(source: &FerroEhrService) -> Extract {
    let ehr = Box::pin(source.create_ehr_meta(None, None))
        .await
        .map(|(id, _)| id)
        .expect("source ehr");
    source
        .create_directory(
            ehr,
            uv(
                &json!({
                    "_type": "FOLDER",
                    "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1",
                    "archetype_details": {
                        "_type": "ARCHETYPED",
                        "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-FOLDER.generic.v1" },
                        "rm_version": "1.1.0"
                    },
                    "name": { "_type": "DV_TEXT", "value": "root" }
                }),
                "249",
                None,
            ),
        )
        .await
        .expect("directory");
    let whole = source.extract_ehrs(ehr).await.expect("whole export");
    let folder_vo = folder_item(&whole[0])["item"]["uid"]["value"]
        .as_str()
        .expect("folder vo uid")
        .to_owned();
    let spec: ExtractSpec = openehr_its::json::from_canonical_value(&json!({
        "_type": "EXTRACT_SPEC",
        "manifest": {
            "_type": "EXTRACT_MANIFEST",
            "entities": [ {
                "_type": "EXTRACT_ENTITY_MANIFEST",
                "extract_id_key": ehr.to_string(),
                "ehr_id": ehr.to_string(),
                "other_ids": [],
                "item_list": [ {
                    "_type": "OBJECT_REF",
                    "namespace": "local",
                    "type": "VERSIONED_FOLDER",
                    "id": { "_type": "HIER_OBJECT_ID", "value": folder_vo }
                } ]
            } ]
        },
        "extract_type": {
            "_type": "DV_CODED_TEXT",
            "value": "openehr-ehr",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "openehr-ehr"
            }
        },
        "include_multimedia": true,
        "priority": 0,
        "link_depth": 0,
        "criteria": []
    }))
    .expect("EXTRACT_SPEC");
    let mut extracts = source
        .export_ehr_extracts(spec)
        .await
        .expect("folder-only export");
    openehr_its::json::from_canonical_value(&extracts.remove(0)).expect("folder EXTRACT")
}

/// Two imports of the same container run at once: exactly one lands it, the
/// other waits for it and is refused as a re-import of the version it now
/// holds, and the container ends with that one version.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn concurrent_imports_into_one_container_are_serialised() {
    let source_db = testkit::db().await.expect("testkit database");
    let source = FerroEhrService::new(&DomainPools::from_shared(&source_db.pool()));
    let target_db = testkit::db().await.expect("testkit database");
    let target = FerroEhrService::new(&DomainPools::from_shared(&target_db.pool()));

    let extract = folder_only_extract(&source).await;
    let ehr = Box::pin(target.create_ehr_meta(None, None))
        .await
        .map(|(id, _)| id)
        .expect("target ehr");

    let (first, second) = tokio::join!(
        target.import_ehr_extract(ehr, extract.clone()),
        target.import_ehr_extract(ehr, extract),
    );
    let refused = match (first, second) {
        (Ok(()), Err(refused)) | (Err(refused), Ok(())) => refused,
        (Ok(()), Ok(())) => panic!("both imports landed the same version"),
        (Err(a), Err(b)) => panic!("neither import landed: {a} / {b}"),
    };
    assert!(
        matches!(&refused, ServiceError::Conflict(r) if r.kind == Clash::State),
        "{refused}"
    );
    assert!(
        refused.message().contains("cannot re-import trunk version"),
        "the loser sees the winner's version, not a unique-index collision: {refused}"
    );

    let export = target.extract_ehrs(ehr).await.expect("re-export target");
    let versions = folder_item(&export[0])["item"]["versions"]
        .as_array()
        .expect("folder versions");
    assert_eq!(
        versions.len(),
        1,
        "the container holds the one landed version"
    );
}
