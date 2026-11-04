// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The 1.4 → 2 conversion cases the openEHR Discourse thread "ADL2 generation
//! appears to be incorrect" raised against Archie, each run on a vendored
//! archetype: an at-code used as node id and value (#4963), a terminology
//! constraint with no codes (#4965), a specialisation introducing an undotted
//! code (#4966) and specialised slots through the differ (#4967).
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion, our own design; the ADL 2
//! output is judged by the AOM 2 validator and each case cites its rule.
#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use openehr_adl::adl14::convert::{ConvertConfig, ConvertError, parse_and_convert};
use openehr_adl::adl14::differ::differentiate;
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::adl14::specialised_codes::check_introduced_codes;
use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::print::print;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::validate_source;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::aom2::constraint_model::archetype_slot::ArchetypeSlot;
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;
use openehr_am::v2_4::aom2::constraint_model::c_primitive_object::CPrimitiveObject;

fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/archetypes")
}

fn ckm(name: &str) -> String {
    read(&corpus().join("ckm/adl14").join(name))
}

fn pack_2013(name: &str) -> String {
    read(&corpus().join("adl2/ckm-2013-12-09/cluster").join(name))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn converted(src: &str, log: &mut ConversionLog) -> Archetype {
    parse_and_convert(src, &ConvertConfig::default(), log).expect("the 1.4 source converts")
}

fn data(a: &Archetype) -> &AuthoredArchetypeData {
    match a {
        Archetype::AuthoredArchetype(b) => match b.as_ref() {
            AuthoredArchetype::AuthoredArchetype(d) => d,
            _ => panic!("not a plain authored archetype"),
        },
        Archetype::TemplateOverlay(_) => panic!("a template overlay"),
    }
}

/// The error-severity issues of an ADL 2 text, validated as an upload is.
fn errors(adl2: &str, repo: Option<&ArchetypeRepository>) -> Vec<String> {
    validate_source(adl2, repo, &ProductionRmModel, &NoTerminologyResolver)
        .expect("the printed ADL 2 parses")
        .into_iter()
        .filter(|i| i.severity == Severity::Error)
        .map(|i| format!("{} {} {:?}", i.code, i.message, i.path))
        .collect()
}

/// Every `C_TERMINOLOGY_CODE.constraint` under `node`, tuple rows included.
fn term_constraints(node: &CComplexObject, out: &mut Vec<String>) {
    let CComplexObject::CComplexObject(d) = node else {
        return;
    };
    for attr in d.attributes.iter().flatten() {
        for child in attr.children.iter().flatten() {
            match child {
                CObject::CTerminologyCode(tc) => out.push(tc.constraint.clone()),
                CObject::CComplexObject(cco) => term_constraints(cco, out),
                _ => {}
            }
        }
    }
    for tuple in d.attribute_tuples.iter().flatten() {
        for row in tuple.tuples.iter().flatten() {
            for member in &row.members {
                if let CPrimitiveObject::CTerminologyCode(tc) = member {
                    out.push(tc.constraint.clone());
                }
            }
        }
    }
}

/// The complex object with `node_id` anywhere under `node`.
fn find_node<'a>(node: &'a CComplexObject, node_id: &str) -> Option<&'a CComplexObject> {
    let CComplexObject::CComplexObject(d) = node else {
        return None;
    };
    if d.node_id == node_id {
        return Some(node);
    }
    d.attributes
        .iter()
        .flatten()
        .flat_map(|attr| attr.children.iter().flatten())
        .find_map(|child| match child {
            CObject::CComplexObject(cco) => find_node(cco, node_id),
            _ => None,
        })
}

/// Every archetype slot under `node`.
fn slots<'a>(node: &'a CComplexObject, out: &mut Vec<&'a ArchetypeSlot>) {
    let CComplexObject::CComplexObject(d) = node else {
        return;
    };
    for child in d
        .attributes
        .iter()
        .flatten()
        .flat_map(|a| a.children.iter().flatten())
    {
        match child {
            CObject::ArchetypeSlot(slot) => out.push(slot),
            CObject::CComplexObject(cco) => slots(cco, out),
            _ => {}
        }
    }
}

