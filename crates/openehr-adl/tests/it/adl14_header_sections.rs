// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 **header sections** — `AM/docs/ADL1.4/master08-adl.adoc`
//! §Header Sections, §Syntax Specification (the grammar + §Symbols lexical
//! rules) and §Validity Rules.
//!
//! Three behaviours of the 1.4 outer structure that ADL2 does not share, each
//! pinned here with both twins (`.claude/rules/testing.md`):
//!
//! 1. **Section keywords are case-insensitive.** §Symbols spells every one of
//!    them in the bracketed-alternative form
//!    (`^[Aa][Rr][Cc][Hh][Ee][Tt][Yy][Pp][Ee][ \t\r]*\n -> SYM_ARCHETYPE`, and
//!    likewise `SYM_SPECIALIZE`/`SYM_CONCEPT`/`SYM_DEFINITION`/`SYM_LANGUAGE`/
//!    `SYM_DESCRIPTION`/`SYM_INVARIANT`/`SYM_ONTOLOGY`). The ADL2 grammar
//!    (`adl_keywords.g4`) spells them as exact lowercase literals, so the
//!    tolerance is `Dialect::Adl14`-only — the refusing twin is an ADL2 source.
//! 2. **The old form without a `language` section is accepted and upgraded.**
//!    §Syntax Specification has `arch_language: // empty OK`, and both
//!    §Language Section and Language Translation and §Ontology Header
//!    Statements instruct tool builders to accept archetypes carrying
//!    `primary_language`/`languages_available` in the ontology and upgrade them
//!    on parse to `original_language`/`translations`. With no
//!    `primary_language` to upgrade from, `SALAN` stands.
//! 3. **The `concept` section is mandatory and its term must be defined.**
//!    §Syntax Specification gives `arch_concept: SYM_CONCEPT
//!    V_LOCAL_TERM_CODE_REF | SYM_CONCEPT error` — the only header production
//!    with no empty alternative — and §Validity Rules VARCN: "The archetype
//!    must have an archetype term value in the concept section. The term must
//!    exist in the archetype ontology."

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::assemble::parse_artefact;
use openehr_adl::error::{SyntaxError, SyntaxErrorCode};
use openehr_adl::parse::Dialect;
use openehr_adl::source::parse_source;
use openehr_adl::validate::catalogue::{Severity, ValidationCode};
use openehr_adl::validate::validate_source_integrity;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};

/// A well-formed ADL 1.4 archetype in the canonical lower-case spelling.
const LOWER_CASE_14: &str = "\
archetype (adl_version=1.4)
\topenEHR-EHR-CLUSTER.header_sections.v1

concept
\t[at0000]\t-- Header sections

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"AuthorDraft\">

definition
\tCLUSTER[at0000] matches {
\t\titems cardinality matches {0..*; unordered} matches {
\t\t\tELEMENT[at0001] occurrences matches {0..1} matches {*}
\t\t}
\t}

