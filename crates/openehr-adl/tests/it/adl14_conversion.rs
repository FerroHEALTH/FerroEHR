// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ADL 1.4 → ADL 2 conversion — the paired `upgrade_from_14` corpus is the
//! oracle. Each 1.4 `.adl` source is converted and compared, structurally,
//! against its expected ADL2 `.adls`.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion — the whole `adl14` pipeline
//! is our own design (archie is prior art only). The assertions here are pinned
//! by the fixtures, not by a spec clause.
//!
//! Comparison scope (documented tolerances — every one justified):
//! - **Definition tree**: compared by full structural (`PartialEq`) equality of
//!   the `C_COMPLEX_OBJECT` — the load-bearing conversion output.
//! - **Terminology**: compared by the SET of term-definition codes per language,
//!   the value sets (id + members), and the term bindings (code → URI). Term
//!   RUBRIC TEXT is deliberately NOT compared: a synthesised at-code minted for
//!   an external code (e.g. `openehr::524`) carries the code as a placeholder
//!   rubric, whereas the fixture carries the human name ("initial") resolved
//!   against the external openEHR terminology — a rendering nicety that needs
//!   `openehr-term` resolution (a TODO in `convert.rs`), not a structural
//!   property.
//! - **Description / meta prose** (copyright hoist, `other_details`
//!   hoisting/splitting, per-language synthesised rubrics) is NOT compared:
//!   `openehr-adl`'s assembler drops unknown 1.4 `details` keys (e.g. inline
//!   `copyright`) before the converter sees them, so this is not recoverable
//!   in the current front end (TODO in `convert.rs`). It does not affect the
//!   constraint model or validity.
//!
//! Three pairs are ADJUDICATED (converted + idempotent, but not a full
//! structural match) — the remaining work is specialisation-aware conversion,
//! called out per case below and reported to the orchestrator.
#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use openehr_adl::adl14::convert::{ConvertConfig, convert, parse_and_convert};
use openehr_adl::adl14::differ::differentiate;
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetype;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::terminology::archetype_terminology::ArchetypeTerminology;

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/corpus/adl2-reference/upgrade/upgrade_from_14")
}

fn read(name: &str) -> String {
    std::fs::read_to_string(dir().join(name)).unwrap_or_else(|e| panic!("read {name}: {e}"))
}

fn data(a: &Archetype) -> (&CComplexObject, &ArchetypeTerminology) {
    match a {
        Archetype::AuthoredArchetype(b) => match b.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => (&d.definition, &d.terminology),
            _ => panic!("not a plain authored archetype"),
        },
        Archetype::TemplateOverlay(_) => panic!("template overlay"),
    }
}

/// Set of term-definition codes per language.
fn code_sets(t: &ArchetypeTerminology) -> BTreeMap<String, BTreeSet<String>> {
    t.term_definitions
        .iter()
        .map(|(lang, terms)| (lang.clone(), terms.keys().cloned().collect()))
        .collect()
}

fn binding_map(t: &ArchetypeTerminology) -> BTreeMap<String, BTreeMap<String, String>> {
    t.term_bindings.clone().unwrap_or_default()
}

fn value_sets(t: &ArchetypeTerminology) -> BTreeMap<String, Vec<String>> {
    t.value_sets
        .clone()
        .unwrap_or_default()
        .into_iter()
        .map(|(k, v)| (k, v.members.into_vec()))
        .collect()
}

/// Convert a source (running the differ against a converted parent for a
/// specialised source), returning the converted archetype.
fn convert_pair(adl: &str, parent: Option<&str>) -> Archetype {
    let cfg = ConvertConfig::default();
    let mut log = ConversionLog::new();
    let mut got = parse_and_convert(&read(adl), &cfg, &mut log).expect("convert");
    if let Some(p) = parent {
        let parent_conv =
            parse_and_convert(&read(p), &cfg, &mut ConversionLog::new()).expect("convert parent");
        differentiate(&mut got, &parent_conv);
    }
    got
}

