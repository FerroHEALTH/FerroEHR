// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ADL 1.4 phase-1 validation subset
//! ([`openehr_adl::validate::validate_source_integrity`] in
//! [`openehr_adl::parse::Dialect::Adl14`]).
//!
//! A 1.4 source is judged **as 1.4** (never post-conversion): the checks that
//! correspond to an ADL 1.4 / AOM 1.4 standalone rule run; the AOM2-only rules
//! that would false-reject a valid 1.4 archetype (VARAV/VARRV/VCOID-strict/
//! VATCV-form/VCOSU-archetype-wide/WOUC) are suppressed. Oracle:
//! `docs/specs/openehr/AM/docs/ADL1.4/master08-adl.adoc` §Validity Rules +
//! `docs/specs/openehr/AM/docs/AOM1.4/` class invariants.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::parse::Dialect;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::validate_source_integrity;

/// A minimal, valid standalone ADL 1.4 archetype (`adl_version=1.4`, no
/// `rm_release`, at-code node ids, `ontology` keyword) — the exact shape a 1.4
/// upload carries.
const VALID_14: &str = "\u{feff}archetype (adl_version=1.4)
\topenEHR-EHR-OBSERVATION.test14.v1

concept
\t[at0000]\t-- Test 14
language
\toriginal_language = <[ISO_639-1::en]>
description
\tlifecycle_state = <\"Initial\">
\tdetails = <
\t\t[\"en\"] = <
\t\t\tlanguage = <[ISO_639-1::en]>
\t\t\tpurpose = <\"testing\">
\t\t>
\t>
definition
\tOBSERVATION[at0000] matches {
\t\tdata matches {
\t\t\tHISTORY matches {*}
\t\t}
\t}
ontology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <
\t\t\t\t\ttext = <\"Test 14\">
\t\t\t\t\tdescription = <\"A test archetype.\">
\t\t\t\t>
\t\t\t>
\t\t>
\t>
";

fn errors(src: &str) -> Vec<String> {
    let issues = validate_source_integrity(src, Dialect::Adl14, None).expect("1.4 source parses");
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code.mnemonic().to_owned())
        .collect()
}

#[test]
fn valid_14_archetype_is_clean() {
    // A well-formed 1.4 archetype validates clean — and specifically does NOT
    // raise VARAV (adl_version=1.4) or VARRV (no rm_release), the AOM2-only
    // rules the 1.4 subset suppresses.
    let errs = errors(VALID_14);
    assert!(errs.is_empty(), "expected clean, got {errs:?}");
    assert!(
        !errs.contains(&"VARAV".to_owned()) && !errs.contains(&"VARRV".to_owned()),
        "the adl_version/rm_release 3-part rules must not apply to 1.4"
    );
}

#[test]
fn definition_type_mismatch_raises_vardt() {
    // ADL1.4 master08 §Validity Rules VARDT: the topmost definition typename
    // must match the RM class of the archetype id. Corresponds to the engine's
    // VARDT (a shared ADL 1.4 / AOM2 rule) — it still fires in the 1.4 subset.
    let bad = VALID_14.replace("OBSERVATION[at0000] matches", "CLUSTER[at0000] matches");
    assert!(
        errors(&bad).contains(&"VARDT".to_owned()),
        "a definition type not matching the id class must raise VARDT"
    );
}

#[test]
fn undefined_node_code_raises_vatdf() {
    // ADL1.4 master08 §Validity Rules VATDF: every at-code used as a node
    // identifier must be defined in the ontology. Add a data node with an
    // at-code the ontology does not define.
    let bad = VALID_14.replace(
        "HISTORY matches {*}",
        "HISTORY[at0001] matches {*}", // at0001 is not in term_definitions
    );
    assert!(
        errors(&bad).contains(&"VATDF".to_owned()),
        "an undefined node-id at-code must raise VATDF, got {:?}",
        errors(&bad)
    );
}

#[test]
fn unparseable_source_is_a_parse_error() {
    assert!(validate_source_integrity("this is not an archetype", Dialect::Adl14, None).is_err());
}

// ── the one 1.4 rule that needs a reference model: VUNT ──────────────────────