/// The ADL 2 codes a 1.4 depth-0 code becomes: the number one up, as node id
/// and as value code (the converter's numbering, fixture-pinned).
fn shifted(code_14: &str) -> [String; 2] {
    let n: u32 = code_14
        .strip_prefix("at")
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("a depth-0 1.4 at-code: {code_14}"));
    [format!("id{}", n + 1), format!("at{}", n + 1)]
}

/// The 1.4 code an ADL 2 depth-0 id- or at-code came from.
fn unshifted(code_2: &str) -> String {
    let n: u32 = code_2
        .strip_prefix("id")
        .or_else(|| code_2.strip_prefix("at"))
        .and_then(|n| n.parse().ok())
        .unwrap_or_else(|| panic!("a depth-0 ADL 2 code: {code_2}"));
    format!("at{:04}", n - 1)
}

/// `waterlow_score`'s `at0116` is an ELEMENT node id and the ordinal value the
/// node carries. ADL 2 keeps id-codes and at-codes apart
/// (`ADL2/master07.04-adl_basics.adoc` §Node Identifier Codes), so it becomes
/// the node `id117` and the value `at117`, each defined in every language
/// (VATDF, `AOM2/master03-archetype_package.adoc` §Validity Rules), and no
/// other term is lost or invented.
#[test]
fn waterlow_score_keeps_a_code_used_as_node_id_and_value() {
    let src = ckm("openEHR-EHR-OBSERVATION.waterlow_score.v0.adl");
    let source = parse_artefact(&src, Dialect::Adl14).expect("the 1.4 source parses");
    let source = data(&source);
    let mut log = ConversionLog::default();
    let out = converted(&src, &mut log);
    let target = data(&out);

    let node = find_node(&target.definition, "id117").expect("at0116's node is id117");
    let mut values = Vec::new();
    term_constraints(node, &mut values);
    assert_eq!(values, ["at117"], "the node's ordinal value is at117");

    let languages: BTreeSet<_> = source.terminology.term_definitions.keys().collect();
    assert_eq!(
        languages,
        target.terminology.term_definitions.keys().collect(),
        "every language survives"
    );
    // The converter drops `@ internal @` node terms (`adl14/convert.rs`,
    // fixture-pinned), so those are not expected to survive.
    let internal: BTreeSet<&String> = source.terminology.term_definitions
        [&source.terminology.original_language]
        .iter()
        .filter(|(_, t)| t.description.trim() == "@ internal @")
        .map(|(code, _)| code)
        .collect();
    assert!(!internal.contains(&"at0116".to_owned()));
    for (lang, terms_14) in &source.terminology.term_definitions {
        let terms_2 = &target.terminology.term_definitions[lang];
        let healthy = &terms_14["at0116"];
        for code in ["id117", "at117"] {
            let term = terms_2
                .get(code)
                .unwrap_or_else(|| panic!("{lang}: {code} is defined"));
            assert_eq!(
                (&term.text, &term.description),
                (&healthy.text, &healthy.description),
                "{lang}: {code} keeps at0116's term"
            );
        }
        for (code, term) in terms_14.iter().filter(|(c, _)| !internal.contains(c)) {
            let kept = shifted(code)
                .iter()
                .filter_map(|c| terms_2.get(c))
                .any(|t| t.text == term.text && t.description == term.description);
            assert!(kept, "{lang}: the term of {code} survives");
        }
        for (code, term) in terms_2 {
            let origin = terms_14
                .get(&unshifted(code))
                .unwrap_or_else(|| panic!("{lang}: {code} comes from a 1.4 code"));
            assert_eq!(origin.text, term.text, "{lang}: {code} keeps its text");
        }
    }
    assert!(
        log.value_sets.is_empty() && log.notes.is_empty(),
        "nothing was minted or dropped: {log:?}"
    );
    assert_eq!(
        errors(&print(&out).expect("the result prints"), None),
        Vec::<String>::new(),
        "the converted archetype is valid ADL 2"
    );
}