/// Assert full structural equality (definition + terminology code structure)
/// against the expected `.adls`.
fn assert_structural_match(adl: &str, adls: &str, parent: Option<&str>) {
    let got = convert_pair(adl, parent);
    let exp = parse_artefact(&read(adls), Dialect::Adl2).expect("parse expected .adls");
    let (gd, gt) = data(&got);
    let (ed, et) = data(&exp);
    assert_eq!(gd, ed, "{adl}: definition tree mismatch vs {adls}");
    assert_eq!(
        code_sets(gt),
        code_sets(et),
        "{adl}: term-definition code set mismatch vs {adls}"
    );
    assert_eq!(value_sets(gt), value_sets(et), "{adl}: value-set mismatch");
    assert_eq!(
        binding_map(gt),
        binding_map(et),
        "{adl}: term-binding mismatch"
    );

    // The converted output validates clean (phase 1).
    let issues = openehr_adl::validate::validate_integrity(&got, None);
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == openehr_adl::validate::catalogue::Severity::Error)
        .map(|i| i.code.mnemonic())
        .collect();
    assert!(
        errors.is_empty(),
        "{adl}: converted output failed validation: {errors:?}"
    );
}

/// Idempotency: re-converting the same source with the *same* log yields an
/// identical definition (the conversion log makes synthesised codes stable).
fn assert_idempotent(adl: &str, parent: Option<&str>) {
    let cfg = ConvertConfig::default();
    let mut log = ConversionLog::new();
    let first = parse_and_convert(&read(adl), &cfg, &mut log).expect("first convert");
    // Re-run consulting the SAME log (re-parse the source afresh).
    let reparsed = parse_artefact(&read(adl), Dialect::Adl14).expect("reparse");
    let second = convert(&reparsed, &cfg, &mut log).expect("second convert");
    let (fd, _) = data(&first);
    let (sd, _) = data(&second);
    assert_eq!(fd, sd, "{adl}: conversion is not idempotent under the log");
    let _ = parent;
}

// ── the six exact-match pairs ────────────────────────────────────────────────

#[test]
fn exclusion_converts_exactly() {
    assert_structural_match(
        "openEHR-EHR-EVALUATION.exclusion.v1.adl",
        "openEHR-EHR-EVALUATION.exclusion.v1.0.0.adls",
        None,
    );
    assert_idempotent("openEHR-EHR-EVALUATION.exclusion.v1.adl", None);
}

#[test]
fn id_codes_as_at_codes_converts_exactly() {
    // at-codes used as BOTH node ids and values split into id-codes + at-codes;
    // an external `openehr::524` becomes a synthesised at-code + a
    // `http://openehr.org/id/524` binding.
    assert_structural_match(
        "openEHR-EHR-ACTION.id_codes_as_at_codes.v1.adl",
        "openEHR-EHR-ACTION.id_codes_as_at_codes.v1.0.0.adls",
        None,
    );
    assert_idempotent("openEHR-EHR-ACTION.id_codes_as_at_codes.v1.adl", None);
}

#[test]
fn inherit_unchanged_parent_converts_exactly() {
    // A local code LIST becomes a synthesised `ac` value set; `@ internal @`
    // node terms are dropped.
    assert_structural_match(
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_parent.v1.adl",
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_parent.v1.0.0.adls",
        None,
    );
    assert_idempotent(
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_parent.v1.adl",
        None,
    );
}

#[test]
fn test_regex_converts_exactly() {
    // Version from `other_details["revision"]` (`1.1.0`); a slot + archetype-id
    // regex passes through; `occurrences {0..*}` is elided.
    assert_structural_match(
        "openEHR-EHR-OBSERVATION.test_regex.v1.adl",
        "openEHR-EHR-OBSERVATION.test_regex.v1.1.0.adls",
        None,
    );
    assert_idempotent("openEHR-EHR-OBSERVATION.test_regex.v1.adl", None);
}

#[test]
fn upgrade_parent_converts_exactly() {
    // Large multi-language source; `@ internal @` terms dropped across every
    // language (incl. the `*@ internal @(en)` untranslated form).
    assert_structural_match(
        "openEHR-EHR-OBSERVATION.upgrade_parent.v1.adl",
        "openEHR-EHR-OBSERVATION.upgrade_parent.v1.0.0.adls",
        None,
    );
    assert_idempotent("openEHR-EHR-OBSERVATION.upgrade_parent.v1.adl", None);
}

