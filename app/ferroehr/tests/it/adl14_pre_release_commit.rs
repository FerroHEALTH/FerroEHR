// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A composition recorded against ADL 1.4 archetypes commits under an ADL 2
//! template over their conversions, over a real PostgreSQL 19 (shared
//! testkit harness).
//!
//! Each CKM ADL 1.4 archetype is uploaded, converted through the service to
//! at-coded ADL 2.4 and stored; the template references the two pre-release
//! archetypes by their full identifiers (BASE `master07-referencing.adoc`
//! §Referencing), one of them as an overlay parent. A composition carrying the
//! 1.4 at-codes validates, since at-coded ADL 2.4 "is fully backwards
//! compatible with openEHR RM data created based on ADL 1.4 archetypes"
//! (`ADL2/master01-preface.adoc` §Backward Compatibility), and one breaking a
//! constraint is refused (ITS-REST `responses/422.yaml`).

use std::path::Path;

use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use serde_json::{Value, json};

/// The CKM ADL 1.4 archetypes the template compiles against.
const CONSTITUENTS: &[&str] = &[
    "openEHR-EHR-COMPOSITION.health_summary.v1.adl",
    "openEHR-EHR-SECTION.immunisation_list.v0.adl",
    "openEHR-EHR-SECTION.adhoc.v1.adl",
    "openEHR-EHR-EVALUATION.exclusion_global.v1.adl",
    "openEHR-EHR-EVALUATION.device_summary.v0.adl",
];

/// The at-coded ADL 2.4 template over the pre-release archetypes.
const TEMPLATE: &str =
    include_str!("../../../../crates/openehr-adl/tests/fixtures/pre_release_summary.adlt");

fn ckm(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/archetypes/ckm/adl14")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn text(value: &str) -> Value {
    json!({"_type": "DV_TEXT", "value": value})
}

fn coded(value: &str, terminology: &str, code: &str) -> Value {
    json!({
        "_type": "DV_CODED_TEXT",
        "value": value,
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
            "code_string": code
        }
    })
}

fn code_phrase(terminology: &str, code: &str) -> Value {
    json!({
        "_type": "CODE_PHRASE",
        "terminology_id": {"_type": "TERMINOLOGY_ID", "value": terminology},
        "code_string": code
    })
}

fn archetyped(id: &str) -> Value {
    json!({
        "_type": "ARCHETYPED",
        "archetype_id": {"_type": "ARCHETYPE_ID", "value": id},
        "rm_version": "1.1.0"
    })
}

fn element(node: &str, name: &Value, value: &Value) -> Value {
    json!({"_type": "ELEMENT", "name": name, "archetype_node_id": node, "value": value})
}

/// An EVALUATION of the legacy (ADL 1.4) archetype `id` holding `items`.
fn evaluation(id: &str, name: &str, items: &Value) -> Value {
    json!({
        "_type": "EVALUATION",
        "name": text(name),
        "archetype_node_id": id,
        "archetype_details": archetyped(id),
        "language": code_phrase("ISO_639-1", "en"),
        "encoding": code_phrase("IANA_character-sets", "UTF-8"),
        "subject": {"_type": "PARTY_SELF"},
        "data": {
            "_type": "ITEM_TREE",
            "name": text("Tree"),
            "archetype_node_id": "at0001",
            "items": items
        }
    })
}

