// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Annex I priority categories the FHIR read façade claims to serve,
//! proven end to end over the committed CKM corpus: the category's real OPT
//! builds a Web Template, its committed example COMPOSITION flattens against
//! that template to a non-empty FLAT map (`simplified_formats` master04
//! §Field Identifiers), and a mapping entry over a leaf TAKEN FROM THAT MAP
//! drives the reverse transform to a FHIR resource carrying the
//! composition's own value.
//!
//! No openEHR spec governs FHIR conversion — our own design/extension; the
//! FHIR side of each assertion is R4 `Observation`
//! (<https://hl7.org/fhir/R4/observation.html>).
//!
//! The mapped leaf is DERIVED, never written down here: a corpus refresh that
//! changed the templates would move the leaf and still be proven, and a
//! template whose flat map offers no such leaf fails the case instead of
//! skipping it.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers; panicking assertions are the intended shape here \
              (the Rust Book ch11)"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use ferroehr_ext::fhir::mapping::{FhirMappingDefinition, MappingEntry, SubjectMapping, Transform};
use ferroehr_ext::fhir::reverse::to_fhir;
use openehr_its::opt14;
use openehr_sdt::flat::convert::composition_to_flat;
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use serde_json::{Map, Value};

/// The FHIR element every case maps its derived leaf onto: R4
/// `Observation.note.text` (`Annotation.text`), a free-text target that
/// accepts any leaf the derivation picks.
const FHIR_TARGET: &str = "note[0].text";

/// The EHR subject the façade would supply for the reverse transform.
const SUBJECT_ID: &str = "ehds-subject-1";

fn corpus_file(slug: &str, extension: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../corpus/templates/ckm/{slug}.{extension}"))
}

/// The first plain-string leaf of the ascending key order, excluding the
/// `ctx/` composition-context keys (`simplified_formats` master04 §Context)
/// and every `|suffix` sub-leaf: a stable choice across runs, derived from
/// the flat map rather than written per template. `None` = the template
/// offers no such leaf, which fails its case rather than skipping it.
fn derive_mapped_leaf(flat: &Map<String, Value>) -> Option<(String, String)> {
    let mut keys: Vec<&String> = flat.keys().collect();
    keys.sort();
    keys.into_iter()
        .filter(|key| !key.starts_with("ctx/") && !key.contains('|'))
        .find_map(|key| {
            flat.get(key)
                .and_then(Value::as_str)
                .map(|text| (key.clone(), text.to_owned()))
        })
}

/// One mapping entry binding the derived leaf to [`FHIR_TARGET`].
fn definition_over(leaf: &str, template_id: &str) -> FhirMappingDefinition {
    FhirMappingDefinition {
        resource_type: "Observation".to_owned(),
        profile_url: None,
        template_id: template_id.to_owned(),
        subject: SubjectMapping {
            reference_path: "subject.reference".to_owned(),
            namespace: "fhir".to_owned(),
            strip_prefix: Some("Patient/".to_owned()),
        },
        context: Map::new(),
        entries: vec![MappingEntry {
            openehr_path: leaf.to_owned(),
            fhir_path: Some(FHIR_TARGET.to_owned()),
            constant: None,
            transform: Transform::Text,
            code_map: BTreeMap::new(),
            required: true,
        }],
    }
}

/// Runs one category end to end and returns the leaf it mapped with the value
/// it asserted, so the caller's test name states what was proven.
fn round_trip_category(slug: &str, min_leaves: usize) -> (String, String) {
    let xml = std::fs::read_to_string(corpus_file(slug, "opt")).expect("read the committed OPT");
    let opt = opt14::from_xml(&xml).expect("the committed OPT parses");
    let wt = build_web_template(&opt).expect("the OPT builds a Web Template");

    let example = std::fs::read_to_string(corpus_file(slug, "example.json"))
        .expect("read the committed example composition");
    let composition: Value = serde_json::from_str(&example).expect("the example is canonical JSON");

    let flat = composition_to_flat(&composition, &wt).expect("the example flattens");
    assert!(
        flat.len() >= min_leaves,
        "{slug}: {} flat leaves is below the committed corpus floor {min_leaves} — an \
         empty or gutted flat map is not a round trip",
        flat.len()
    );

    let derived = derive_mapped_leaf(&flat);
    assert!(
        derived.is_some(),
        "{slug}: the flat map carries no plain-string data leaf to map"
    );
    let (leaf, value) = derived.expect("the assertion above rejected the absent case");
    let definition = definition_over(&leaf, slug);
    let resource = to_fhir(
        "Observation",
        &composition,
        &wt,
        &definition,
        Some(SUBJECT_ID),
    )
    .expect("the reverse transform maps the composition");

    assert_eq!(
        resource.pointer("/note/0/text"),
        Some(&Value::String(value.clone())),
        "{slug}: the FHIR resource must carry the composition's own value for {leaf}"
    );
    assert_eq!(
        resource.pointer("/subject/reference"),
        Some(&Value::String(format!("Patient/{SUBJECT_ID}"))),
        "{slug}: the subject reference is reconstructed with strip_prefix re-applied"
    );
    (leaf, value)
}

#[test]
fn patient_summaries_round_trip_through_the_fhir_read_facade() {
    let (leaf, value) = round_trip_category("international-patient-summary", 392);
    assert!(leaf.starts_with("international_patient_summary/"), "{leaf}");
    assert!(!value.is_empty(), "{leaf} mapped an empty value");
}

#[test]
fn electronic_prescriptions_round_trip_through_the_fhir_read_facade() {
    let (leaf, value) = round_trip_category("eprescription-fhir", 82);
    assert!(leaf.starts_with("prescription/"), "{leaf}");
    assert!(!value.is_empty(), "{leaf} mapped an empty value");
}

#[test]
fn imaging_reports_round_trip_through_the_fhir_read_facade() {
    let (leaf, value) = round_trip_category("ccta-report", 801);
    assert!(leaf.starts_with("result_report/"), "{leaf}");
    assert!(!value.is_empty(), "{leaf} mapped an empty value");
}

#[test]
fn laboratory_results_round_trip_through_the_fhir_read_facade() {
    let (leaf, value) = round_trip_category("generic-lab-test-result", 14);
    assert!(
        leaf.starts_with("generic_lab_test_result_example/"),
        "{leaf}"
    );
    assert!(!value.is_empty(), "{leaf} mapped an empty value");
}