#[test]
fn inherit_unchanged_node_differentiates_exactly() {
    // A specialised child structurally identical to its parent: the differ
    // strips every inherited-unchanged node, leaving only the re-rubriced root.
    assert_structural_match(
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_node.v1.adl",
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_node.v1.0.0.adls",
        Some("openEHR-EHR-INSTRUCTION.inherit_unchanged_parent.v1.adl"),
    );
    assert_idempotent(
        "openEHR-EHR-INSTRUCTION.inherit_unchanged_node.v1.adl",
        None,
    );
}

// ── the three adjudicated pairs (converted + idempotent; full structural match
//    deferred — reported to the orchestrator) ──────────────────────────────────

/// `adl14_meta_data`: the source's `events cardinality matches {1..*; unordered}`
/// is byte-identical to `upgrade_parent`'s, yet the reference converter ELIDES
/// it here and KEEPS it there — the two fixtures contradict, so no deterministic
/// converter matches both. We keep any non-`{0..*}` cardinality (only the
/// RM-universal `{0..*}` default is elided; a precise RM-default-aware elision
/// needs the RM attribute model, which the converter does not consult), which
/// matches `upgrade_parent` exactly. This pair's definition is therefore
/// adjudicated: we assert it
/// converts, validates clean, and is idempotent.
#[test]
fn adl14_meta_data_converts_and_validates() {
    let got = convert_pair("openEHR-EHR-OBSERVATION.adl14_meta_data.adl", None);
    let issues = openehr_adl::validate::validate_integrity(&got, None);
    let errors: Vec<_> = issues
        .iter()
        .filter(|i| i.severity == openehr_adl::validate::catalogue::Severity::Error)
        .map(|i| i.code.mnemonic())
        .collect();
    assert!(errors.is_empty(), "adl14_meta_data validation: {errors:?}");
    assert_idempotent("openEHR-EHR-OBSERVATION.adl14_meta_data.adl", None);
}

/// `exclusion-adverse_reaction`: a specialised child whose expected output uses
/// ADL2 DIFFERENTIAL PATHS (`/data[id2]/items[id4.1]/value …`) and
/// parent-aligned value-node ids (`id5.1`/`id5`). Our differ strips
/// inherited-unchanged nodes but does not yet (a) collapse a
/// single-differential-child chain into a differential path, nor (b) align
/// synthesised value-node ids to the flat parent. Adjudicated: converts +
/// idempotent. TODO: differential-path emission + parent-aligned numbering.
#[test]
fn exclusion_adverse_reaction_converts() {
    let got = convert_pair(
        "openEHR-EHR-EVALUATION.exclusion-adverse_reaction.v1.adl",
        Some("openEHR-EHR-EVALUATION.exclusion.v1.adl"),
    );
    assert!(matches!(got, Archetype::AuthoredArchetype(_)));
    assert_idempotent(
        "openEHR-EHR-EVALUATION.exclusion-adverse_reaction.v1.adl",
        Some("openEHR-EHR-EVALUATION.exclusion.v1.adl"),
    );
}

/// `upgrade_add_use_nodes`: the richest specialised source — `0.`-namespace
/// synthesis (new-at-level id-/at-/ac-codes), parent-aligned numbering,
/// `use_node` id synthesis, and `C_DV_QUANTITY`→tuple with external-property
/// binding. Our base conversion handles the tuple + binding shape but not the
/// `0.`-space specialisation synthesis, so codes land at the wrong
/// specialisation level. Adjudicated: converts + idempotent. TODO: `0.`-space
/// specialisation-aware synthesis + parent-aligned numbering.
#[test]
fn upgrade_add_use_nodes_converts() {
    let got = convert_pair(
        "openEHR-EHR-OBSERVATION.upgrade_add_use_nodes.v1.adl",
        Some("openEHR-EHR-OBSERVATION.upgrade_parent.v1.adl"),
    );
    assert!(matches!(got, Archetype::AuthoredArchetype(_)));
    assert_idempotent(
        "openEHR-EHR-OBSERVATION.upgrade_add_use_nodes.v1.adl",
        Some("openEHR-EHR-OBSERVATION.upgrade_parent.v1.adl"),
    );
}