/// The ac-code each `[openEHR::]` or `[local::]` constraint became, in
/// document order, with the terminology entries checked. The log records
/// those under the `terminology::` signature.
fn open_value_set_codes(out: &Archetype, log: &ConversionLog) -> Vec<String> {
    let open: BTreeSet<&String> = log
        .value_sets
        .iter()
        .filter(|(signature, _)| signature.ends_with("::"))
        .map(|(_, code)| code)
        .collect();
    let target = data(out);
    let mut codes = Vec::new();
    term_constraints(&target.definition, &mut codes);
    codes.retain(|c| open.contains(c));
    for code in &codes {
        for (lang, terms) in &target.terminology.term_definitions {
            assert!(
                terms.contains_key(code),
                "{lang}: {code} is defined (VACDF)"
            );
        }
        assert!(
            target
                .terminology
                .value_sets
                .iter()
                .flatten()
                .all(|(id, _)| id != code),
            "{code} has no internal value set"
        );
        assert!(
            target
                .terminology
                .term_bindings
                .iter()
                .flatten()
                .all(|(_, bindings)| !bindings.contains_key(code)),
            "{code} has no binding"
        );
    }
    codes
}

/// A 1.4 constraint naming a terminology and no codes (`[openEHR::]`, the
/// shape openEHR/archie#896 crashes on) becomes an ac-code with its term
/// definition, no internal value set and no binding: no URI names a whole
/// terminology as a value set (`ADL2/master07.13-adl_terminology.adoc`
/// §`Value_sets` Sub-section), and a constraint with no codes constrains no
/// value (`ADL2/master04.5-cadl_primitive_types.adoc` §Soft Terminology
/// Constraint).
#[test]
fn a_terminology_constraint_with_no_codes_converts_to_an_open_value_set() {
    let src = ckm("openEHR-EHR-CLUSTER.waveform.v0.adl");
    assert_eq!(src.matches("[openEHR::]").count(), 1);
    let mut log = ConversionLog::default();
    let out = converted(&src, &mut log);
    assert_eq!(open_value_set_codes(&out, &log), ["ac1"]);
    assert!(
        log.notes
            .iter()
            .any(|n| n.contains("[openEHR::] names no codes")),
        "the conversion records the terminology: {:?}",
        log.notes
    );
    assert_eq!(
        errors(&print(&out).expect("the result prints"), None),
        Vec::<String>::new()
    );
}

/// Two such constraints share one ac-code when they name the same
/// terminology and get their own when they name different ones.
#[test]
fn constraints_with_no_codes_share_an_ac_code_per_terminology() {
    let src = ckm("openEHR-EHR-CLUSTER.microscopy_renal_biopsy_non_neoplastic.v0.adl");
    assert_eq!(src.matches("[openEHR::]").count(), 2);
    let mut log = ConversionLog::default();
    let same = converted(&src, &mut log);
    let codes = open_value_set_codes(&same, &log);
    assert_eq!(codes.len(), 2);
    assert_eq!(codes.first(), codes.last(), "one terminology, one ac-code");

    let mixed_src = src.replacen("[openEHR::]", "[local::]", 1);
    let mut log = ConversionLog::default();
    let mixed = converted(&mixed_src, &mut log);
    let codes: BTreeSet<_> = open_value_set_codes(&mixed, &log).into_iter().collect();
    assert_eq!(codes.len(), 2, "two terminologies, two ac-codes: {codes:?}");
    assert_eq!(
        errors(&print(&mixed).expect("the result prints"), None),
        Vec::<String>::new()
    );
}

fn parsed_14(src: &str) -> Archetype {
    parse_artefact(src, Dialect::Adl14).expect("the 1.4 source parses")
}