ontology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"root\"> description=<\"root\">>
\t\t\t\t[\"at0001\"] = <text=<\"element\"> description=<\"element\">>
\t\t\t>
\t\t>
\t>
";

/// The same archetype with every SECTION KEYWORD in a different case — the
/// §Symbols lexical rules fold all of them.
const MIXED_CASE_14: &str = "\
ARCHETYPE (adl_version=1.4)
\topenEHR-EHR-CLUSTER.header_sections.v1

Concept
\t[at0000]\t-- Header sections

LANGUAGE
\toriginal_language = <[ISO_639-1::en]>

DeScRiPtIoN
\tlifecycle_state = <\"AuthorDraft\">

DEFINITION
\tCLUSTER[at0000] matches {
\t\titems cardinality matches {0..*; unordered} matches {
\t\t\tELEMENT[at0001] occurrences matches {0..1} matches {*}
\t\t}
\t}

ONTOLOGY
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"root\"> description=<\"root\">>
\t\t\t\t[\"at0001\"] = <text=<\"element\"> description=<\"element\">>
\t\t\t>
\t\t>
\t>
";

/// A minimal ADL2 source (exact-lowercase keywords, `adl_keywords.g4`).
const LOWER_CASE_ADL2: &str = "\
archetype (adl_version=2.0.5; rm_release=1.0.2)
\topenehr-TEST_PKG-WHOLE.case.v1.0.0

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"published\">

definition
\tWHOLE[id1]

terminology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\t[\"id1\"] = <text=<\"root\"> description=<\"root\">>
\t\t>
\t>
";

/// The `AUTHORED_ARCHETYPE` data of an assembled archetype.
fn authored(archetype: &Archetype) -> &AuthoredArchetypeData {
    match archetype {
        Archetype::AuthoredArchetype(a) => match a.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => d,
            other
            @ (AuthoredArchetype::Template(_) | AuthoredArchetype::OperationalTemplate(_)) => {
                panic!("expected a plain AUTHORED_ARCHETYPE, got {other:?}")
            }
        },
        other @ Archetype::TemplateOverlay(_) => {
            panic!("expected an AUTHORED_ARCHETYPE, got {other:?}")
        }
    }
}

/// The phase-1 error codes a 1.4 source raises.
fn error_codes(src: &str) -> Vec<ValidationCode> {
    validate_source_integrity(src, Dialect::Adl14, None)
        .expect("the 1.4 source parses")
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code)
        .collect()
}

/// §Symbols: `^[Aa][Rr][Cc][Hh][Ee][Tt][Yy][Pp][Ee]…` — every 1.4 section
/// keyword lexes case-insensitively, so the mixed-case spelling must parse,
/// assemble and validate exactly like the canonical one.
#[expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the parse plumbing while the \
              assertions ARE the test — an assertion panic is how this test fails, and \
              the clippy.toml allow-*-in-tests scoping does not reach an integration \
              test binary"
)]
#[test]
fn adl14_section_keywords_are_case_insensitive() -> Result<(), Vec<SyntaxError>> {
    let lower = parse_artefact(LOWER_CASE_14, Dialect::Adl14)?;
    let mixed = parse_artefact(MIXED_CASE_14, Dialect::Adl14)?;
    assert_eq!(authored(&mixed).archetype_id, authored(&lower).archetype_id);
    assert_eq!(authored(&mixed).definition, authored(&lower).definition);
    assert_eq!(authored(&mixed).terminology, authored(&lower).terminology);
    assert!(error_codes(MIXED_CASE_14).is_empty());
    Ok(())
}

/// §Specialise Section + §Symbols
/// (`^[Ss][Pp][Ee][Cc][Ii][Aa][Ll][Ii][SsZz][Ee]`): both spellings of the word
/// AND any case of either are the same keyword.
#[expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the parse plumbing while the \
              assertions ARE the test — an assertion panic is how this test fails, and \
              the clippy.toml allow-*-in-tests scoping does not reach an integration \
              test binary"
)]
#[test]
fn adl14_specialise_keyword_is_case_insensitive() -> Result<(), Vec<SyntaxError>> {
    for spelling in ["Specialise", "SPECIALIZE", "specialise", "specialize"] {
        let src = LOWER_CASE_14.replacen(
            "\nconcept\n\t[at0000]",
            &format!(
                "\n{spelling}\n\topenEHR-EHR-CLUSTER.header_sections.v1\n\nconcept\n\t[at0000.1]"
            ),
            1,
        );
        let parsed = parse_source(&src, Dialect::Adl14)?;
        let parent = parsed
            .parent_ref
            .expect("the specialise parent is captured");
        assert_eq!(parent.concept_id, "header_sections");
        assert_eq!(parsed.concept.as_deref(), Some("at0000.1"));
    }
    Ok(())
}