/// The composition, recorded with the ADL 1.4 identifiers and at-codes; its
/// device status is coded `status`.
fn composition(status: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "name": text("Pre-release summary"),
        "archetype_node_id": "openEHR-EHR-COMPOSITION.health_summary.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": {
                "_type": "ARCHETYPE_ID",
                "value": "openEHR-EHR-COMPOSITION.health_summary.v1"
            },
            "template_id": {
                "_type": "TEMPLATE_ID",
                "value": "openEHR-EHR-COMPOSITION.t_pre_release_summary.v1.0.0"
            },
            "rm_version": "1.1.0"
        },
        "language": code_phrase("ISO_639-1", "en"),
        "territory": code_phrase("ISO_3166-1", "NL"),
        "category": coded("event", "openehr", "433"),
        "composer": {"_type": "PARTY_IDENTIFIED", "name": "Synthetic composer"},
        "context": {
            "_type": "EVENT_CONTEXT",
            "start_time": {"_type": "DV_DATE_TIME", "value": "2026-10-01T08:00:00Z"},
            "setting": coded("other care", "openehr", "238")
        },
        "content": [
            {
                "_type": "SECTION",
                "name": text("Vaccination list"),
                "archetype_node_id": "openEHR-EHR-SECTION.immunisation_list.v0",
                "archetype_details": archetyped("openEHR-EHR-SECTION.immunisation_list.v0"),
                "items": [evaluation(
                    "openEHR-EHR-EVALUATION.exclusion_global.v1",
                    "Global exclusion",
                    &json!([element(
                        "at0002",
                        &coded("No known medications", "local", "at0005"),
                        &text("No vaccinations recorded")
                    )])
                )]
            },
            {
                "_type": "SECTION",
                "name": text("Medical devices"),
                "archetype_node_id": "openEHR-EHR-SECTION.adhoc.v1",
                "archetype_details": archetyped("openEHR-EHR-SECTION.adhoc.v1"),
                "items": [evaluation(
                    "openEHR-EHR-EVALUATION.device_summary.v0",
                    "Medical device summary",
                    &json!([
                        element(
                            "at0020",
                            &text("Device type"),
                            &coded("Cardiac pacemaker", "http://snomed.info/sct", "14106009")
                        ),
                        element("at0002", &text("Status"), &coded("Current", "local", status)),
                        {
                            "_type": "CLUSTER",
                            "name": text("Device details"),
                            "archetype_node_id": "at0022",
                            "items": [element(
                                "at0007",
                                &text("Device name"),
                                &text("Cardiac pacemaker")
                            )]
                        }
                    ])
                )]
            }
        ]
    })
}

/// Uploads every constituent, stores its at-coded conversion and uploads the
/// template.
async fn provision(svc: &FerroEhrService) {
    for name in CONSTITUENTS {
        let id = svc
            .upload_archetype(ckm(name))
            .await
            .unwrap_or_else(|e| panic!("upload {name}: {e:?}"));
        let conversion = svc
            .adl14_convert_to_adl2(id.clone())
            .await
            .unwrap_or_else(|e| panic!("convert {id}: {e:?}"));
        let root = conversion.roots.first().expect("one converted root");
        svc.template_adl2_upload(&root.adl2)
            .await
            .unwrap_or_else(|e| panic!("store the conversion of {id}: {e:?}"));
    }
    svc.template_adl2_upload(TEMPLATE)
        .await
        .expect("the template over the full pre-release ids compiles");
}

#[tokio::test]
async fn a_14_archetype_converts_to_at_coded_adl_2_4_under_its_full_identifier() {
    let (_db, svc) = crate::fixtures::service().await;
    let id = svc
        .upload_archetype(ckm("openEHR-EHR-EVALUATION.device_summary.v0.adl"))
        .await
        .expect("the 1.4 archetype uploads");
    let conversion = svc.adl14_convert_to_adl2(id).await.expect("it converts");
    let adl2 = &conversion.roots.first().expect("one converted root").adl2;
    assert!(adl2.contains("openEHR-EHR-EVALUATION.device_summary.v0.0.1-alpha"));
    assert!(adl2.contains("adl_version=2.4.0"));
    assert!(
        adl2.contains("EVALUATION[at0000]"),
        "the 1.4 root code stays"
    );
    assert!(
        adl2.contains("ITEM_TREE[at0001]"),
        "the 1.4 node codes stay"
    );
}

#[tokio::test]
async fn a_composition_with_14_at_codes_commits_under_a_template_over_their_conversions() {
    let (_db, svc) = crate::fixtures::service().await;
    provision(&svc).await;
    let ehr: EhrId = crate::fixtures::create_ehr(&svc).await;
    svc.create_composition(
        ehr,
        crate::fixtures::uv(&composition("at0004"), "249", None),
    )
    .await
    .unwrap_or_else(|e| panic!("the 1.4-coded composition commits: {e:?}"));

    match svc
        .create_composition(
            ehr,
            crate::fixtures::uv(&composition("at0099"), "249", None),
        )
        .await
    {
        Err(ServiceError::ValidationFailed(violations)) => assert!(
            violations
                .iter()
                .any(|v| v.message.contains("at0099") && v.path.contains("device_summary")),
            "the code outside the value set is named: {violations:?}"
        ),
        other => panic!("a code outside the 1.4 value set is refused, got {other:?}"),
    }
}
