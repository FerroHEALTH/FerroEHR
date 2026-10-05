// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 cADL regression corpus (`tests/corpus/adl14-cadl/`).
//!
//! Sibling of `adl14-dadl/` (the dADL leaf/structure breadth tree): this one
//! covers the **cADL** half of an ADL 1.4 text — `master05-cadl.adoc` — where the
//! vendored `adl2-reference` library gives no coverage because it is an ADL2
//! corpus. Like its sibling it is **hand-written, not vendored**
//! (`tests/corpus/PROVENANCE.md` §`adl14-cadl/`), and every expectation is
//! derived first-hand from the spec text, cited in the fixture beside the
//! construct it exercises.
//!
//! Four families live here:
//!
//! 1. **Dialect gates.** ADL 1.4's cADL keyword set is CLOSED
//!    (`ADL1.4/master05-cadl.adoc` §Keywords L48-53), so a construct ADL 2
//!    introduced is a syntax error in a 1.4 text, not a tolerated superset. Each
//!    such construct gets a refusal fixture named for the `S*` code it raises.
//!    The ACCEPTING twin of every one of them is the vendored ADL2 corpus, which
//!    exercises the same construct in its own dialect — a parser that stopped
//!    accepting it there would fail `corpus_definition_parse.rs`.
//! 2. **Refusals with their accepting twin in-tree** — the inline dADL domain
//!    lowering (`ADL1.4/master09-customising_adl.adoc`) and the 1.4 term-constraint
//!    definedness rules (`ADL1.4/master08-adl.adoc` §Validity Rules VATDF/VACDF),
//!    where the accepted form is itself 1.4-only and so has no ADL2 twin.
//! 3. **Positive fixtures** for behaviour a lenient OR an over-strict reader would
//!    get wrong: `before`/`after` sibling order (a 1.4 keyword — L53 — that must
//!    NOT be gated), the effective occurrences default `{1..1}` (L316), and the
//!    assumed-value lowering.
//! 4. **The breadth trio** — `cadl_breadth_{structure,primitives,datetime}` —
//!    which exercises every construct `master05-cadl.adoc` defines (existence
//!    and occurrences and cardinality spellings incl. two modifiers and the bare
//!    star, the whole interval shape set incl. the `infinity` endpoints and both
//!    exclusive-lower spellings, every string/boolean/character/term-code form,
//!    every date/time/date-time/duration pattern family incl. the
//!    literal-substituted and ASCII-timezone variants and the mixed
//!    `pattern/interval` form, assumed values on every primitive, `use_node`
//!    with and without occurrences, generic type names, and the bare and
//!    identified slot forms) plus `cadl_keyword_case`, which repeats a whole
//!    definition in upper/mixed case because the chapter's own lexical
//!    specification is case-insensitive (§Symbols L1326-1354).
//!
//! Both twins are kept for every refusal (`.claude/rules/testing.md`).

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
    // `ADL1.4/master08-adl.adoc` §Syntax Specification makes `arch_concept`
    // mandatory and §Validity Rules VARCN requires its term, so the two
    // concept-less `legacy_adl_1.4` fixtures are refused (SACO — pinned in
    // `legacy14_corpus.rs`). These are their concept-carrying twins, so the
    // constructs they exercise (1.4 inline dADL `C_DV_QUANTITY` blocks; the
    // `CODE_PHRASE` constraint spellings) keep their accepted coverage.
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
    // The chapter defines BOTH `\uHHHH` (BMP) and `\uHHHHHHHH`
    // (U+10000-U+10FFFF, "the algorithm is described in IETF RFC 2781"), so a
    // C_STRING carrying either spelling must decode; an 8-digit spelling of a
    // code point outside that range denotes nothing the form can carry and is
    // refused rather than silently substituted.
    (
        "openEHR-EHR-CLUSTER.unicode_escape_8_digit.v1.adl",
        Expect::Pass,
    ),
    (
        "openEHR-EHR-CLUSTER.SUNK_unicode_escape_out_of_range.v1.adl",
        Expect::Refuse(SyntaxErrorCode::Sunk),
    ),
    // The escape set is CLOSED — the six customary quoted forms plus the two
    // `\u` spellings — because "Any other character combination starting with
    // a backslash is illegal" (§Special Character Sequences). The accepting
    // twin of this one is `unicode_escape_8_digit` above, whose three legal
    // spellings decode; the PERL classes the same section names (`\s`, `\d`)
    // are legal only inside a regex literal, which is never escape-decoded
    // (`slot_domain_concept_regex` and `cadl_breadth_primitives` are their
    // accepting twins).
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

/// The `\u` escape twins, checked on their CONTENT rather than only their
/// outcome: the 4-digit BMP form, the RFC 2781 surrogate pair and the
/// zero-filled 8-digit spelling all decode to the characters
/// `ADL1.4/master03-file_encoding.adoc` §File Encoding names, and the
/// out-of-range twin's refusal names the defect.
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
/// The illegal-escape twin: the escape set is CLOSED at the six customary
/// quoted forms plus the two `\u` spellings, so a `C_STRING` carrying any
/// other backslash combination is refused at the lex rather than read as
/// literal text (`ADL1.4/master03-file_encoding.adoc` §Special Character
/// Sequences: "Any other character combination starting with a backslash is
/// illegal"). Its accepting twin is `unicode_escape_8_digit` above.
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

/// W14DEP (#1470): the deprecated paren-less domain-block spelling is
/// ACCEPTED but WARNED, per occurrence, at exactly the deprecation's spec
/// strength (`master05-cadl` §Symbols `V_C_DOMAIN_TYPE`); the parenthesised
/// spelling warns nothing. The `empty_domain_block` fixture carries exactly
/// one of each spelling, so the warning count pins both directions.
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