/// The refusing twin of the two tests above: the ADL2 grammar spells its
/// keywords as exact lowercase literals (`adl_keywords.g4`), so an upper-case
/// word is not a keyword there. `ARCHETYPE` is no artefact keyword (`SUNK`),
/// and `TERMINOLOGY` is no section header — the word is swallowed by the
/// preceding section's body, leaving the artefact without a terminology
/// section (`SAON`).
#[expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the parse plumbing while the \
              assertions ARE the test — an assertion panic is how this test fails, and \
              the clippy.toml allow-*-in-tests scoping does not reach an integration \
              test binary"
)]
#[test]
fn adl2_rejects_upper_case_section_keywords() -> Result<(), Vec<SyntaxError>> {
    parse_source(LOWER_CASE_ADL2, Dialect::Adl2)?;

    let upper_artefact = LOWER_CASE_ADL2.replacen("archetype", "ARCHETYPE", 1);
    let errs =
        parse_source(&upper_artefact, Dialect::Adl2).expect_err("ADL2 keywords are case-sensitive");
    assert!(
        errs.iter().any(|e| e.code == SyntaxErrorCode::Sunk),
        "{errs:?}"
    );

    let upper_section = LOWER_CASE_ADL2.replacen("\nterminology\n", "\nTERMINOLOGY\n", 1);
    let errs =
        parse_source(&upper_section, Dialect::Adl2).expect_err("ADL2 keywords are case-sensitive");
    assert!(
        errs.iter().any(|e| e.code == SyntaxErrorCode::Saon),
        "{errs:?}"
    );
    Ok(())
}

/// An old-form 1.4 archetype: no `language` section, `primary_language` +
/// `languages_available` in the ontology instead. §Language Section and
/// Language Translation + §Ontology Header Statements: "tool builders should
/// consider accepting archetypes of the old form and upgrading them when
/// parsing to the correct form, which should then be used for
/// serialising/saving."
const OLD_FORM_14: &str = "\
archetype (adl_version=1.4)
\topenEHR-EHR-CLUSTER.old_form_language.v1

concept
\t[at0000]\t-- Old-form language

description
\tlifecycle_state = <\"AuthorDraft\">

definition
\tCLUSTER[at0000] matches {
\t\titems cardinality matches {0..*; unordered} matches {
\t\t\tELEMENT[at0001] occurrences matches {0..1} matches {*}
\t\t}
\t}

