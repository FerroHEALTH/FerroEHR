// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An operational template built over a CKM archetype names that archetype as
//! its `parent_archetype_id` (#5148).
//!
//! AOM2 §3.6.6 gives `OPERATIONAL_TEMPLATE` the invariant `is_specialised`,
//! and OPT2 `master02` resolves every archetype reference to its full id, so
//! the OPT carries the archetype its root template specialises without a
//! consumer filling it in. Both CKM forms are covered: the ADL 1.4 pack, read
//! unconverted as a consumer reads it, and the ADL 2 pack. In each, the
//! template names its parent by the interface id and the OPT by the full one.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::operational_template::OperationalTemplate;

/// The CKM ADL 1.4 `report` COMPOSITION, the parent of `report-result`.
const REPORT_14: &str =
    include_str!("../../../../corpus/archetypes/ckm/adl14/openEHR-EHR-COMPOSITION.report.v1.adl");

/// The CKM ADL 1.4 `report-result` COMPOSITION.
const REPORT_RESULT_14: &str = include_str!(
    "../../../../corpus/archetypes/ckm/adl14/openEHR-EHR-COMPOSITION.report-result.v1.adl"
);

/// The CKM ADL 2 `prescription` COMPOSITION (`openEHR-EHR-COMPOSITION.prescription.v1.0.0`).
const PRESCRIPTION_2: &str = include_str!(
    "../../../../corpus/archetypes/adl2/ckm-2013-12-09/composition/openEHR-EHR-COMPOSITION.prescription.v1.0.0.adls"
);

/// An ADL 2 template that specialises `parent` and constrains nothing of its
/// own, its root renamed by the code `root_code`.
fn template(id: &str, parent: &str, root_code: &str) -> String {
    format!(
        "\
template (adl_version=2.4.0; rm_release=1.1.0)
    {id}

specialize
    {parent}

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"unmanaged\">
    original_author = <
        [\"name\"] = <\"FerroEHR\">
    >

definition
    COMPOSITION[{root_code}]

terminology
    term_definitions = <
        [\"en\"] = <
            [\"{root_code}\"] = <
                text = <\"Templated composition\">
                description = <\"A template over a CKM composition\">
            >
        >
    >
"
    )
}

fn build(template_src: &str, sources: &[(&str, Dialect)]) -> OperationalTemplate {
    let mut repo = ArchetypeRepository::default();
    for (source, dialect) in sources {
        repo.insert(parse_artefact(source, *dialect).expect("the CKM source parses"));
    }
    let root = parse_artefact(template_src, Dialect::Adl2).expect("the template parses");
    create_opt(&root, &repo).unwrap_or_else(|e| panic!("create_opt failed: {e}"))
}

#[test]
fn an_opt_over_a_ckm_adl14_archetype_names_it_by_its_full_id() {
    let opt = build(
        &template(
            "openEHR-EHR-COMPOSITION.t_report_result.v1.0.0",
            "openEHR-EHR-COMPOSITION.report-result.v1",
            "at0000.1.1",
        ),
        &[
            (REPORT_14, Dialect::Adl14),
            (REPORT_RESULT_14, Dialect::Adl14),
        ],
    );
    assert_eq!(
        opt.parent_archetype_id.as_deref(),
        Some("openEHR-EHR-COMPOSITION.report-result.v1.0.0"),
        "the OPT names the CKM archetype its template specialises, by its full id"
    );
}

#[test]
fn an_opt_over_a_ckm_adl2_archetype_names_it_by_its_full_id() {
    let opt = build(
        &template(
            "openEHR-EHR-COMPOSITION.t_prescription.v1.0.0",
            "openEHR-EHR-COMPOSITION.prescription.v1",
            "id1.1",
        ),
        &[(PRESCRIPTION_2, Dialect::Adl2)],
    );
    assert_eq!(
        opt.parent_archetype_id.as_deref(),
        Some("openEHR-EHR-COMPOSITION.prescription.v1.0.0"),
        "the OPT names the CKM archetype its template specialises, by its full id"
    );
}
