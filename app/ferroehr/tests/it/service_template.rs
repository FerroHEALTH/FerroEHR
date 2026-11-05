// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end service tests for OPT 1.4 operational-template ingestion against a
//! real `PostgreSQL` 18 (shared testkit harness): upload a corpus `.opt` template, list it,
//! retrieve its XML, and re-upload (idempotent replace) — driven through the
//! `FerroEhrService` definition methods the REST layer calls.

use serde_json::Value;

use ferroehr::error::ServiceError;
use ferroehr::error::refusal::{Clash, Invalid, Malformed, Missing, Refusal};
use ferroehr::service::definition::types::TemplateListFilter;
use ferroehr::service::list::Page;

/// Fixed `ctx/time` default for the FLAT rebuild directions (ITS-REST
/// `simplified_formats` master04 §Context) so round-trips stay deterministic.
const NOW: &str = "2024-01-01T00:00:00Z";

/// A representative corpus template (Ocean Template Designer OPT 1.4 XML).
const TEMPLATE_REL: &str = "knowledge/IDCR Allergies List.v0.opt";
const TEMPLATE_ID: &str = "IDCR Allergies List.v0";

fn corpus_opt(rel: &str) -> String {
    let path = format!(
        "{}/../../corpus/fixtures/service/{rel}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
}

#[tokio::test]
async fn template_upload_list_get_roundtrip() {
    let (_db, svc) = crate::fixtures::service().await;
    let xml = corpus_opt(TEMPLATE_REL);

    // Upload the OPT XML (arrives as a JSON string, as the lenient body reader hands it over).
    let desc = svc.template_adl14_upload(&xml).await.expect("upload");
    assert_eq!(desc["template_id"], TEMPLATE_ID, "descriptor template_id");
    assert!(
        desc["archetype_id"]
            .as_str()
            .is_some_and(|a| a.contains("COMPOSITION")),
        "root archetype extracted: {desc}"
    );

    // List includes the uploaded template.
    let list = svc
        .template_adl14_list(TemplateListFilter::default(), Page::default())
        .await
        .expect("list");
    assert!(
        list.iter().any(|t| t["template_id"] == TEMPLATE_ID),
        "list contains the template: {list:?}"
    );

    // Retrieve returns the stored OPT XML verbatim.
    let got = svc.template_adl14_get(TEMPLATE_ID).await.expect("get");
    assert_eq!(
        got, xml,
        "retrieved OPT XML is byte-identical to the upload"
    );

    // Re-uploading an existing template_id is a 409 Conflict, not a silent
    // overwrite: OPTs are immutable on the adl1.4 endpoint (ITS-REST
    // `409_template_already_exists.yaml`; CNF `upload_opt-valid_opt_twice_conflict`).
    let conflict = svc
        .template_adl14_upload(&xml)
        .await
        .expect_err("re-upload of an existing template_id must conflict");
    assert!(
        matches!(
            conflict,
            ServiceError::Conflict(Refusal {
                kind: Clash::State,
                ..
            })
        ),
        "got {conflict:?}"
    );

    // The original template is untouched and there is still exactly one row.
    let list2 = svc
        .template_adl14_list(TemplateListFilter::default(), Page::default())
        .await
        .expect("list after conflicting re-upload");
    assert_eq!(
        list2
            .iter()
            .filter(|t| t["template_id"] == TEMPLATE_ID)
            .count(),
        1,
        "conflicting re-upload did not duplicate"
    );
    let still = svc
        .template_adl14_get(TEMPLATE_ID)
        .await
        .expect("get after conflict");
    assert_eq!(
        still, xml,
        "conflicting re-upload did not overwrite the stored OPT"
    );
}

#[tokio::test]
async fn get_unknown_template_is_not_found() {
    let (_db, svc) = crate::fixtures::service().await;
    // An unknown template_id → 404 (`template_does_not_exist`).
    let err = svc
        .template_adl14_get("unknown template.v0")
        .await
        .expect_err("expected not-found");
    assert!(
        matches!(
            err,
            ServiceError::NotFound(Refusal {
                kind: Missing::Template,
                ..
            })
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn invalid_opt_xml_is_rejected() {
    let (_db, svc) = crate::fixtures::service().await;
    let err = svc
        .template_adl14_upload("<not-a-template/>")
        .await
        .expect_err("expected rejection of a non-OPT body");
    assert!(
        matches!(
            err,
            ServiceError::Unprocessable {
                kind: Invalid::Template,
                ..
            }
        ),
        "got {err:?}"
    );
}

// ── example generation (`adl1.4/{id}/example`) ───────────────────────────────

/// Varied real templates: an OBSERVATION with a history of events, an
/// EVALUATION-list, and one carrying an ACTION/INSTRUCTION structure.
const EXAMPLE_TEMPLATES: &[&str] = &[
    "knowledge/Vital Signs Encounter (Composition).opt",
    "knowledge/IDCR Allergies List.v0.opt",
    "knowledge/IDCR - Immunisation summary.v0.opt",
];

/// The (cached) `WebTemplate` built from an OPT file, as the service builds it.
fn web_template_of(rel: &str) -> openehr_sdt::flat::webtemplate::model::WebTemplate {
    let opt = openehr_its::opt14::from_xml(&corpus_opt(rel)).expect("parse OPT");
    openehr_sdt::flat::webtemplate::builder::build_web_template(&opt).expect("build web template")
}

/// The generated `required` example is committable (passes the validator)
/// and survives FLAT round-trip + canonical-XML serialization for real
/// templates. The example is fetched through the service definition methods
/// exactly as the REST layer calls it; validation/conversion use the same
/// `WebTemplate` the service caches.
#[tokio::test]
async fn required_example_validates_and_converts() {
    let (_db, svc) = crate::fixtures::service().await;

    for (i, rel) in EXAMPLE_TEMPLATES.iter().enumerate() {
        let xml = corpus_opt(rel);
        let desc = svc
            .template_adl14_upload(&xml)
            .await
            .unwrap_or_else(|e| panic!("upload {rel}: {e:?}"));
        let template_id = desc["template_id"]
            .as_str()
            .unwrap_or_else(|| panic!("template_id for {rel}"))
            .to_owned();
        let wt = web_template_of(rel);

        // `required` example, via the generated trait (as the REST layer calls it).
        let comp = svc
            .template_adl14_example(template_id.clone(), Some("required".to_owned()), None)
            .await
            .unwrap_or_else(|e| panic!("example for {rel}: {e:?}"));
        assert_eq!(
            comp.get("_type").and_then(Value::as_str),
            Some("COMPOSITION"),
            "{rel}: example is a COMPOSITION"
        );

        // Acceptance bar: the required example is committable — it passes the
        // full validator (RM invariants + terminology + archetype
        // conformance) with no violations.
        let violations = openehr_sdt::rm_instance::validate_composition(&comp, &wt);
        assert!(
            violations.is_empty(),
            "{rel}: required example must validate clean, got {} violation(s): {violations:?}",
            violations.len()
        );

        // FLAT round-trip is stable (canonical → FLAT → canonical → FLAT). The
        // rebuild's ctx/time default is a fixed instant (ITS-REST
        // simplified_formats master04 §Context) so the round-trip is deterministic.
        let flat1 = openehr_sdt::flat::convert::composition_to_flat(&comp, &wt)
            .unwrap_or_else(|e| panic!("{rel} to_flat: {e}"));
        let flat1_map: serde_json::Map<String, Value> = flat1.clone().into_iter().collect();
        let comp2 = openehr_sdt::flat::convert::composition_from_flat(&flat1_map, &wt, NOW)
            .unwrap_or_else(|e| panic!("{rel} from_flat: {e}"));
        let flat2 = openehr_sdt::flat::convert::composition_to_flat(&comp2, &wt)
            .unwrap_or_else(|e| panic!("{rel} to_flat2: {e}"));
        assert_eq!(flat1, flat2, "{rel}: FLAT round-trip is stable");

        // Canonical-XML serialization succeeds (deserialises as an RM COMPOSITION
        // then emits canonical XML — the XML `Accept` path in the dispatcher).
        let typed: openehr_rm::prelude::Composition =
            openehr_its::json::from_canonical_value(&comp)
                .unwrap_or_else(|e| panic!("{rel}: example deserialises as Composition: {e}"));
        let xml_out = openehr_its::xml::runtime::to_xml(
            &typed,
            "composition",
            openehr_its::xml::runtime::Namespace::V2,
        )
        .unwrap_or_else(|e| panic!("{rel}: canonical XML: {e}"));
        assert!(
            xml_out.contains("<composition"),
            "{rel}: XML has a composition root"
        );

        // The `output` form carries a deterministic uid; `input` does not.
        let output = svc
            .template_adl14_example(template_id.clone(), None, Some("output".to_owned()))
            .await
            .unwrap_or_else(|e| panic!("output example for {rel}: {e:?}"));
        assert!(
            output.pointer("/uid/value").is_some(),
            "{rel}: output example carries a uid"
        );
        assert!(comp.get("uid").is_none(), "{rel}: input example has no uid");

        // Distinct databases per iteration are unnecessary; template ids differ.
        let _ = i;
    }
}

#[tokio::test]
async fn example_for_unknown_template_is_not_found() {
    let (_db, svc) = crate::fixtures::service().await;
    let err = svc
        .template_adl14_example("does.not.exist.v0".to_owned(), None, None)
        .await
        .expect_err("expected not-found for an unknown template");
    assert!(
        matches!(
            err,
            ServiceError::NotFound(Refusal {
                kind: Missing::Template,
                ..
            })
        ),
        "got {err:?}"
    );
}

#[tokio::test]
async fn example_with_invalid_detail_level_is_bad_request() {
    let (_db, svc) = crate::fixtures::service().await;
    // Upload a template so the failure is the detail_level, not a missing id.
    let xml = corpus_opt(TEMPLATE_REL);
    svc.template_adl14_upload(&xml).await.expect("upload");
    let err = svc
        .template_adl14_example(TEMPLATE_ID.to_owned(), Some("exhaustive".to_owned()), None)
        .await
        .expect_err("expected bad-request for an invalid detail_level");
    assert!(
        matches!(
            err,
            ServiceError::BadRequest(Refusal {
                kind: Malformed::Precondition,
                ..
            })
        ),
        "got {err:?}"
    );
}

/// The OPT-1.4 → ADL2 conversion capability (service-only, no wire): a stored
/// operational template is loaded by `template_id`, decomposed into one
/// 1.4-shaped source per embedded archetype root, and each is converted to ADL2 source text. No
/// openEHR spec governs 1.4 → 2 conversion — our own design/extension.
#[tokio::test]
async fn opt_converts_to_adl2_sources() {
    let (_db, svc) = crate::fixtures::service().await;
    // A minimal template: a COMPOSITION root with one embedded OBSERVATION root.
    let xml = corpus_opt("knowledge/opt/minimal_observation.opt");
    let summary = svc.template_adl14_upload(&xml).await.expect("upload opt");
    let template_id = summary["template_id"]
        .as_str()
        .expect("the upload names its template_id")
        .to_owned();

    // The service loads the stored OPT by template_id, decomposes + converts it, and
    // returns one ADL2 source per embedded archetype root (>= 2 here: the
    // COMPOSITION root and the embedded OBSERVATION).
    let conversion = svc
        .adl14_convert_opt_to_adl2(template_id)
        .await
        .expect("convert stored OPT to ADL2");
    assert!(
        !conversion.structure.is_empty(),
        "the embedded OBSERVATION fills a slot of the COMPOSITION"
    );
    let sources: Vec<&str> = conversion.roots.iter().map(|r| r.adl2.as_str()).collect();
    assert!(
        sources.len() >= 2,
        "expected >= 2 converted sources, got {}",
        sources.len()
    );
    for src in sources {
        assert!(
            src.contains("archetype"),
            "each converted source is ADL2 text: {}",
            src.lines().next().unwrap_or_default()
        );
    }

    // An unknown OPT id is a 404 (`template_does_not_exist`).
    let missing = svc
        .adl14_convert_opt_to_adl2("unknown template.v0".to_owned())
        .await
        .expect_err("unknown OPT id must 404");
    assert!(
        matches!(
            missing,
            ServiceError::NotFound(Refusal {
                kind: Missing::Template,
                ..
            })
        ),
        "got {missing:?}"
    );
}

// ── the example of a party template that marks no node mandatory (#5057) ────

/// An ADL 2 `GROUP` archetype `hrid` with the given `definition` body and term
/// definitions `id1` to `id5`.
fn group_archetype(hrid: &str, definition: &str) -> String {
    let terms = ["Care team", "Identity", "Identity details", "Name", "Value"]
        .iter()
        .enumerate()
        .map(|(i, text)| {
            format!(
                "            [\"id{}\"] = <text = <\"{text}\"> description = <\"{text}.\">>\n",
                i + 1
            )
        })
        .collect::<Vec<_>>()
        .concat();
    format!(
        "archetype (adl_version=2.0.6; rm_release=1.1.0)\n    {hrid}\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\n\
         description\n    original_author = <[\"name\"] = <\"FerroEHR tests\">>\n    \
         lifecycle_state = <\"published\">\n    details = <\n        \
         [\"en\"] = <\n            language = <[ISO_639-1::en]>\n        >\n    >\n\n\
         definition\n    {definition}\n\n\
         terminology\n    term_definitions = <\n        [\"en\"] = <\n{terms}        >\n    >\n"
    )
}

/// The `GROUP` whose only `PARTY_IDENTITY` carries the AOM2 default
/// occurrences, so its Web Template marks `/identities[id2]` optional.
const OPTIONAL_IDENTITY_GROUP: &str = "GROUP[id1] matches { identities matches { \
     PARTY_IDENTITY[id2] matches { details matches { ITEM_TREE[id3] matches { \
     items matches { ELEMENT[id4] occurrences matches {1} matches { \
     value matches { DV_TEXT[id5] } } } } } } } }";

/// The OPT 1.4 twin of [`OPTIONAL_IDENTITY_GROUP`]: `identities` at the RM
/// existence `1..1` with cardinality `0..*`, its `PARTY_IDENTITY` occurring
/// `0..*`.
fn optional_identity_group_opt(template_id: &str) -> String {
    let iv = |lower: u32, upper: Option<u32>| {
        let upper_part = upper.map_or_else(
            || "<upper_unbounded>true</upper_unbounded>".to_owned(),
            |u| format!("<upper_unbounded>false</upper_unbounded><upper>{u}</upper>"),
        );
        format!(
            "<lower_included>true</lower_included><upper_included>{}</upper_included>\
             <lower_unbounded>false</lower_unbounded>{upper_part}<lower>{lower}</lower>",
            upper.is_some()
        )
    };
    let term = |code: &str, text: &str| {
        format!(
            "<term_definitions code=\"{code}\"><items id=\"text\">{text}</items>\
             <items id=\"description\">{text}</items></term_definitions>"
        )
    };
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
         <template xmlns=\"http://schemas.openehr.org/v1\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\
         <language><terminology_id><value>ISO_639-1</value></terminology_id>\
         <code_string>en</code_string></language>\
         <template_id><value>{template_id}</value></template_id><concept>Care team</concept>\
         <definition xsi:type=\"C_ARCHETYPE_ROOT\"><rm_type_name>GROUP</rm_type_name>\
         <occurrences>{one}</occurrences><node_id>at0000</node_id>\
         <attributes xsi:type=\"C_MULTIPLE_ATTRIBUTE\"><rm_attribute_name>identities</rm_attribute_name>\
         <existence>{one}</existence>\
         <children xsi:type=\"C_COMPLEX_OBJECT\"><rm_type_name>PARTY_IDENTITY</rm_type_name>\
         <occurrences>{many}</occurrences><node_id>at0001</node_id>\
         <attributes xsi:type=\"C_SINGLE_ATTRIBUTE\"><rm_attribute_name>details</rm_attribute_name>\
         <existence>{one}</existence>\
         <children xsi:type=\"C_COMPLEX_OBJECT\"><rm_type_name>ITEM_TREE</rm_type_name>\
         <occurrences>{one}</occurrences><node_id>at0002</node_id>\
         <attributes xsi:type=\"C_MULTIPLE_ATTRIBUTE\"><rm_attribute_name>items</rm_attribute_name>\
         <existence>{opt}</existence>\
         <children xsi:type=\"C_COMPLEX_OBJECT\"><rm_type_name>ELEMENT</rm_type_name>\
         <occurrences>{one}</occurrences><node_id>at0003</node_id>\
         <attributes xsi:type=\"C_SINGLE_ATTRIBUTE\"><rm_attribute_name>value</rm_attribute_name>\
         <existence>{one}</existence>\
         <children xsi:type=\"C_COMPLEX_OBJECT\"><rm_type_name>DV_TEXT</rm_type_name>\
         <occurrences>{one}</occurrences><node_id></node_id></children></attributes>\
         </children><cardinality><is_ordered>false</is_ordered><is_unique>false</is_unique>\
         <interval>{many}</interval></cardinality></attributes></children></attributes>\
         </children><cardinality><is_ordered>false</is_ordered><is_unique>false</is_unique>\
         <interval>{many}</interval></cardinality></attributes>\
         <archetype_id><value>openEHR-DEMOGRAPHIC-GROUP.care_team.v1</value></archetype_id>\
         <template_id><value>{template_id}</value></template_id>\
         {t0}{t1}{t2}{t3}</definition></template>",
        one = iv(1, Some(1)),
        opt = iv(0, Some(1)),
        many = iv(0, None),
        t0 = term("at0000", "Care team"),
        t1 = term("at0001", "Identity"),
        t2 = term("at0002", "Identity details"),
        t3 = term("at0003", "Name"),
    )
}

/// What every example of the care-team `GROUP` must be: a `GROUP` that the
/// strict reader decodes, carrying the identity RM demographic §PARTY
/// `Identities_valid` requires (`org.openehr.rm.demographic.party.adoc`).
fn assert_example_group(party: &Value, what: &str) {
    assert_eq!(
        party.get("_type").and_then(Value::as_str),
        Some("GROUP"),
        "{what}: {party}"
    );
    let identity = party
        .pointer("/identities/0")
        .unwrap_or_else(|| panic!("{what}: no identity: {party}"));
    assert_eq!(
        identity.get("_type").and_then(Value::as_str),
        Some("PARTY_IDENTITY"),
        "{what}: {party}"
    );
    openehr_its::json::from_canonical_value::<openehr_rm::v1_2::demographic::group::Group>(party)
        .unwrap_or_else(|e| panic!("{what}: decodes as GROUP: {e}"));
}

/// A `required` example of a party template whose Web Template marks no node
/// mandatory is still a party of its class with one identity, through the
/// ADL 2 and the OPT 1.4 example routes, at every detail level.
#[tokio::test]
async fn a_party_template_with_no_mandatory_node_has_an_example_party() {
    let (_db, svc) = crate::fixtures::service().await;
    let adl2 = svc
        .template_adl2_upload(&group_archetype(
            "openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0",
            OPTIONAL_IDENTITY_GROUP,
        ))
        .await
        .expect("upload the ADL 2 GROUP");
    let opt = svc
        .template_adl14_upload(&optional_identity_group_opt("care_team.optional.v1"))
        .await
        .expect("upload the OPT 1.4 GROUP");
    let opt = opt["template_id"].as_str().expect("template_id").to_owned();
    for level in ["required", "medium", "complete"] {
        let party = svc
            .template_adl2_example(adl2.clone(), Some(level.to_owned()), None)
            .await
            .unwrap_or_else(|e| panic!("ADL 2 {level}: {e:?}"));
        assert_example_group(&party, &format!("ADL 2 {level}"));
        let party = svc
            .template_adl14_example(opt.clone(), Some(level.to_owned()), None)
            .await
            .unwrap_or_else(|e| panic!("OPT 1.4 {level}: {e:?}"));
        assert_example_group(&party, &format!("OPT 1.4 {level}"));
    }
}

/// A party template that constrains no `PARTY_IDENTITY` names no node to make
/// the identity RM demographic §PARTY requires from, so its example is a `400`
/// refusal naming the class.
#[tokio::test]
async fn a_party_template_with_no_identity_has_no_example() {
    let (_db, svc) = crate::fixtures::service().await;
    let id = svc
        .template_adl2_upload(&group_archetype(
            "openEHR-DEMOGRAPHIC-GROUP.open_team.v1.0.0",
            "GROUP[id1] matches { * }",
        ))
        .await
        .expect("upload the open GROUP");
    match svc.template_adl2_example(id, None, None).await {
        Err(e @ ServiceError::BadRequest(_)) => {
            assert!(e.to_string().contains("GROUP"), "{e}");
        }
        other => panic!("expected a 400 refusal, got {other:?}"),
    }
}