ontology
\tprimary_language = <\"en\">
\tlanguages_available = <\"en\", \"de\", ...>
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"root\"> description=<\"root\">>
\t\t\t\t[\"at0001\"] = <text=<\"element\"> description=<\"element\">>
\t\t\t>
\t\t>
\t\t[\"de\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"wurzel\"> description=<\"wurzel\">>
\t\t\t\t[\"at0001\"] = <text=<\"element\"> description=<\"element\">>
\t\t\t>
\t\t>
\t>
";

#[expect(
    clippy::panic_in_result_fn,
    reason = "the Book ch11 test shape: `?` propagates the parse plumbing while the \
              assertions ARE the test — an assertion panic is how this test fails, and \
              the clippy.toml allow-*-in-tests scoping does not reach an integration \
              test binary"
)]
#[test]
fn adl14_old_form_language_is_upgraded_from_the_ontology() -> Result<(), Vec<SyntaxError>> {
    let archetype = parse_artefact(OLD_FORM_14, Dialect::Adl14)?;
    let data = authored(&archetype);
    // `primary_language` → `original_language`.
    assert_eq!(data.original_language.code_string, "en");
    assert_eq!(data.original_language.terminology_id, "ISO_639-1");
    // every other `languages_available` entry → a minimal translation.
    let translations = data
        .translations
        .as_ref()
        .expect("languages_available yields translations");
    assert_eq!(translations.keys().collect::<Vec<_>>(), vec!["de"]);
    assert_eq!(
        translations
            .get("de")
            .expect("the de translation")
            .language
            .code_string,
        "de"
    );
    // the terminology's own original_language follows the upgrade.
    assert_eq!(data.terminology.original_language, "en");
    Ok(())
}

/// The refusing twin: with NO `primary_language` in the ontology there is
/// nothing to upgrade from, so the missing `language` section stays `SALAN`.
#[test]
fn adl14_old_form_without_primary_language_is_salan() {
    let src = OLD_FORM_14.replacen("\tprimary_language = <\"en\">\n", "", 1);
    let errs = parse_artefact(&src, Dialect::Adl14).expect_err("nothing to upgrade from");
    assert!(
        errs.iter().any(|e| e.code == SyntaxErrorCode::Salan),
        "{errs:?}"
    );
}

/// ADL2 keeps `SALAN` unconditionally: it has no old form to upgrade.
#[test]
fn adl2_missing_language_section_is_salan() {
    let src = LOWER_CASE_ADL2.replacen(
        "language\n\toriginal_language = <[ISO_639-1::en]>\n\n",
        "",
        1,
    );
    let errs = parse_source(&src, Dialect::Adl2).expect_err("ADL2 requires a language section");
    assert!(
        errs.iter().any(|e| e.code == SyntaxErrorCode::Salan),
        "{errs:?}"
    );
}

/// §Syntax Specification `arch_concept` has no empty alternative (unlike
/// `arch_specialisation`/`arch_language`/`arch_description`/`arch_invariant`),
/// and §Validity Rules VARCN requires the term value: a 1.4 archetype without a
/// `concept` section is refused with the concept clause's catalogue code.
#[test]
fn adl14_missing_concept_section_is_saco() {
    let src = LOWER_CASE_14.replacen("concept\n\t[at0000]\t-- Header sections\n\n", "", 1);
    assert!(!src.contains("concept"), "the concept section was removed");
    let errs = parse_artefact(&src, Dialect::Adl14).expect_err("1.4 mandates the concept section");
    assert!(
        errs.iter().any(|e| e.code == SyntaxErrorCode::Saco),
        "{errs:?}"
    );
}

/// The same code for the grammar's `SYM_CONCEPT error` alternative: a `concept`
/// clause whose body is not a local term-code reference.
#[test]
fn adl14_malformed_concept_clause_is_saco() {
    let src = LOWER_CASE_14.replacen("\t[at0000]\t-- Header sections", "\t\"at0000\"", 1);
    let errs =
        parse_artefact(&src, Dialect::Adl14).expect_err("the concept clause needs a term code");
    assert!(
        errs.iter().any(|e| e.code == SyntaxErrorCode::Saco),
        "{errs:?}"
    );
}

/// VARCN, terminology half: "The term must exist in the archetype ontology."
/// `[at0099]` is defined nowhere in `term_definitions`.
#[test]
fn adl14_undefined_concept_term_raises_varcn() {
    let src = LOWER_CASE_14.replacen("\t[at0000]\t-- Header", "\t[at0099]\t-- Header", 1);
    assert!(
        error_codes(&src).contains(&ValidationCode::Varcn),
        "expected VARCN, got {:?}",
        error_codes(&src)
    );
}

/// The accepting twin: the concept term IS defined in the ontology, so VARCN
/// stays quiet and the whole 1.4 phase-1 subset is clean.
#[test]
fn adl14_defined_concept_term_validates_clean() {
    assert!(
        error_codes(LOWER_CASE_14).is_empty(),
        "expected a clean 1.4 validation, got {:?}",
        error_codes(LOWER_CASE_14)
    );
}

// ── ADL2: the deprecated concept section (`ADL2/master07.09`) ────────────────

/// A well-formed deprecated concept section is accepted and ignored in ADL2
/// (both the inline and next-line spellings), and a malformed one raises SACO
/// — "if a concept section is present, it must consist of the 'concept'
/// keyword and a single local term" (`master07.09-adl_deprecated.adoc`).
#[test]
fn adl2_deprecated_concept_section_is_shape_checked() {
    let rest = "\n\nlanguage\n    original_language = <[ISO_639-1::en]>\n\ndescription\n    lifecycle_state = <\"draft\">\n\ndefinition\n    OBSERVATION[id1] matches {*}\n\nterminology\n    term_definitions = <\n        [\"en\"] = <\n            [\"id1\"] = < text = <\"t\"> description = <\"t\"> >\n        >\n    >\n";
    let head =
        "archetype (adl_version=2.0.5; rm_release=1.0.2)\n    openEHR-EHR-OBSERVATION.t.v1.0.0\n\n";
    for ok in [
        format!("{head}concept [at0000] -- haematology result{rest}"),
        format!("{head}concept\n    [at0000]{rest}"),
    ] {
        assert!(
            parse_artefact(&ok, Dialect::Adl2).is_ok(),
            "a well-formed deprecated concept section must be accepted"
        );
    }
    let bad = format!("{head}concept\n    not_a_code{rest}");
    let errs = parse_artefact(&bad, Dialect::Adl2).unwrap_err();
    assert!(
        errs.iter().any(|e| e.code == SyntaxErrorCode::Saco),
        "a malformed concept section must raise SACO, got {errs:?}"
    );
}