/// A 1.4 source that reuses one at-code across two sibling subtrees (legal in
/// 1.4 — node ids are only sibling-unique) converts with archetype-wide
/// unique node ids (`AOM2/master04.5` §Validity Rules: `C_OBJECT`, VCOSU): the
/// second occurrence is re-minted and its terminology cloned from the shared
/// 1.4 term.
#[test]
fn reused_node_codes_re_mint_archetype_wide_unique_ids() {
    let src = r#"archetype (adl_version=1.4)
    openEHR-EHR-CLUSTER.reused_codes.v1

concept
    [at0000]

language
    original_language = <[ISO_639-1::en]>

description
    original_author = <
        ["name"] = <"t">
    >
    details = <
        ["en"] = <
            language = <[ISO_639-1::en]>
            purpose = <"t">
        >
    >
    lifecycle_state = <"AuthorDraft">

definition
    CLUSTER[at0000] matches {
        items cardinality matches {0..*; unordered} matches {
            CLUSTER[at0001] matches {
                items cardinality matches {0..*; unordered} matches {
                    ELEMENT[at0004] matches {*}
                }
            }
            CLUSTER[at0002] matches {
                items cardinality matches {0..*; unordered} matches {
                    ELEMENT[at0004] matches {*}
                }
            }
        }
    }

ontology
    term_definitions = <
        ["en"] = <
            items = <
                ["at0000"] = <text = <"root">; description = <"root">>
                ["at0001"] = <text = <"left">; description = <"left">>
                ["at0002"] = <text = <"right">; description = <"right">>
                ["at0004"] = <text = <"shared">; description = <"shared">>
            >
        >
    >
"#;
    let cfg = ConvertConfig::default();
    let mut log = ConversionLog::new();
    let got = parse_and_convert(src, &cfg, &mut log).expect("convert");
    let issues = openehr_adl::validate::validate_integrity(&got, None);
    let errors: Vec<&str> = issues
        .iter()
        .filter(|i| i.severity == openehr_adl::validate::catalogue::Severity::Error)
        .map(|i| i.code.mnemonic())
        .collect();
    assert!(errors.is_empty(), "phase-1 errors: {errors:?}");

    // Both occurrences exist as DISTINCT node ids, each defined with the
    // shared rubric.
    let (_, terminology) = data(&got);
    let en = terminology
        .term_definitions
        .get("en")
        .expect("en definitions");
    let shared: Vec<&String> = en
        .iter()
        .filter(|(_, t)| t.text == "shared")
        .map(|(code, _)| code)
        .collect();
    assert_eq!(
        shared.len(),
        2,
        "the reused code re-mints a second defined node id: {en:?}"
    );
    assert!(!log.notes.is_empty(), "the re-mint is logged as provenance");
}

