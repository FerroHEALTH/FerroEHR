// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 assertion sub-language operator/symbol matrix
//! (`ADL1.4/master06-assertions.adoc`): every operator the chapter defines —
//! textual and symbolic renderings alike — parses in a 1.4 `invariant`
//! section, and the reserved-but-undefined forms reject loudly.
//!
//! The chapter's own yacc omits `%` (prose §Arithmetic Operators defines it)
//! and gives `for_all` no production at all (it is only a listed keyword) —
//! both upstream defects recorded in the #768 audit; the prose-defined `%`
//! parses, the production-less `for_all`-over-paths stays a typed reject.

use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;

/// Wrap one invariant line in a minimal, valid 1.4 archetype.
fn archetype_with_invariant(invariant: &str) -> String {
    format!(
        "archetype (adl_version=1.4)\n    openEHR-EHR-OBSERVATION.inv.v1\n\nconcept\n    [at0000]\n\n\
         language\n    original_language = <[ISO_639-1::en]>\n\ndescription\n    original_author = <\n        [\"name\"] = <\"t\">\n    >\n    \
         details = <\n        [\"en\"] = <\n            language = <[ISO_639-1::en]>\n            purpose = <\"t\">\n        >\n    >\n    \
         lifecycle_state = <\"unmanaged\">\n\ndefinition\n    OBSERVATION[at0000] matches {{\n        data matches {{\n            HISTORY[at0001] matches {{*}}\n        }}\n    }}\n\n\
         invariant\n    {invariant}\n\nontology\n    term_definitions = <\n        [\"en\"] = <\n            items = <\n                [\"at0000\"] = <\n                    text = <\"t\">\n                    description = <\"t\">\n                >\n                [\"at0001\"] = <\n                    text = <\"h\">\n                    description = <\"h\">\n                >\n            >\n        >\n    >\n"
    )
}

/// Every chapter-defined assertion form parses (§Keywords, §Operators,
/// §Operands — textual and symbolic renderings).
#[test]
fn chapter_operator_matrix_parses() {
    let accepted = [
        // tagged assertion + arithmetic (§Overview's own example)
        "validity: /speed[at0002]/kilometres/magnitude = /speed[at0004]/miles/magnitude * 1.6",
        // arithmetic operators (§Arithmetic Operators) — incl. the
        // prose-defined `%` the chapter yacc omits, and `^`
        "/a[at0001]/b % 2 = 0",
        "/a[at0001]/b ^ 2 > 4",
        "/a[at0001]/b + 1 - 2 * 3 / 4 < 9",
        // equality (§Equality Operators) — both `=` and the 1.4 `<>`
        "/a[at0001]/b <> 3",
        // relational (§Relational Operators)
        "/a[at0001]/b < 1 and /a[at0001]/b <= 1 and /a[at0001]/b > 0 and /a[at0001]/b >= 0",
        // boolean operators + quantifier keyword `exists` (§Boolean
        // Operators, §Quantifiers)
        "exists /data[at0001]",
        "True or False",
        "not (/data[at0001]/origin/value matches {\"x\"})",
        "/a[at0001]/b > 3 and /a[at0001]/c <= 2 xor True implies False",
        // symbolic renderings (§Keywords table): ∃ over a path ≡ exists,
        // ∧ ∨ ¬ ~ ∈
        "\u{2203} /data[at0001]",
        "True \u{2227} False",
        "True \u{2228} False",
        "\u{ac} True",
        "~ True",
        "/a[at0001]/b \u{2208} {1, 2}",
        "/a[at0001]/b is_in {1, 2}",
        // `not`/`~` as a prefix on other operators (§Keywords: applies to
        // all operators except for_all)
        "~ (/a[at0001]/b matches {3})",
    ];
    for invariant in accepted {
        let text = archetype_with_invariant(invariant);
        assert!(
            parse_artefact(&text, Dialect::Adl14,).is_ok(),
            "chapter form must parse: {invariant}"
        );
    }
}

/// `for_all` is a listed keyword the chapter's own grammar gives NO
/// production (and the prose exempts it from `not`-prefixing) — a
/// path-quantified spelling has no defined 1.4 form and rejects loudly.
#[test]
fn for_all_over_paths_stays_a_typed_reject() {
    let text = archetype_with_invariant("for_all /data[at0001]/events : /a[at0001]/b > 0");
    assert!(
        parse_artefact(&text, Dialect::Adl14,).is_err(),
        "a production-less form must not silently parse"
    );
}

/// Every path form the paths chapter defines parses as an assertion operand
/// (`ADL1.4/master07-paths.adoc`): absolute and relative paths (incl. the
/// yacc's single-segment `relative_path: path_segment`), movable `//` path
/// patterns (§Grammar `movable_path: SYM_MOVABLE_LEADER relative_path`), and
/// the three §Relationship-with-Xpath predicate forms — position (`[1]`),
/// meaning (`[systolic]`), node id (`[at0001]`, incl. dotted specialised
/// codes).
#[test]
fn chapter7_path_forms_parse() {
    let accepted = [
        // absolute, at-code predicates (incl. specialised/dotted codes)
        "exists /data[at0001.1]/items[at0001-2]",
        // position + meaning predicates ("legal for cADL structures")
        "/data[at0001]/items[1]/value/magnitude > 0",
        "/data[at0001]/items[systolic]/value/magnitude > 0",
        // relative paths: multi-segment and the single-segment yacc form
        "items[at0001]/value/magnitude > 0",
        "exists items[at0001]",
        // movable path patterns (leading '//')
        "exists //items[at0001]",
        "//items[at0001]/value/magnitude > 0",
    ];
    for invariant in accepted {
        let text = archetype_with_invariant(invariant);
        assert!(
            parse_artefact(&text, Dialect::Adl14,).is_ok(),
            "chapter path form must parse: {invariant}"
        );
    }
}
