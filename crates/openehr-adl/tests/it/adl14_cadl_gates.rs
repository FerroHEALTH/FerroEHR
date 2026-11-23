// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hand-written ADL 1.4 cADL regression corpus (`tests/corpus/adl14-cadl/`),
//! each expectation cited from `ADL1.4/master05-cadl.adoc` in its fixture.
//!
//! Dialect gates refuse the constructs ADL 2 introduced (§Keywords), whose
//! accepting twins are the ADL2 corpus; 1.4-only refusals keep their accepting
//! twins here; positive fixtures pin sibling order, the `{1..1}` default and
//! assumed values; and the breadth fixtures exercise every construct of the
//! chapter, case-insensitively (§Symbols).

#![allow(
    clippy::expect_used,
    clippy::panic,
    clippy::unwrap_used,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::assemble::parse_artefact;
use openehr_adl::error::SyntaxErrorCode;
use openehr_adl::parse::Dialect;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::validate_source_integrity;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

/// What a fixture must do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Expect {
    /// Parses in the 1.4 dialect and validates clean under the 1.4 phase-1 subset.
    Pass,
    /// Refused at parse with this syntax code.
    Refuse(SyntaxErrorCode),
    /// Parses, but the 1.4 phase-1 subset raises this validation code as an error.
    Invalid(ValidationCode),
}