/// A specialised source emitted STANDALONE (no resolvable parent — the
/// flattened-OPT decomposition case) collapses to depth 0 under
/// `collapse_specialised_codes`: no dotted code survives anywhere, the root
/// is `id1` (VARCN), and phase 1 is clean — no VACSD/VASID (the archetype is
/// unspecialised) and no VATCD (all codes at level 0).
#[test]
fn standalone_specialised_source_collapses_to_depth_zero() {
    let src = r#"archetype (adl_version=1.4)
    openEHR-EHR-CLUSTER.collapse-parent.v1

concept
    [at0000.1]

language
    original_language = <[ISO_639-1::en]>

description
    original_author = <
        ["name"] = <"t">
    >
    details = <
        ["en"] = <
            language = <[ISO_639-1::en]>
            purpose = <"t">
        >
    >
    lifecycle_state = <"AuthorDraft">

definition
    CLUSTER[at0000.1] matches {
        items cardinality matches {0..*; unordered} matches {
            ELEMENT[at0001] matches {
                value matches {
                    DV_CODED_TEXT matches {
                        defining_code matches {[local::at0.32, at0.33]}
                    }
                }
            }
            ELEMENT[at0002.1] matches {*}
        }
    }

ontology
    term_definitions = <
        ["en"] = <
            items = <
                ["at0000.1"] = <text = <"root">; description = <"root">>
                ["at0001"] = <text = <"coded">; description = <"coded">>
                ["at0002.1"] = <text = <"added">; description = <"added">>
                ["at0.32"] = <text = <"a">; description = <"a">>
                ["at0.33"] = <text = <"b">; description = <"b">>
            >
        >
    >
"#;
    let cfg = ConvertConfig {
        collapse_specialised_codes: true,
        ..ConvertConfig::default()
    };
    let mut log = ConversionLog::new();
    let got = parse_and_convert(src, &cfg, &mut log).expect("convert");
    let issues = openehr_adl::validate::validate_integrity(&got, None);
    let errors: Vec<&str> = issues
        .iter()
        .filter(|i| i.severity == openehr_adl::validate::catalogue::Severity::Error)
        .map(|i| i.code.mnemonic())
        .collect();
    assert!(errors.is_empty(), "phase-1 errors: {errors:?}");

    // Depth 0 everywhere: no dotted code remains in USE — as a bracketed
    // node id/constraint (`[at0.32]`) or a terminology key (`["at0.32"]`).
    // The conversion_details provenance deliberately NAMES the original
    // dotted codes, so a bare-substring scan would false-positive there.
    let printed = openehr_adl::print::print(&got).expect("print the converted artefact");
    for token in [
        "[id1.", "[id3.", "[at0.", "[\"at0.", "[\"id1.", ".1]", ".1\"]",
    ] {
        assert!(
            !printed.contains(token),
            "dotted code survived the collapse ({token:?}):\n{printed}"
        );
    }
    let (_, terminology) = data(&got);
    assert_eq!(terminology.concept_code, "id1", "root collapses to id1");
    assert!(
        log.notes.iter().any(|n| n.contains("collapsed")),
        "collapse remaps are logged as provenance: {:?}",
        log.notes
    );
    // The printed ADL2 re-parses (the standalone artefact is well-formed).
    parse_artefact(&printed, Dialect::Adl2)
        .unwrap_or_else(|e| panic!("printed ADL2 does not re-parse: {e:?}\n{printed}"));
}

