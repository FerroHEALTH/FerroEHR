// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The OPT 1.4 writer's output against the template upload's artefact
//! validation: an operational template the writer builds from CKM archetypes
//! is one FerroEHR accepts, unless the archetype itself breaks a rule.

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers; panicking assertions are the intended shape here \
              (the Rust Book ch11)"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use openehr_base::v1_3::base_types::identification::archetype_id::ArchetypeId;
use openehr_base::v1_3::base_types::identification::template_id::TemplateId;
use openehr_its::opt14::types::{Archetype, CAttribute, CObject, Intervalofinteger};
use openehr_its::opt14_writer::{
    ArchetypeLibrary, ArchetypeUse, Narrowing, NodeClone, SlotFill, TemplateDefinition,
};

use ferroehr::service::error::ServiceError;
use ferroehr::validation::validate_opt_artefact;

/// CKM archetypes whose own `existence` on a reference-model attribute is
/// wider than the RM's (AOM2 VCAEX), which the writer copies as it stands.
/// Each entry is a negative test, so a corrected CKM export shows up here.
const VCAEX_ARCHETYPES: &[&str] = &[
    "openEHR-DEMOGRAPHIC-ADDRESS.address-provider.v0",
    "openEHR-DEMOGRAPHIC-ADDRESS.address.v0",
    "openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication-provider.v0",
    "openEHR-DEMOGRAPHIC-ADDRESS.electronic_communication.v0",
    "openEHR-DEMOGRAPHIC-CAPABILITY.individual_credentials.v0",
    "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v0",
    "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.organisation_name.v0",
    "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name-individual_provider.v0",
    "openEHR-DEMOGRAPHIC-PARTY_IDENTITY.person_name.v0",
    "openEHR-DEMOGRAPHIC-PERSON.person-patient.v0",
    "openEHR-DEMOGRAPHIC-PERSON.person.v0",
    "openEHR-DEMOGRAPHIC-ROLE.healthcare_consumer.v0",
    "openEHR-DEMOGRAPHIC-ROLE.healthcare_provider_organisation.v0",
    "openEHR-DEMOGRAPHIC-ROLE.individual_provider.v0",
    "openEHR-DEMOGRAPHIC-ROLE.third_party_payer.v0",
];

fn pack() -> Vec<PathBuf> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/archetypes/ckm/xml");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("read {}: {e}", dir.display()))
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "xml"))
        .collect();
    files.sort();
    files
}

/// Every CKM archetype the XML codec reads; its refusals are adjudicated by
/// `openehr-its`'s own CKM archetype gate.
fn library() -> ArchetypeLibrary {
    let library: ArchetypeLibrary = pack()
        .iter()
        .filter_map(|p| {
            let xml = std::fs::read_to_string(p).expect("read archetype XML");
            openehr_its::xml::runtime::from_xml::<Archetype>(&xml).ok()
        })
        .collect();
    assert!(library.len() >= 900, "the CKM archetype pack is missing");
    library
}

fn definition(
    template_id: &str,
    root: ArchetypeUse,
    library: &ArchetypeLibrary,
) -> TemplateDefinition {
    let language = library
        .get(&root.archetype_id.value)
        .unwrap_or_else(|| panic!("{} is in the CKM pack", root.archetype_id.value))
        .original_language
        .clone();
    TemplateDefinition {
        template_id: TemplateId {
            value: template_id.to_owned(),
        },
        concept: template_id.to_owned(),
        language,
        description: None,
        uid: None,
        root,
    }
}

fn archetype_use(id: &str) -> ArchetypeUse {
    ArchetypeUse::new(ArchetypeId {
        value: id.to_owned(),
    })
}

#[test]
fn every_ckm_archetype_opt_passes_the_upload_validation() {
    let library = library();
    let mut findings = String::new();
    let mut refused = Vec::new();
    for path in pack() {
        let Some(id) = path.file_stem().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        if library.get(id).is_none() {
            continue;
        }
        let built = openehr_its::opt14_writer::build(
            &definition(id, archetype_use(id), &library),
            &library,
        )
        .unwrap_or_else(|e| panic!("{id}: the writer refuses a one-archetype template: {e}"));
        match validate_opt_artefact(&built) {
            Ok(()) => {}
            Err(ServiceError::ValidationFailed(violations))
                if violations.iter().all(|v| v.path == "VCAEX") =>
            {
                refused.push(id.to_owned());
            }
            Err(e) => {
                let _ = writeln!(findings, "{id}: {e:?}");
            }
        }
    }
    assert!(
        findings.is_empty(),
        "writer OPTs the upload refuses:\n{findings}"
    );
    assert_eq!(
        refused, VCAEX_ARCHETYPES,
        "the archetypes refused for their own existence constraints"
    );
}

