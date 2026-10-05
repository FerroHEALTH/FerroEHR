// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end service tests for the EHR Extract **export** path (SM
//! `I_EHR_EXTRACT_SERVICE.export_ehrs` / `export_ehr_extracts`) against a real
//! `PostgreSQL` 18 (shared testkit harness).
//!
//! Spec: SM `docs/specs/openehr/SM/docs/UML/classes/i_ehr_extract_service.adoc`;
//! RM EHR Extract IM master05 (`X_VERSIONED_*`) + master09 (creation
//! semantics). The acceptance properties:
//!
//! 1. **Whole-EHR export** carries every versioned object of the EHR
//!    (`EHR_STATUS`, `EHR_ACCESS`, directory `FOLDER`) as an
//!    `OPENEHR_CONTENT_ITEM` wrapping the matching `X_VERSIONED_<kind>`, latest
//!    version only, with the version data byte-equal (canonical JSON) to what
//!    the read surface serves.
//! 2. **Spec-driven export** honours the `EXTRACT_ENTITY_MANIFEST.item_list`
//!    (only the named version container) and the `EXTRACT_VERSION_SPEC`
//!    (`include_all_versions` ⇒ every version; `include_revision_history` ⇒ the
//!    full `REVISION_HISTORY`).

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use serde_json::{Value, json};

use openehr_rm::prelude::PartyProxy;
use openehr_rm::v1_2::ehr_extract::common::extract_spec::ExtractSpec;

use ferroehr::service::FerroEhrService;
use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};

fn uv<T: serde::de::DeserializeOwned>(
    data: &Value,
    change_code: &str,
    preceding: Option<&str>,
) -> UpdateVersion<T> {
    UpdateVersion {
        preceding_version_uid: preceding.map(|p| p.parse().expect("OBJECT_VERSION_ID")),
        lifecycle_state: lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the fixture commit body decodes as its RM type"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded(change_code),
            description: None,
            committer: openehr_its::json::from_canonical_value::<PartyProxy>(
                &json!({ "_type": "PARTY_IDENTIFIED", "name": "conformance tester" }),
            )
            .expect("committer"),
            additional_properties: std::collections::BTreeMap::new(),
        }),
        signature: None,
        additional_properties: std::collections::BTreeMap::new(),
    }
}

/// Seed an EHR with an `EHR_STATUS` (create → update = two versions), a directory
/// `FOLDER`, and the auto-created `EHR_ACCESS`. Returns the EHR id and the
/// `EHR_STATUS` version-container uid.
async fn seed_ehr(svc: &FerroEhrService) -> (ferroehr::ids::EhrId, String) {
    let ehr_uuid = svc.create_ehr(None).await.expect("ehr");

    let mut status = svc
        .get_ehr_status_at_time(ehr_uuid, None)
        .await
        .expect("status get");
    let status_ovid = status["uid"]["value"].as_str().expect("uid").to_owned();
    let status_vo = status_ovid.split("::").next().unwrap().to_owned();
    status.as_object_mut().expect("status obj").remove("uid");

    svc.create_directory(
        ehr_uuid,
        uv(
            &json!({ "_type": "FOLDER", "archetype_node_id": "openEHR-EHR-FOLDER.generic.v1", "name": { "_type": "DV_TEXT", "value": "root" } }),
            "249",
            None,
        ),
    )
    .await
    .expect("directory");

    status["is_modifiable"] = json!(false);
    svc.replace_ehr_status(ehr_uuid, uv(&status, "251", Some(&status_ovid)))
        .await
        .expect("status update");

    (ehr_uuid, status_vo)
}

/// The content items of the (single) chapter of an extract.
fn items(extract: &Value) -> &Vec<Value> {
    extract["chapters"][0]["items"]
        .as_array()
        .expect("chapter items")
}

/// The content item whose wrapped object has the given `X_VERSIONED_*` `_type`.
fn find_by_xtype<'a>(extract: &'a Value, xtype: &str) -> Option<&'a Value> {
    items(extract)
        .iter()
        .find(|it| it["item"]["_type"] == json!(xtype))
}