/// The ADL 1.4 default `occurrences` is WRITTEN OUT on container children by the
/// conversion, because the two formalisms read an absent `occurrences`
/// differently: ADL 1.4 means `{1..1}` (`ADL1.4/master05-cadl.adoc` §Occurrences
/// L316) while ADL 2 infers `0..cardinality.upper`
/// (`AOM2/master04.5-constraint_model-class_definitions.adoc` §Occurrences
/// inferencing rules). Leaving it implicit would widen "exactly once" to "none to
/// many". A `use_node` proxy stays unstated — master05 L515 gives it the
/// referenced node's occurrences.
#[test]
fn conversion_writes_out_the_14_default_occurrences_on_container_children() {
    let src = "\
archetype (adl_version=1.4)
\topenEHR-EHR-CLUSTER.default_occurrences.v1

concept
\t[at0000]

language
\toriginal_language = <[ISO_639-1::en]>

description
\tlifecycle_state = <\"AuthorDraft\">

definition
\tCLUSTER[at0000] matches {
\t\titems cardinality matches {1..*; unordered} matches {
\t\t\tELEMENT[at0001] matches {*}
\t\t\tELEMENT[at0002] occurrences matches {0..2} matches {*}
\t\t\tuse_node ELEMENT /items[at0002]
\t\t}
\t}

ontology
\tterm_definitions = <
\t\t[\"en\"] = <
\t\t\titems = <
\t\t\t\t[\"at0000\"] = <text=<\"\"> description=<\"\">>
\t\t\t\t[\"at0001\"] = <text=<\"\"> description=<\"\">>
\t\t\t\t[\"at0002\"] = <text=<\"\"> description=<\"\">>
\t\t\t>
\t\t>
\t>
";
    let mut log = ConversionLog::new();
    let converted = parse_and_convert(src, &ConvertConfig::default(), &mut log).expect("converts");
    let (definition, _) = data(&converted);
    let items = openehr_adl::aom::access::complex_attributes(definition)
        .iter()
        .find(|a| a.rm_attribute_name == "items")
        .expect("the items attribute")
        .clone();

    // at0001 stated nothing in 1.4 ⇒ explicit {1..1} in the ADL2 output.
    let first = occurrences_of(&items.children.as_deref().unwrap_or_default()[0])
        .expect("materialised occurrences");
    assert_eq!((first.lower, first.upper), (Some(1), Some(1)));
    // at0002 stated {0..2} ⇒ carried through unchanged.
    let second = occurrences_of(&items.children.as_deref().unwrap_or_default()[1])
        .expect("stated occurrences");
    assert_eq!((second.lower, second.upper), (Some(0), Some(2)));
    // The use_node keeps no occurrences of its own (master05 L515).
    assert!(occurrences_of(&items.children.as_deref().unwrap_or_default()[2]).is_none());
}

/// The `occurrences` of a converted child object, if it carries one.
fn occurrences_of(
    obj: &openehr_am::v2_4::aom2::constraint_model::c_object::CObject,
) -> Option<&openehr_base::prelude::MultiplicityInterval> {
    use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
    match obj {
        CObject::CComplexObject(CComplexObject::CComplexObject(d)) => d.occurrences.as_ref(),
        CObject::CComplexObject(CComplexObject::CArchetypeRoot(r)) => r.occurrences.as_ref(),
        CObject::CComplexObjectProxy(p) => p.occurrences.as_ref(),
        CObject::ArchetypeSlot(s) => s.occurrences.as_ref(),
        _ => None,
    }
}

// ── App.B extended meta-data (`description/other_details`) ───────────────────

/// The archetype data of a converted plain authored archetype.
fn authored(
    a: &Archetype,
) -> &openehr_am::v2_4::aom2::archetype::authored_archetype::AuthoredArchetypeData {
    match a {
        Archetype::AuthoredArchetype(b) => match b.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => d,
            _ => panic!("not a plain authored archetype"),
        },
        Archetype::TemplateOverlay(_) => panic!("template overlay"),
    }
}

/// A minimal 1.4 archetype carrying `other_details_block` verbatim.
fn meta_data_source(other_details_block: &str) -> String {
    format!(
        "\
archetype (adl_version=1.4)
    openEHR-EHR-CLUSTER.app_b_meta_data.v1

concept
    [at0000]

language
    original_language = <[ISO_639-1::en]>

description
    lifecycle_state = <\"AuthorDraft\">
{other_details_block}

definition
    CLUSTER[at0000] matches {{
        items cardinality matches {{0..*; unordered}} matches {{
            ELEMENT[at0001] matches {{*}}
        }}
    }}

ontology
    term_definitions = <
        [\"en\"] = <
            items = <
                [\"at0000\"] = <text=<\"root\"> description=<\"root\">>
                [\"at0001\"] = <text=<\"item\"> description=<\"item\">>
            >
        >
    >
"
    )
}

fn convert_source(src: &str) -> Archetype {
    let mut log = ConversionLog::new();
    parse_and_convert(src, &ConvertConfig::default(), &mut log).expect("converts")
}

/// A 1.4 source carrying the appendix's §Standardised Items example block
/// verbatim (`ADL1.4/masterAppB-extended_metadata.adoc`).
fn app_b_standardised_items_source() -> String {
    meta_data_source(
        r#"    other_details = <
        ["licence"] = <"This work is licensed under the Creative Commons Attribution-ShareAlike 3.0 License. To view a copy of this license, visit http://creativecommons.org/licenses/by-sa/3.0/.">
        ["original_namespace"] = <"au.gov.nehta">
        ["original_publisher"] = <"NEHTA CTI Team, National E-Health Transition Authority <clinicalinfo@nehta.gov.au>">
        ["custodian_organisation"] = <"openEHR Foundation <http://www.openEHR.org>">
        ["custodian_namespace"] = <"org.openehr">
        ["build_uid"] = <"3076af96-e1dd-4f9b-abf2-23913fcf52b1">
        ["revision"] = <"0.0.1-alpha">
        ["references"] = <"
            O'Brien E, Asmar R, Beilin L, et al. European Society of Hypertension recommendations for conventional, ambulatory and home blood pressure measurement. Journal of Hypertension. 2003; 21(5):821-848. Available from: http://www.ncbi.nlm.nih.gov/pubmed/12714851
            Perloff D, Grim C, Flack J, Frohlich ED, Hill M, McDonald M, Morgenstern BZ. Human blood pressure determination by sphygmomanometry. Circulation. 1993; 88(5):2460. Available from: http://circ.ahajournals.org/cgi/reprint/88/5/2460
        ">
        ["ip_acknowledgements"] = <"
            Content from LOINC® is copyright © 1995 Regenstrief Institute, Inc. and the LOINC Committee, and available at no cost under the license at http://loinc.org/terms-of-use.
            Content from SNOMED CT® is copyright © 2007 IHTSDO <ihtsdo.org>.
        ">
    >"#,
    )
}

/// The appendix's §Standardised Items example block converts to its AOM2
/// homes: the five governance strings transfer verbatim, `build_uid` parses
/// into the archetype's `build_uid`, and `revision` still sets the release
/// version.
#[test]
fn app_b_standardised_items_convert_to_their_aom2_homes() {
    let converted = convert_source(&app_b_standardised_items_source());
    let archetype = authored(&converted);
    let desc = archetype.description.as_ref().expect("a description");

    // The five governance strings transfer VERBATIM — the "name <URN>" shapes
    // are display conventions, never decomposed.
    assert_eq!(desc.original_namespace.as_deref(), Some("au.gov.nehta"));
    assert_eq!(
        desc.original_publisher.as_deref(),
        Some("NEHTA CTI Team, National E-Health Transition Authority <clinicalinfo@nehta.gov.au>")
    );
    assert_eq!(desc.custodian_namespace.as_deref(), Some("org.openehr"));
    assert_eq!(
        desc.custodian_organisation.as_deref(),
        Some("openEHR Foundation <http://www.openEHR.org>")
    );
    assert_eq!(
        desc.licence.as_deref(),
        Some(
            "This work is licensed under the Creative Commons Attribution-ShareAlike 3.0 License. To view a copy of this license, visit http://creativecommons.org/licenses/by-sa/3.0/."
        )
    );

    // `build_uid` is a GUID string → the archetype's typed `build_uid`.
    assert_eq!(
        *archetype.build_uid.value(),
        uuid::Uuid::parse_str("3076af96-e1dd-4f9b-abf2-23913fcf52b1").expect("a valid GUID")
    );

    // `revision` still drives the release version (unchanged behaviour).
    assert_eq!(archetype.archetype_id.release_version, "0.0.1");
}

/// `references` and `ip_acknowledgements` are "string with one LF (`\n`)
/// terminated line for each reference … Intervening LFs and leading and
/// trailing whitespace may be added for clarity, to be stripped on conversion
/// to ADL2" (`ADL1.4/masterAppB-extended_metadata.adoc` §Standardised Items):
/// one keyed entry per non-blank line, trimmed. Every converted key is
/// consumed from `other_details`, and the result prints + re-parses as ADL2.
#[test]
fn app_b_reference_lists_split_on_lf_into_keyed_entries() {
    let converted = convert_source(&app_b_standardised_items_source());
    let desc = authored(&converted)
        .description
        .as_ref()
        .expect("a description");

    let references = desc.references.as_ref().expect("references");
    assert_eq!(
        references.keys().collect::<Vec<_>>(),
        vec!["1", "2"],
        "1-based ordinals in source line order"
    );
    assert_eq!(
        references.get("1").map(String::as_str),
        Some(
            "O'Brien E, Asmar R, Beilin L, et al. European Society of Hypertension recommendations for conventional, ambulatory and home blood pressure measurement. Journal of Hypertension. 2003; 21(5):821-848. Available from: http://www.ncbi.nlm.nih.gov/pubmed/12714851"
        )
    );
    assert_eq!(
        references.get("2").map(String::as_str),
        Some(
            "Perloff D, Grim C, Flack J, Frohlich ED, Hill M, McDonald M, Morgenstern BZ. Human blood pressure determination by sphygmomanometry. Circulation. 1993; 88(5):2460. Available from: http://circ.ahajournals.org/cgi/reprint/88/5/2460"
        )
    );
    let ip = desc
        .ip_acknowledgements
        .as_ref()
        .expect("ip_acknowledgements");
    assert_eq!(ip.keys().collect::<Vec<_>>(), vec!["1", "2"]);
    assert_eq!(
        ip.get("1").map(String::as_str),
        Some(
            "Content from LOINC® is copyright © 1995 Regenstrief Institute, Inc. and the LOINC Committee, and available at no cost under the license at http://loinc.org/terms-of-use."
        )
    );
    assert_eq!(
        ip.get("2").map(String::as_str),
        Some("Content from SNOMED CT® is copyright © 2007 IHTSDO <ihtsdo.org>.")
    );

    // Every converted key is consumed.
    let left: Vec<&String> = desc
        .other_details
        .as_ref()
        .map(|o| o.keys().collect())
        .unwrap_or_default();
    assert!(
        left.is_empty(),
        "every standardised item is consumed from other_details: {left:?}"
    );

    // The converted homes are visible in the printed ADL2, which re-parses.
    let printed = openehr_adl::print::print(&converted).expect("print the converted artefact");
    for token in [
        "build_uid=3076af96-e1dd-4f9b-abf2-23913fcf52b1",
        "original_namespace = <\"au.gov.nehta\">",
        "custodian_namespace = <\"org.openehr\">",
        "references = <",
        "ip_acknowledgements = <",
    ] {
        assert!(
            printed.contains(token),
            "printed ADL2 is missing {token:?}:\n{printed}"
        );
    }
    assert!(
        !printed.contains("other_details"),
        "no other_details section survives:\n{printed}"
    );
    parse_artefact(&printed, Dialect::Adl2)
        .unwrap_or_else(|e| panic!("printed ADL2 does not re-parse: {e:?}\n{printed}"));
}

/// The appendix's §Other Items example block passes through untouched: those
/// names are reserved/display-only ("CKM does nothing with this, except
/// display") and no conversion is mandated for them.
#[test]
fn app_b_other_items_pass_through_other_details_unchanged() {
    let src = meta_data_source(
        r#"    other_details = <
        ["review_date"] = <"2014-06-10">
        ["current_contact"] = <"Ian McNicoll, freshEHR Clinical Informatics Ltd <ian.mcnicoll@freshehr.com>">
        ["responsible_organisation"] = <"Nehta">
        ["MD5-CAM-1.0.1"] = <"C5016B71B55DBDCBCAA8531CC1A982E3">
    >"#,
    );
    let converted = convert_source(&src);
    let desc = authored(&converted)
        .description
        .as_ref()
        .expect("a description");
    let other = desc.other_details.as_ref().expect("other_details");
    assert_eq!(
        other.get("review_date").map(String::as_str),
        Some("2014-06-10")
    );
    assert_eq!(
        other.get("current_contact").map(String::as_str),
        Some("Ian McNicoll, freshEHR Clinical Informatics Ltd <ian.mcnicoll@freshehr.com>")
    );
    assert_eq!(
        other.get("responsible_organisation").map(String::as_str),
        Some("Nehta")
    );
    assert_eq!(
        other.get("MD5-CAM-1.0.1").map(String::as_str),
        Some("C5016B71B55DBDCBCAA8531CC1A982E3")
    );
    assert_eq!(other.len(), 4, "nothing added, nothing removed");
}

/// A `build_uid` that violates the table's "Guid string" syntax is never
/// guessed at: it stays in `other_details` verbatim, the archetype keeps its
/// nil `build_uid`, and the remaining standardised items still convert.
#[test]
fn malformed_build_uid_stays_in_other_details() {
    let src = meta_data_source(
        r#"    other_details = <
        ["build_uid"] = <"not-a-guid">
        ["custodian_namespace"] = <"org.openehr">
    >"#,
    );
    let converted = convert_source(&src);
    let archetype = authored(&converted);
    assert!(
        archetype.build_uid.value().is_nil(),
        "a non-GUID build_uid is not converted"
    );
    let other = archetype
        .description
        .as_ref()
        .expect("a description")
        .other_details
        .as_ref()
        .expect("other_details");
    assert_eq!(
        other.get("build_uid").map(String::as_str),
        Some("not-a-guid")
    );
    assert_eq!(other.len(), 1, "only the unconvertible value is left");
    assert_eq!(
        archetype
            .description
            .as_ref()
            .and_then(|d| d.custodian_namespace.as_deref()),
        Some("org.openehr"),
        "the other standardised items still convert"
    );
}