/// The shape of an imaging report: a result report composition, which leaves
/// `content` open, holding an imaging examination result with its series.
#[test]
fn an_imaging_report_template_passes_the_upload_validation() {
    let library = library();
    let mut result = archetype_use("openEHR-EHR-OBSERVATION.imaging_exam_result.v1");
    result.fills.push(SlotFill {
        path: "/protocol[at0025]/items[at0065]".to_owned(),
        occurrences: None,
        filler: archetype_use("openEHR-EHR-CLUSTER.imaging_series.v0"),
    });
    let mut root = archetype_use("openEHR-EHR-COMPOSITION.report-result.v1");
    root.fills.push(SlotFill {
        path: "/content".to_owned(),
        occurrences: Some(Intervalofinteger {
            lower_included: Some(true),
            upper_included: None,
            lower_unbounded: false,
            upper_unbounded: true,
            lower: Some(1),
            upper: None,
        }),
        filler: result,
    });
    let built =
        openehr_its::opt14_writer::build(&definition("Imaging report", root, &library), &library)
            .expect("the imaging report builds");
    validate_opt_artefact(&built).expect("the upload accepts the imaging report");
}

const CLOSED_ONE: &str = "<lower_included>true</lower_included><upper_included>true</upper_included>\
     <lower_unbounded>false</lower_unbounded><upper_unbounded>false</upper_unbounded>\
     <lower>1</lower><upper>1</upper>";

/// The `name` constraint a template gives a node: a `DV_TEXT` whose value is
/// `text`.
fn name_constraint(text: &str) -> CAttribute {
    openehr_its::xml::runtime::from_xml(&format!(
        "<attributes xmlns=\"http://schemas.openehr.org/v1\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"C_SINGLE_ATTRIBUTE\">\
         <rm_attribute_name>name</rm_attribute_name><existence>{CLOSED_ONE}</existence>\
         <children xsi:type=\"C_COMPLEX_OBJECT\"><rm_type_name>DV_TEXT</rm_type_name>\
         <occurrences>{CLOSED_ONE}</occurrences><node_id/>\
         <attributes xsi:type=\"C_SINGLE_ATTRIBUTE\"><rm_attribute_name>value</rm_attribute_name>\
         <existence>{CLOSED_ONE}</existence><children xsi:type=\"C_PRIMITIVE_OBJECT\">\
         <rm_type_name>STRING</rm_type_name><occurrences>{CLOSED_ONE}</occurrences><node_id/>\
         <item xsi:type=\"C_STRING\"><list>{text}</list></item></children></attributes>\
         </children></attributes>"
    ))
    .expect("a well-formed attribute")
}

fn complex(rm_type: &str) -> CObject {
    openehr_its::xml::runtime::from_xml(&format!(
        "<children xmlns=\"http://schemas.openehr.org/v1\" \
         xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\" xsi:type=\"C_COMPLEX_OBJECT\">\
         <rm_type_name>{rm_type}</rm_type_name><occurrences>{CLOSED_ONE}</occurrences>\
         <node_id/></children>"
    ))
    .expect("a well-formed object")
}

/// A template that renames and clones a repeatable node and narrows value
/// types: the address cluster with two named streets.
#[test]
fn a_template_with_clones_and_redefinitions_passes_the_upload_validation() {
    let library = library();
    let mut root = archetype_use("openEHR-EHR-CLUSTER.address.v1");
    root.narrowings.push(Narrowing::Constrain {
        path: "/items[at0001]".to_owned(),
        attribute: name_constraint("Street"),
    });
    root.narrowings.push(Narrowing::Clone(NodeClone::new(
        "/items[at0001]",
        name_constraint("Second street"),
    )));
    root.narrowings.push(Narrowing::Redefine {
        path: "/items[at0001]/value".to_owned(),
        objects: vec![complex("DV_CODED_TEXT")],
    });
    root.narrowings.push(Narrowing::Redefine {
        path: "/items[at0010]/value".to_owned(),
        objects: vec![complex("DV_TEXT")],
    });
    let built =
        openehr_its::opt14_writer::build(&definition("Two streets", root, &library), &library)
            .expect("the template builds");
    validate_opt_artefact(&built).expect("the upload accepts clones and redefinitions");
}