#[tokio::test]
async fn export_ehrs_carries_every_versioned_object_latest_only() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr, _status_vo) = seed_ehr(&svc).await;

    let extracts = svc.extract_ehrs(ehr).await.expect("export_ehrs");
    assert_eq!(
        extracts.len(),
        1,
        "one EHR id → a one-element List<EXTRACT>"
    );
    let extract = &extracts[0];

    // EXTRACT envelope.
    assert_eq!(extract["_type"], json!("EXTRACT"));
    assert_eq!(extract["sequence_nr"], json!(1));
    assert_eq!(extract["chapters"][0]["_type"], json!("EXTRACT_CHAPTER"));
    assert!(
        extract["system_id"]["value"].is_string(),
        "EXTRACT.system_id is a HIER_OBJECT_ID"
    );

    // All three versioned objects present as primary content items.
    for xtype in [
        "X_VERSIONED_EHR_STATUS",
        "X_VERSIONED_EHR_ACCESS",
        "X_VERSIONED_FOLDER",
    ] {
        let item =
            find_by_xtype(extract, xtype).unwrap_or_else(|| panic!("extract must contain {xtype}"));
        assert_eq!(item["_type"], json!("OPENEHR_CONTENT_ITEM"));
        assert_eq!(item["is_primary"], json!(true), "{xtype} is primary");
    }

    // EHR_STATUS: two stored versions, latest only in the extract.
    let status_x = &find_by_xtype(extract, "X_VERSIONED_EHR_STATUS").unwrap()["item"];
    assert_eq!(
        status_x["total_version_count"],
        json!(2),
        "create + update = two versions stored"
    );
    assert_eq!(
        status_x["extract_version_count"],
        json!(1),
        "latest-only default includes one version"
    );
    let versions = status_x["versions"].as_array().expect("versions");
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0]["_type"], json!("ORIGINAL_VERSION"));

    // Canonical-JSON faithfulness: the exported version data is byte-equal to
    // the read surface's current EHR_STATUS — one shape since commit-time uid
    // stamping (#439).
    let current = svc.get_ehr_status_at_time(ehr, None).await.expect("read");
    assert_eq!(
        versions[0]["data"], current,
        "exported EHR_STATUS data must match the stored canonical content"
    );
    assert_eq!(
        versions[0]["data"]["is_modifiable"],
        json!(false),
        "latest EHR_STATUS reflects the update (is_modifiable = false)"
    );
}