/// AOM 1.4 `master07-ontology_package.adoc` §Specialisation Depth: a term code
/// a specialised archetype introduces carries its depth in '.' markers.
/// `exam-uterus.v1` (depth 1) defines `at0001`, which `exam.v1` does not.
#[test]
fn a_specialisation_introducing_an_undotted_code_is_refused() {
    let child = parsed_14(&pack_2013("openEHR-EHR-CLUSTER.exam-uterus.v1.adl"));
    let parent = parsed_14(&pack_2013("openEHR-EHR-CLUSTER.exam.v1.adl"));
    match check_introduced_codes(&child, &parent) {
        Err(ConvertError::UndottedNewCodes { depth, codes }) => {
            assert_eq!((depth, codes), (1, vec!["at0001".to_owned()]));
        }
        other => panic!("expected UndottedNewCodes, got {other:?}"),
    }
}

/// `exam-foot.v0` introduces only dotted codes against `exam.v2` and passes;
/// with an undotted at-code, a dotted at-code and an undotted ac-code added,
/// only the undotted at-code is refused, since constraint codes "exist in a
/// flat code space" (the same §Specialisation Depth).
#[test]
fn only_undotted_new_term_codes_are_refused() {
    let src = ckm("openEHR-EHR-CLUSTER.exam-foot.v0.adl");
    let parent = parsed_14(&ckm("openEHR-EHR-CLUSTER.exam.v2.adl"));
    check_introduced_codes(&parsed_14(&src), &parent).expect("exam-foot dots its new codes");

    // The new terms go before the first term of the English list.
    let anchor = "[\"at0000.1\"] = <";
    assert!(src.contains(anchor), "the term list opens with at0000.1");
    let added: String = ["at0099", "at0.99", "ac0099"]
        .iter()
        .flat_map(|c| ["[\"", c, "\"] = < text = <\"x\"> description = <\"x\"> >\n"])
        .collect();
    let edited = src.replacen(anchor, &format!("{added}{anchor}"), 1);
    match check_introduced_codes(&parsed_14(&edited), &parent) {
        Err(ConvertError::UndottedNewCodes { depth, codes }) => {
            assert_eq!((depth, codes), (1, vec!["at0099".to_owned()]));
        }
        other => panic!("expected UndottedNewCodes, got {other:?}"),
    }
}

/// `exam-foot.v0` narrows `exam.v2`'s slot `at0004` to `at0004.1` (the shape
/// openEHR/archie#772 loses). The redefining slot takes the parent slot's id
/// `id5` (VDSSID, `AOM2/master04.5-constraint_model-class_definitions.adoc`)
/// and keeps its include; differentiated against the converted parent, the
/// slots inherited unchanged go (`ADL2/master09.02-spec_concepts.adoc`
/// §Differential and Flat Forms), and the child validates against its parent.
#[test]
fn a_specialised_slot_survives_and_inherited_slots_are_stripped() {
    let parent = converted(
        &ckm("openEHR-EHR-CLUSTER.exam.v2.adl"),
        &mut ConversionLog::default(),
    );
    let base = converted(
        &ckm("openEHR-EHR-CLUSTER.exam-foot.v0.adl"),
        &mut ConversionLog::default(),
    );
    let mut base_slots = Vec::new();
    slots(&data(&base).definition, &mut base_slots);
    let specialised = base_slots
        .iter()
        .find(|s| s.node_id == "id5")
        .expect("the redefining slot keeps the parent slot's id");
    assert!(
        specialised.includes.is_some(),
        "the specialised slot keeps its include"
    );

    let mut child = base.clone();
    differentiate(&mut child, &parent);
    let mut kept = Vec::new();
    slots(&data(&child).definition, &mut kept);
    assert_eq!(
        kept.iter().map(|s| s.node_id.as_str()).collect::<Vec<_>>(),
        ["id5"],
        "only the redefining slot stays in the differential form"
    );
    assert_eq!(
        kept.first().map(|s| &s.includes),
        Some(&specialised.includes)
    );

    let parent_text = print(&parent).expect("the parent prints");
    assert_eq!(errors(&parent_text, None), Vec::<String>::new());
    let mut repo = ArchetypeRepository::default();
    repo.insert(parse_artefact(&parent_text, Dialect::Adl2).expect("the parent re-parses"));
    assert_eq!(
        errors(&print(&child).expect("the child prints"), Some(&repo)),
        Vec::<String>::new(),
        "the differential child is valid against its parent"
    );
}