/// Every fixture of the tree, with its expected outcome. The coverage gate
/// (`corpus_coverage.rs`) cross-checks this list against the filesystem.
const FIXTURES: &[(&str, Expect)] = &[
    // ── dialect gates (ADL2-only constructs in a 1.4 text) ────────────────
    (
        "openEHR-EHR-OBSERVATION.SCOAT_adl2_default_value.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Scoat),
    ),
    (
        "openEHR-EHR-OBSERVATION.SCOAT_adl2_attribute_tuple.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Scoat),
    ),
    (
        "openEHR-EHR-CLUSTER.SCCOG_adl2_use_archetype.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sccog),
    ),
    (
        "openEHR-EHR-CLUSTER.SCCOG_adl2_slot_closed.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sccog),
    ),
    (
        "openEHR-EHR-CLUSTER.STCCP_adl2_constraint_strength.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Stccp),
    ),
    (
        "openEHR-EHR-CLUSTER.STCCP_adl2_terminology_binding.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Stccp),
    ),
    // ── inline dADL domain lowering (master09) ────────────────────────────
    (
        "openEHR-EHR-CLUSTER.SDINV_unsupported_domain_type.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sdinv),
    ),
    (
        "openEHR-EHR-CLUSTER.SDINV_assumed_value_unmatched.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sdinv),
    ),
    (
        "openEHR-EHR-CLUSTER.domain_assumed_value.v1.adl",
        Expect::Pass,
    ),
    (
        "openEHR-EHR-CLUSTER.code_phrase_dadl_block.v1.adl",
        Expect::Pass,
    ),
    // ── the deprecated 1.4 pipe-ordinal shorthand (master04.4) ────────────
    (
        "openEHR-EHR-CLUSTER.ordinal_pipe_shorthand.v1.adl",
        Expect::Pass,
    ),
    // ── docs-text-silent forms the reference grammar admits and CKM emits ─
    (
        "openEHR-EHR-CLUSTER.term_constraint_open_and_mixed_ordinal.v1.adl",
        Expect::Pass,
    ),
    // The empty inline dADL domain block, both spellings — lowers to the open
    // constraint (#1465 family 3; the citation chain is in the fixture).
    (
        "openEHR-EHR-CLUSTER.empty_domain_block.v1.adl",
        Expect::Pass,
    ),
    // Heterogeneous C_DV_QUANTITY list rows — partition into sibling
    // alternatives (#1466; the conversion shape is pinned in
    // `ckm_conversion_breadth.rs`).
    (
        "openEHR-EHR-CLUSTER.heterogeneous_quantity_rows.v1.adl",
        Expect::Pass,
    ),
    // ── 1.4 term-constraint definedness (master08 VATDF/VACDF) ────────────
    (
        "openEHR-EHR-CLUSTER.VATDF_undefined_listed_code.v1.adl",
        Expect::Invalid(ValidationCode::Vatdf),
    ),
    (
        "openEHR-EHR-CLUSTER.VATDF_undefined_code_with_assumed.v1.adl",
        Expect::Invalid(ValidationCode::Vatdf),
    ),
    (
        "openEHR-EHR-CLUSTER.VACDF_undefined_constraint_code.v1.adl",
        Expect::Invalid(ValidationCode::Vacdf),
    ),
    (
        "openEHR-EHR-CLUSTER.term_constraint_codes_defined.v1.adl",
        Expect::Pass,
    ),
    // ── 1.4 term-constraint LIST integrity (master04.6 STCDC/STCAC) ───────
    (
        "openEHR-EHR-CLUSTER.STCDC_duplicate_code_in_list.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Stcdc),
    ),
    (
        "openEHR-EHR-CLUSTER.STCAC_assumed_code_not_in_list.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Stcac),
    ),
    // ── operators the chapter names but no grammar defines ────────────────
    (
        "openEHR-EHR-CLUSTER.SCCOG_negated_matches.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sccog),
    ),
    (
        "openEHR-EHR-CLUSTER.SCOAT_negated_is_in.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Scoat),
    ),
    (
        "openEHR-EHR-CLUSTER.SCOAT_not_in_symbol.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Scoat),
    ),
    (
        "openEHR-EHR-CLUSTER.SCCOG_regex_match_operator.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sccog),
    ),
    // ── the date/time interval timezone-symmetry rule (master05 L932) ─────
    (
        "openEHR-EHR-CLUSTER.SCDTAV_interval_timezone_asymmetry.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Scdtav),
    ),
    // ── cardinality/occurrences (master05 VCOC + the {1..1} defaults) ─────
    (
        "openEHR-EHR-CLUSTER.VCOC_occurrences_exceed_cardinality.v1.adl",
        Expect::Invalid(ValidationCode::Vcoc),
    ),
    (
        "openEHR-EHR-CLUSTER.VCOC_occurrences_below_cardinality.v1.adl",
        Expect::Invalid(ValidationCode::Vcoc),
    ),
    (
        "openEHR-EHR-CLUSTER.default_occurrences.v1.adl",
        Expect::Pass,
    ),
    // ── 1.4 keywords that must stay accepted ──────────────────────────────
    ("openEHR-EHR-CLUSTER.sibling_order.v1.adl", Expect::Pass),
    ("openEHR-EHR-CLUSTER.cadl_keyword_case.v1.adl", Expect::Pass),
    // ── the accepting twins of the two vendored concept-less 1.4 fixtures ─
    // `ADL1.4/master08-adl.adoc` makes `arch_concept` mandatory (VARCN).
    (
        "openehr-TEST_PKG-SOME_TYPE.c_dv_quantity_concept.v1.adl",
        Expect::Pass,
    ),
    (
        "openehr-TEST_PKG-SOME_TYPE.code_phrase_concept.v1.adl",
        Expect::Pass,
    ),
    // A slot assertion targeting domain_concept: its literal regex is NOT an
    // archetype id, so VDFAI must not fire (master05 §Archetype Slots; the
    // VDFAI subject is the archetype identifier).
    (
        "openEHR-EHR-SECTION.slot_domain_concept_regex.v1.adl",
        Expect::Pass,
    ),
    // ── the `\u` escape twins (`master03-file_encoding.adoc` §File Encoding) ─
    // Both `\uHHHH` and `\uHHHHHHHH` decode; an out-of-range code point is refused.
    (
        "openEHR-EHR-CLUSTER.unicode_escape_8_digit.v1.adl",
        Expect::Pass,
    ),
    (
        "openEHR-EHR-CLUSTER.SUNK_unicode_escape_out_of_range.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sunk),
    ),
    // The escape set is closed: "Any other character combination starting with a
    // backslash is illegal" (§Special Character Sequences); a regex literal is
    // never escape-decoded.
    (
        "openEHR-EHR-CLUSTER.SUNK_illegal_string_escape.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sunk),
    ),
    // ── the master05 breadth trio (every construct of the chapter) ────────
    (
        "openEHR-EHR-OBSERVATION.cadl_breadth_structure.v1.adl",
        Expect::Pass,
    ),
    (
        "openEHR-EHR-OBSERVATION.cadl_breadth_primitives.v1.adl",
        Expect::Pass,
    ),
    (
        "openEHR-EHR-OBSERVATION.cadl_breadth_datetime.v1.adl",
        Expect::Pass,
    ),
];

fn tree() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/adl14-cadl")
}