#[tokio::test]
async fn export_ehr_extracts_honours_item_list_and_all_versions() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr, status_vo) = seed_ehr(&svc).await;

    // Request only the EHR_STATUS version container, all versions, with revision
    // history.
    let spec: ExtractSpec = openehr_its::json::from_canonical_value(&json!({
        "_type": "EXTRACT_SPEC",
        "version_spec": {
            "_type": "EXTRACT_VERSION_SPEC",
            "include_all_versions": true,
            "include_revision_history": true,
            "include_data": true
        },
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
                    "type": "VERSIONED_EHR_STATUS",
                    "id": { "_type": "HIER_OBJECT_ID", "value": status_vo }
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

    let extracts = svc
        .export_ehr_extracts(spec)
        .await
        .expect("export_ehr_extracts");
    assert_eq!(extracts.len(), 1, "one entity → one EXTRACT");
    let extract = &extracts[0];

    // Only the requested EHR_STATUS container is in the extract.
    let its = items(extract);
    assert_eq!(its.len(), 1, "item_list restricts to one content item");
    let status_x =
        &find_by_xtype(extract, "X_VERSIONED_EHR_STATUS").expect("EHR_STATUS present")["item"];

    // All versions included, plus the full revision history.
    assert_eq!(status_x["total_version_count"], json!(2));
    assert_eq!(status_x["extract_version_count"], json!(2));
    let versions = status_x["versions"].as_array().expect("versions");
    assert_eq!(versions.len(), 2);
    assert_eq!(
        versions[0]["data"]["is_modifiable"],
        json!(true),
        "v1 is the original (modifiable)"
    );
    assert_eq!(
        versions[1]["data"]["is_modifiable"],
        json!(false),
        "v2 is the update (is_modifiable = false)"
    );

    let history = &status_x["revision_history"];
    assert_eq!(history["_type"], json!("REVISION_HISTORY"));
    assert_eq!(
        history["items"].as_array().expect("history items").len(),
        2,
        "revision history has one item per version"
    );
}

#[tokio::test]
async fn export_ehrs_unknown_ehr_is_ehr_id_does_not_exist() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let err = svc
        .extract_ehrs(ferroehr::ids::EhrId(uuid::Uuid::now_v7()))
        .await
        .expect_err("unknown EHR must fail");
    assert_eq!(
        err.status,
        ferroehr::service::status::CallStatusType::EhrIdDoesNotExist
    );
}

/// A1 rm-ehr-extract: `EXTRACT_SPEC.extract_type` must come from the extract
/// content type group; `include_multimedia` = false strips inline `DV_MULTIMEDIA`
/// data from exported versions (master09 §Creation Semantics); an
/// `EXTRACT_CONTENT_ITEM` that is masked yet carries an item violates
/// `Item_validity` on import (`extract_content_item.adoc`).
#[tokio::test]
async fn extract_spec_flags_are_honoured() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr, _status_vo) = seed_ehr(&svc).await;

    // Bad extract_type → precondition.
    let mut spec = json!({
        "_type": "EXTRACT_SPEC",
        "version_spec": { "_type": "EXTRACT_VERSION_SPEC",
            "include_all_versions": false, "include_revision_history": false,
            "include_data": true },
        "manifest": { "_type": "EXTRACT_MANIFEST", "entities": [ {
            "_type": "EXTRACT_ENTITY_MANIFEST",
            "extract_id_key": ehr.to_string(), "ehr_id": ehr.to_string(),
            "other_ids": [], "item_list": [] } ] },
        "extract_type": { "_type": "DV_CODED_TEXT", "value": "bogus",
            "defining_code": { "_type": "CODE_PHRASE", "code_string": "bogus",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" } } },
        "include_multimedia": true, "priority": 0, "link_depth": 0, "criteria": []
    });
    let err = svc
        .export_ehr_extracts(openehr_its::json::from_canonical_value(&spec).expect("spec"))
        .await
        .expect_err("an extract_type outside the group must be rejected");
    assert!(
        err.message.contains("extract content type"),
        "{}",
        err.message
    );

    // …but the accepted set may not be NARROWER than the codes the RM itself
    // names: `master04-common_package.adoc` §Content Criteria Specification
    // lists `openehr-ehr`, `openehr-demographic`, `openehr-synchronisation`,
    // `openehr-generic` and `generic-emr` by example, so refusing one of them
    // would be refusing what must be accepted.
    for named in [
        "openehr-ehr",
        "openehr-demographic",
        "openehr-synchronisation",
        "openehr-generic",
        "generic-emr",
        "other",
    ] {
        spec["extract_type"]["defining_code"]["code_string"] = json!(named);
        svc.export_ehr_extracts(openehr_its::json::from_canonical_value(&spec).expect("spec"))
            .await
            .unwrap_or_else(|e| panic!("extract_type {named:?} must be accepted: {}", e.message));
    }

    // TERM `SupportTerminology/master03-terminology.adoc` §Vocabularies binds
    // the attribute to the `extract_content_type` group (803 "openEHR EHR" …
    // 808 "other"), so every group member coded in the openEHR terminology
    // must be accepted too.
    for term_coded in ["803", "804", "805", "806", "807", "808"] {
        spec["extract_type"]["defining_code"]["code_string"] = json!(term_coded);
        svc.export_ehr_extracts(openehr_its::json::from_canonical_value(&spec).expect("spec"))
            .await
            .unwrap_or_else(|e| {
                panic!(
                    "TERM-coded extract_type {term_coded:?} must be accepted: {}",
                    e.message
                )
            });
    }

    // A TERM group code under a FOREIGN terminology id is not a group member:
    // the openEHR concept codes are meaningful only in the openehr
    // terminology, and "803" is no RM-named token either.
    spec["extract_type"]["defining_code"]["code_string"] = json!("803");
    spec["extract_type"]["defining_code"]["terminology_id"]["value"] = json!("SNOMED-CT");
    svc.export_ehr_extracts(openehr_its::json::from_canonical_value(&spec).expect("spec"))
        .await
        .expect_err("a foreign-terminology 803 must be rejected");
    spec["extract_type"]["defining_code"]["terminology_id"]["value"] = json!("openehr");

    // Valid type + include_multimedia = false → exported bodies carry no
    // inline DV_MULTIMEDIA data.
    spec["extract_type"]["defining_code"]["code_string"] = json!("openehr-ehr");
    spec["include_multimedia"] = json!(false);
    let extracts = svc
        .export_ehr_extracts(openehr_its::json::from_canonical_value(&spec).expect("spec"))
        .await
        .expect("export");
    let wire = serde_json::to_string(&extracts[0]).unwrap();
    assert!(
        !wire.contains("\"data\":\"") || !wire.contains("DV_MULTIMEDIA"),
        "no inline multimedia data may remain when include_multimedia = false"
    );

    // Masked-with-item violates Item_validity on import.
    let mut extract = extracts.into_iter().next().unwrap();
    extract["chapters"][0]["items"][0]["is_masked"] = json!(true);
    let err = svc
        .import_ehr(
            Some(ferroehr::ids::EhrId(uuid::Uuid::now_v7())),
            openehr_its::json::from_canonical_value(&extract).expect("extract"),
        )
        .await
        .expect_err("masked item carrying content must be rejected");
    assert!(err.message.contains("Item_validity"), "{}", err.message);
}

/// The `EXTRACT_SPEC` skeleton the criteria tests share: one entity, no
/// `item_list`, the given criteria list (RM `ehr_extract`
/// `master04-common_package.adoc` §`EXTRACT_SPEC` — criteria "defines in the
/// form of generic queries … which items are to be retrieved from each
/// entity's record").
fn criteria_spec(ehr: &str, criteria: &Value) -> ExtractSpec {
    openehr_its::json::from_canonical_value(&json!({
        "_type": "EXTRACT_SPEC",
        "version_spec": {
            "_type": "EXTRACT_VERSION_SPEC",
            "include_all_versions": false,
            "include_revision_history": false,
            "include_data": true
        },
        "manifest": {
            "_type": "EXTRACT_MANIFEST",
            "entities": [ {
                "_type": "EXTRACT_ENTITY_MANIFEST",
                "extract_id_key": ehr,
                "ehr_id": ehr,
                "other_ids": [],
                "item_list": []
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
        "criteria": criteria
    }))
    .expect("EXTRACT_SPEC")
}

/// #1736 — `EXTRACT_SPEC.criteria` selects the primary set: an AQL criterion
/// whose rows identify the `EHR_STATUS` version container yields an extract
/// carrying exactly that container, not the whole EHR (master04
/// §`EXTRACT_SPEC`: criteria define "which items are to be retrieved";
/// "Query expressions use variables such as $ehr to mean the current EHR").
#[tokio::test]
async fn criteria_select_the_primary_set_ehr_bound() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr, _status_vo) = seed_ehr(&svc).await;

    let spec = criteria_spec(
        &ehr.to_string(),
        &json!([{
            "_type": "DV_PARSABLE",
            "value": "SELECT s/uid/value FROM EHR e CONTAINS EHR_STATUS s",
            "formalism": "AQL"
        }]),
    );
    let extracts = svc
        .export_ehr_extracts(spec)
        .await
        .expect("criteria-driven export");
    assert_eq!(extracts.len(), 1, "one entity → one EXTRACT");
    let its = items(&extracts[0]);
    assert_eq!(
        its.len(),
        1,
        "the criterion selects exactly the EHR_STATUS container: {its:#?}"
    );
    assert!(
        find_by_xtype(&extracts[0], "X_VERSIONED_EHR_STATUS").is_some(),
        "the selected container is the EHR_STATUS"
    );
}

/// #1736, refusal twin (a): a criterion in a formalism this service does not
/// evaluate is a typed precondition refusal — never a silent skip or a silent
/// whole-EHR over-export.
#[tokio::test]
async fn criteria_in_a_foreign_formalism_are_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr, _status_vo) = seed_ehr(&svc).await;

    let spec = criteria_spec(
        &ehr.to_string(),
        &json!([{ "_type": "DV_PARSABLE", "value": "irrelevant", "formalism": "xquery" }]),
    );
    let err = svc
        .export_ehr_extracts(spec)
        .await
        .expect_err("a non-AQL criteria formalism must be refused");
    assert_eq!(
        err.status,
        ferroehr::service::status::CallStatusType::PreconditionViolation
    );
    assert!(
        err.message.contains("formalism"),
        "the refusal names the formalism: {}",
        err.message
    );
}

/// #1736, refusal twin (b): a criterion that does not parse as AQL is a typed
/// precondition refusal naming the criterion index.
#[tokio::test]
async fn criteria_that_do_not_parse_as_aql_are_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let (ehr, _status_vo) = seed_ehr(&svc).await;

    let spec = criteria_spec(
        &ehr.to_string(),
        &json!([{ "_type": "DV_PARSABLE", "value": "THIS IS NOT AQL", "formalism": "aql" }]),
    );
    let err = svc
        .export_ehr_extracts(spec)
        .await
        .expect_err("an unparseable AQL criterion must be refused");
    assert_eq!(
        err.status,
        ferroehr::service::status::CallStatusType::PreconditionViolation
    );
    assert!(
        err.message.contains("criteria[0]"),
        "the refusal names the criterion: {}",
        err.message
    );
}
