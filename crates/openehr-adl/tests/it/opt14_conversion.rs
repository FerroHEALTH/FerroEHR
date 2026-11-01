// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! OPT 1.4 → ADL 2 conversion through the public entry,
//! `openehr_adl::adl14::opt::convert_opt_to_adl2`, over the vendored OPT corpus
//! under `corpus/fixtures/`.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion; the claim asserted here is
//! self-consistency (convert → print → re-read) and the result's own shape.

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::adl14::opt::convert_opt_to_adl2;
use openehr_adl::parse::Dialect;
use openehr_its::opt14;

/// A minimal COMPOSITION with one embedded OBSERVATION root.
const MINIMAL_OBSERVATION: &str = "service/knowledge/opt/minimal_observation.opt";
/// A minimal EVALUATION, which carries a `C_DV_QUANTITY`.
const MINIMAL_EVALUATION: &str = "service/knowledge/opt/minimal_evaluation.opt";
/// A large template with 12 embedded roots and every domain constrainer.
const VITAL_SIGNS: &str = "service/knowledge/opt/Vital Signs Encounter (Composition).opt";
/// A ROLE template whose defects the conversion reports.
const ROLE: &str = "opt/valid/role.v2.opt";

/// The parsed OPT at `rel` under `corpus/fixtures/`.
fn parse_opt(rel: &str) -> opt14::types::OperationalTemplate {
    let path = format!("{}/../../corpus/fixtures/{rel}", env!("CARGO_MANIFEST_DIR"));
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse OPT {rel}: {e:?}"))
}

/// Every converted root prints to ADL2 text that re-parses — the printed
/// surface (tuples, slot assertions, defaults) is parser-valid, not just
/// object-model-valid.
#[test]
fn printed_adl2_reparses() {
    for rel in [MINIMAL_OBSERVATION, MINIMAL_EVALUATION, VITAL_SIGNS] {
        let opt = parse_opt(rel);
        let conversion = convert_opt_to_adl2(&opt).expect("convert");
        for root in &conversion.roots {
            openehr_adl::assemble::parse_artefact(&root.adl2, Dialect::Adl2).unwrap_or_else(|e| {
                let line = e.first().map_or(0, |err| err.line);
                let context = root
                    .adl2
                    .lines()
                    .enumerate()
                    .skip(line.saturating_sub(4))
                    .take(7)
                    .map(|(i, l)| format!("{:>4} | {l}", i + 1))
                    .collect::<Vec<_>>()
                    .join("\n");
                panic!(
                    "{rel}/{}: printed ADL2 does not re-parse: {e:?}\n{context}",
                    root.archetype_id
                )
            });
        }
    }
}

/// A filled slot carries the canonical include assertion naming the child
/// (`archetype_id/value matches {/…/}` —
/// `org.openehr.am.aom2.archetype_slot.adoc`), alongside the fill edge.
#[test]
fn filled_slots_carry_include_assertions() {
    let opt = parse_opt(MINIMAL_OBSERVATION);
    let conversion = convert_opt_to_adl2(&opt).expect("convert");
    let composition = conversion
        .roots
        .iter()
        .find(|r| r.archetype_id == "openEHR-EHR-COMPOSITION.minimal.v1")
        .expect("composition root");
    assert!(
        composition.adl2.contains("include"),
        "no include assertion in the filled slot:\n{}",
        composition.adl2
    );
    assert!(
        composition
            .adl2
            .contains("openEHR-EHR-OBSERVATION\\.minimal\\.v1"),
        "the include does not name the filling archetype:\n{}",
        composition.adl2
    );
}

/// The `C_DV_ORDINAL` domain constrainer becomes the AOM2
/// `[value, symbol]` attribute tuple (`master04.4-cadl_second_order.adoc`
/// §Tuple Constraints) — visible on the printed ADL2 surface of the
/// Vital Signs conversion. (Its quantities are magnitude-less, so they
/// widen to plain `units` lists — the reference-corpus form; the tuple
/// path is pinned by the unit test `quantity_tuple_emitted_when_magnitudes_present`.)
#[test]
fn domain_types_become_attribute_tuples() {
    let opt = parse_opt(VITAL_SIGNS);
    let conversion = convert_opt_to_adl2(&opt).expect("convert");
    let all = conversion
        .roots
        .iter()
        .map(|r| r.adl2.as_str())
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        all.contains("[value, symbol]"),
        "no ordinal tuple emitted anywhere in the Vital Signs conversion"
    );
    assert!(
        all.contains("[units, magnitude, precision]"),
        "the fully-constrained quantities must carry [units, magnitude, precision] tuples"
    );
}

/// The public result carries each root's conversion details and log beside its
/// ADL 2 text, and the printed text states the same details.
#[test]
fn converted_roots_carry_their_conversion_details() {
    let opt = parse_opt(ROLE);
    let conversion = convert_opt_to_adl2(&opt).unwrap_or_else(|e| panic!("convert Role.v2: {e}"));
    let role = conversion
        .roots
        .iter()
        .find(|root| root.archetype_id == "openEHR-DEMOGRAPHIC-ROLE.Role.v1")
        .expect("the ROLE root converts");
    assert!(
        role.conversion_details
            .values()
            .any(|v| v.contains("ac0001")),
        "the dangling ac0001 is reported on the result: {:?}",
        role.conversion_details
    );
    assert!(
        role.adl2.contains("conversion_details"),
        "the printed ADL 2 states the conversion details:\n{}",
        role.adl2
    );
    let first = convert_opt_to_adl2(&opt).expect("second convert");
    assert_eq!(
        conversion, first,
        "the conversion result, logs included, is deterministic"
    );
}

/// The roots come in document order: the template's top root first, then each
/// embedded root as the definition reaches it.
#[test]
fn roots_come_in_document_order() {
    let conversion = convert_opt_to_adl2(&parse_opt(MINIMAL_OBSERVATION)).expect("convert");
    let ids: Vec<&str> = conversion
        .roots
        .iter()
        .map(|root| root.archetype_id.as_str())
        .collect();
    assert_eq!(
        ids,
        [
            "openEHR-EHR-COMPOSITION.minimal.v1",
            "openEHR-EHR-OBSERVATION.minimal.v1"
        ]
    );
}