/// A 1.4 archetype whose `use_node` names `rm_type` and points at an
/// `ELEMENT[at0002]` node. `master05-cadl.adoc` §Internal References L510-513:
/// the type named must be the same as, or a super-type of, the referenced
/// node's type.
fn use_node_archetype(rm_type: &str) -> String {
    format!(
        "archetype (adl_version=1.4)
\topenEHR-EHR-CLUSTER.use_node14.v1

concept
\t[at0000]\t-- Use node 14
language
\toriginal_language = <[ISO_639-1::en]>
description
\toriginal_author = <
\t\t[\"name\"] = <\"FerroEHR tests\">
\t>
\tdetails = <
\t\t[\"en\"] = <
\t\t\tlanguage = <[ISO_639-1::en]>
\t\t\tpurpose = <\"Internal-reference validity fixture.\">
\t\t>
\t>
\tlifecycle_state = <\"Initial\">
definition
\tCLUSTER[at0000] matches {{
\t\titems cardinality matches {{0..*}} matches {{
\t\t\tCLUSTER[at0001] occurrences matches {{0..1}} matches {{
\t\t\t\titems cardinality matches {{0..*}} matches {{
\t\t\t\t\tELEMENT[at0002] occurrences matches {{0..1}} matches {{
\t\t\t\t\t\tvalue matches {{
\t\t\t\t\t\t\tDV_TEXT matches {{*}}
\t\t\t\t\t\t}}
\t\t\t\t\t}}
\t\t\t\t}}
\t\t\t}}
\t\t\tuse_node {rm_type}[at0003] occurrences matches {{0..1}} /items[at0001]/items[at0002]
\t\t}}
\t}}
ontology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <
\t\t\t\t\ttext = <\"Use node 14\">
\t\t\t\t\tdescription = <\"A 1.4 archetype with an internal reference.\">
\t\t\t\t>
\t\t\t\t[\"at0001\"] = <
\t\t\t\t\ttext = <\"Group\">
\t\t\t\t\tdescription = <\"A group of items.\">
\t\t\t\t>
\t\t\t\t[\"at0002\"] = <
\t\t\t\t\ttext = <\"Item\">
\t\t\t\t\tdescription = <\"The referenced item.\">
\t\t\t\t>
\t\t\t\t[\"at0003\"] = <
\t\t\t\t\ttext = <\"Re-used item\">
\t\t\t\t\tdescription = <\"The internal reference.\">
\t\t\t\t>
\t\t\t>
\t\t>
\t>
"
    )
}

fn adl14_codes(src: &str) -> Vec<String> {
    let issues = openehr_adl::validate::validate_adl14_source(
        src,
        &openehr_adl::validate::rm::ProductionRmModel,
    )
    .expect("1.4 source parses");
    issues
        .iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| i.code.mnemonic().to_owned())
        .collect()
}

#[test]
fn use_node_type_conformance_is_reachable_for_a_14_source() {
    // VUNT is a rule of the ADL 1.4 formalism itself (`master05-cadl.adoc`
    // §Internal References L512-513), so a 1.4 source must be able to fail it —
    // which the phase-1-only entry point can never report, because deciding
    // super-type-hood is "according to the reference model".
    let bad = use_node_archetype("CLUSTER"); // CLUSTER is not a super-type of ELEMENT
    assert!(
        adl14_codes(&bad).contains(&"VUNT".to_owned()),
        "a use_node naming a non-super-type must raise VUNT, got {:?}",
        adl14_codes(&bad)
    );
    // The phase-1-only entry point is the contrast: it cannot see the rule.
    assert!(
        !errors(&bad).contains(&"VUNT".to_owned()),
        "phase 1 has no reference model and must not claim VUNT"
    );
}

#[test]
fn use_node_naming_the_same_or_a_super_type_is_clean() {
    // The same type, and the super-type the chapter's own example relies on
    // ("a use_node reference to such a node can legally mention the parent
    // type", L510) — `ITEM` is the RM parent of `ELEMENT`.
    for ty in ["ELEMENT", "ITEM"] {
        let codes = adl14_codes(&use_node_archetype(ty));
        assert!(
            codes.is_empty(),
            "use_node {ty} must validate clean, got {codes:?}"
        );
    }
}

// ── VDFPT: path validity in the 1.4 definition section ───────────────────────

#[test]
fn use_node_target_that_does_not_resolve_raises_vdfpt() {
    // `ADL1.4/master08-adl.adoc` §Definition Section, VDFPT: any path mentioned
    // in the definition section must be valid with respect to its hierarchical
    // structure. The helper's resolving target is the accepting twin
    // (`use_node_naming_the_same_or_a_super_type_is_clean` proves it stays
    // clean); here the same archetype pointing at a node that does not exist
    // must be reported.
    let bad = use_node_archetype("ELEMENT").replace(
        "/items[at0001]/items[at0002]",
        "/items[at0001]/items[at0099]",
    );
    assert!(
        errors(&bad).contains(&"VDFPT".to_owned()),
        "a use_node target that does not resolve must raise VDFPT, got {:?}",
        errors(&bad)
    );
    // The full-catalogue entry reports it too.
    assert!(adl14_codes(&bad).contains(&"VDFPT".to_owned()));
}

#[test]
fn use_node_target_leaving_the_archetype_raises_vdfpt() {
    // A target whose first segment is not a constrained attribute of the root:
    // syntactically fine, structurally invalid per VDFPT.
    let bad = use_node_archetype("ELEMENT")
        .replace("/items[at0001]/items[at0002]", "/no_such_attr[at0002]");
    assert!(
        errors(&bad).contains(&"VDFPT".to_owned()),
        "a use_node path outside the definition structure must raise VDFPT, got {:?}",
        errors(&bad)
    );
}

// ── the RM resource-package meta-data rows (RM common ch.8) ──────────────────

/// The RM resource-package invariants bind a 1.4 source's meta-data
/// (`RM/docs/common/master08-resource_package.adoc` front-matter NOTE; the
/// class tables' §Invariants). Each row refuses through its invariant-named
/// code, and the valid fixture stays clean.
#[test]
fn resource_meta_rows_bind_a_14_source() {
    let valid = use_node_archetype("ELEMENT");
    assert!(adl14_codes(&valid).is_empty());

    // RESOURCE_DESCRIPTION.Details_valid: a description with no details.
    let no_details = valid.replace(
        "\tdetails = <\n\t\t[\"en\"] = <\n\t\t\tlanguage = <[ISO_639-1::en]>\n\t\t\tpurpose = <\"Internal-reference validity fixture.\">\n\t\t>\n\t>\n",
        "",
    );
    assert!(
        adl14_codes(&no_details).contains(&"RESOURCE_DESCRIPTION.Details_valid".to_owned()),
        "got {:?}",
        adl14_codes(&no_details)
    );

    // RESOURCE_DESCRIPTION_ITEM.Purpose_valid: an empty purpose is a
    // WARNING on a 1.4 source (`purpose = <"">` is endemic real-world 1.4
    // authoring; 1.4 tolerance is our own design) — named, never refused.
    let empty_purpose = valid.replace("Internal-reference validity fixture.", "");
    assert!(
        !adl14_codes(&empty_purpose)
            .contains(&"RESOURCE_DESCRIPTION_ITEM.Purpose_valid".to_owned()),
        "an empty 1.4 purpose must not be an Error"
    );
    let warning_codes: Vec<String> = openehr_adl::validate::validate_adl14_source(
        &empty_purpose,
        &openehr_adl::validate::rm::ProductionRmModel,
    )
    .expect("1.4 source parses")
    .iter()
    .filter(|i| i.severity == Severity::Warning)
    .map(|i| i.code.mnemonic().to_owned())
    .collect();
    assert!(
        warning_codes.contains(&"RESOURCE_DESCRIPTION_ITEM.Purpose_valid".to_owned()),
        "an empty 1.4 purpose must be a named Warning, got {warning_codes:?}"
    );

    // RESOURCE_DESCRIPTION.Original_author_valid: an empty author map is not
    // expressible in ODIN, so drop the section — assemble leaves it empty.
    let no_author = valid.replace(
        "\toriginal_author = <\n\t\t[\"name\"] = <\"FerroEHR tests\">\n\t>\n",
        "",
    );
    assert!(
        adl14_codes(&no_author).contains(&"RESOURCE_DESCRIPTION.Original_author_valid".to_owned())
    );

    // AUTHORED_RESOURCE.Translations_valid: a translation re-stating the
    // original language.
    let restated = valid.replace(
        "description\n\toriginal_author",
        "translations = <\n\t[\"en\"] = <\n\t\tlanguage = <[ISO_639-1::en]>\n\t\tauthor = <\n\t\t\t[\"name\"] = <\"FerroEHR tests\">\n\t\t>\n\t>\n>\ndescription\n\toriginal_author",
    );
    assert!(
        adl14_codes(&restated).contains(&"AUTHORED_RESOURCE.Translations_valid".to_owned()),
        "got {:?}",
        adl14_codes(&restated)
    );

    // AUTHORED_RESOURCE.Description_valid: a description detail in a language
    // that is neither the original nor a listed translation.
    let orphan_language = valid.replace(
        "description\n\toriginal_author",
        "translations = <\n\t[\"de\"] = <\n\t\tlanguage = <[ISO_639-1::de]>\n\t\tauthor = <\n\t\t\t[\"name\"] = <\"FerroEHR tests\">\n\t\t>\n\t>\n>\ndescription\n\toriginal_author",
    ).replace(
        "\t\t[\"en\"] = <\n\t\t\tlanguage = <[ISO_639-1::en]>\n\t\t\tpurpose = <\"Internal-reference validity fixture.\">\n\t\t>\n",
        "\t\t[\"en\"] = <\n\t\t\tlanguage = <[ISO_639-1::en]>\n\t\t\tpurpose = <\"Internal-reference validity fixture.\">\n\t\t>\n\t\t[\"nl\"] = <\n\t\t\tlanguage = <[ISO_639-1::nl]>\n\t\t\tpurpose = <\"Doel.\">\n\t\t>\n",
    );
    assert!(
        adl14_codes(&orphan_language).contains(&"AUTHORED_RESOURCE.Description_valid".to_owned()),
        "got {:?}",
        adl14_codes(&orphan_language)
    );
}