fn read(name: &str) -> String {
    let path = tree().join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn error_codes(src: &str, name: &str) -> Vec<ValidationCode> {
    validate_source_integrity(src, Dialect::Adl14, None)
        .unwrap_or_else(|e| panic!("{name}: must parse, got {e:?}"))
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect()
}

#[test]
fn every_fixture_meets_its_declared_outcome() {
    for (name, expect) in FIXTURES {
        let src = read(name);
        match expect {
            Expect::Pass => {
                parse_artefact(&src, Dialect::Adl14)
                    .unwrap_or_else(|e| panic!("{name} must parse, got {e:?}"));
                let codes = error_codes(&src, name);
                assert!(
                    codes.is_empty(),
                    "{name}: expected clean 1.4 validation, got {:?}",
                    codes.iter().map(|c| c.mnemonic()).collect::<Vec<_>>()
                );
            }
            Expect::Refuse(code) => {
                let errs = parse_artefact(&src, Dialect::Adl14)
                    .err()
                    .unwrap_or_else(|| panic!("{name} must be refused at parse"));
                assert!(
                    errs.iter().any(|e| e.code == *code),
                    "{name}: expected {code}, got {:?}",
                    errs.iter().map(|e| e.code).collect::<Vec<_>>()
                );
            }
            Expect::Invalid(code) => {
                let codes = error_codes(&src, name);
                assert!(
                    codes.contains(code),
                    "{name}: expected {code}, got {:?}",
                    codes.iter().map(|c| c.mnemonic()).collect::<Vec<_>>()
                );
            }
        }
    }
}

/// The `\u` escape twins decode to the characters `ADL1.4/master03-file_encoding.adoc`
/// §File Encoding names, and the out-of-range refusal names the defect.
#[test]
fn the_unicode_escape_twins_decode_and_refuse_on_content() {
    let src = read("openEHR-EHR-CLUSTER.unicode_escape_8_digit.v1.adl");
    let art = parse_artefact(&src, Dialect::Adl14).expect("the accepting twin must parse");
    let mut values = Vec::new();
    collect_string_constraints(&art, &mut values);
    assert_eq!(
        values,
        vec![
            "\u{e9}".to_owned(),
            "\u{1F600}".to_owned(),
            "\u{1F600}".to_owned()
        ],
        "the three escape spellings must decode to e-acute and two emoji"
    );

    let src = read("openEHR-EHR-CLUSTER.SUNK_unicode_escape_out_of_range.v1.adl");
    let errs = parse_artefact(&src, Dialect::Adl14).expect_err("the refusing twin must be refused");
    let messages: Vec<&str> = errs.iter().map(|e| e.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("0000FFFF")),
        "the refusal must name the offending escape, got {messages:?}"
    );
}
/// A `C_STRING` with any other backslash combination is refused at the lex
/// (`ADL1.4/master03-file_encoding.adoc` §Special Character Sequences).
#[test]
fn an_illegal_string_escape_is_refused_naming_the_literal() {
    let src = read("openEHR-EHR-CLUSTER.SUNK_illegal_string_escape.v1.adl");
    let errs =
        parse_artefact(&src, Dialect::Adl14).expect_err("the illegal escape must be refused");
    let messages: Vec<&str> = errs.iter().map(|e| e.message.as_str()).collect();
    assert!(
        messages.iter().any(|m| m.contains("qb")),
        "the refusal must name the offending literal, got {messages:?}"
    );
}

/// Every `C_STRING` constraint value of an archetype definition, in document
/// order.
fn collect_string_constraints(art: &Archetype, out: &mut Vec<String>) {
    let Archetype::AuthoredArchetype(authored) = art else {
        return;
    };
    let AuthoredArchetype::AuthoredArchetype(data) = authored.as_ref() else {
        return;
    };
    collect_from_complex(&data.definition, out);
}

fn collect_from_complex(cco: &CComplexObject, out: &mut Vec<String>) {
    let CComplexObject::CComplexObject(data) = cco else {
        return;
    };
    for attr in data.attributes.iter().flatten() {
        for child in attr.children.iter().flatten() {
            match child {
                CObject::CString(c) => out.extend(c.constraint.iter().flatten().cloned()),
                CObject::CComplexObject(inner) => collect_from_complex(inner, out),
                _ => {}
            }
        }
    }
}

/// The whole tree is claimed by the table above — no fixture may sit unexercised.
#[test]
fn every_fixture_file_is_in_the_table() {
    let mut on_disk: Vec<String> = std::fs::read_dir(tree())
        .expect("read the adl14-cadl tree")
        .flatten()
        .map(|e| e.file_name().to_string_lossy().to_string())
        .filter(|n| Path::new(n).extension().is_some_and(|e| e == "adl"))
        .collect();
    on_disk.sort();
    let mut listed: Vec<String> = FIXTURES.iter().map(|(n, _)| (*n).to_owned()).collect();
    listed.sort();
    assert_eq!(on_disk, listed, "the fixture table and the tree disagree");
}

/// W14DEP: the paren-less domain-block spelling warns once per occurrence
/// (`master05-cadl` §Symbols `V_C_DOMAIN_TYPE`); the parenthesised one does not.
#[test]
fn deprecated_paren_less_domain_spelling_warns_once() {
    let src = std::fs::read_to_string(tree().join("openEHR-EHR-CLUSTER.empty_domain_block.v1.adl"))
        .expect("fixture exists");
    let issues = validate_source_integrity(&src, Dialect::Adl14, None).expect("the fixture parses");
    let warnings: Vec<_> = issues
        .iter()
        .filter(|i| i.code == ValidationCode::W14dep)
        .collect();
    assert_eq!(
        warnings.len(),
        1,
        "exactly the bare `C_DV_QUANTITY <` spelling warns (the parenthesised \
         `(C_DV_ORDINAL) <` does not), got: {warnings:?}"
    );
    assert!(
        warnings[0].severity == Severity::Warning,
        "W14DEP is advisory, never an error"
    );
    assert!(
        warnings[0].message.contains("(C_DV_QUANTITY) <"),
        "the warning names the preferred spelling: {}",
        warnings[0].message
    );
}
