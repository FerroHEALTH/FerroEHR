// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! ADL 1.4 archetypes as at-coded ADL 2.4: the conversion keeps every 1.4
//! code, so data recorded against the 1.4 archetype stays valid
//! (`ADL2/master01-preface.adoc` §Backward Compatibility), and a 1.4
//! archetype is held under the full identifier its `revision` states
//! (`ADL1.4/masterAppB-extended_metadata.adoc` §Standardised Items), which a
//! pre-release is referenced by (BASE `master07-referencing.adoc`
//! §Referencing).

#![allow(
    clippy::expect_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::collections::BTreeSet;
use std::path::Path;

use openehr_adl::adl14::convert::{ConvertConfig, parse_and_convert};
use openehr_adl::adl14::differ::differentiate;
use openehr_adl::adl14::log::ConversionLog;
use openehr_adl::artefact::ArchetypeRepository;
use openehr_adl::assemble::parse_artefact;
use openehr_adl::opt::create_opt;
use openehr_adl::parse::Dialect;
use openehr_adl::paths::enumerate_paths;
use openehr_adl::print::print;
use openehr_adl::validate::bindings::NoTerminologyResolver;
use openehr_adl::validate::catalogue::Severity;
use openehr_adl::validate::rm::ProductionRmModel;
use openehr_adl::validate::validate_source;
use openehr_am::v2_4::aom2::archetype::archetype::Archetype;
use openehr_am::v2_4::aom2::archetype::authored_archetype::{
    AuthoredArchetype, AuthoredArchetypeData,
};
use openehr_am::v2_4::aom2::constraint_model::c_complex_object::CComplexObject;
use openehr_am::v2_4::aom2::constraint_model::c_object::CObject;

/// The test template over the pre-release CKM archetypes.
const TEMPLATE: &str = include_str!("../fixtures/pre_release_summary.adlt");

/// The CKM ADL 1.4 archetypes the template compiles against.
const CONSTITUENTS: &[&str] = &[
    "openEHR-EHR-COMPOSITION.health_summary.v1.adl",
    "openEHR-EHR-SECTION.immunisation_list.v0.adl",
    "openEHR-EHR-SECTION.adhoc.v1.adl",
    "openEHR-EHR-EVALUATION.exclusion_global.v1.adl",
    "openEHR-EHR-EVALUATION.device_summary.v0.adl",
];

fn ckm(name: &str) -> String {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/archetypes/ckm/adl14")
        .join(name);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

fn converted(src: &str) -> Archetype {
    parse_and_convert(
        src,
        &ConvertConfig::default(),
        &mut ConversionLog::default(),
    )
    .expect("the 1.4 source converts")
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

/// Every object node id under `node`: complex objects, proxies and slots.
fn node_ids(node: &CComplexObject, out: &mut BTreeSet<String>) {
    let CComplexObject::CComplexObject(d) = node else {
        return;
    };
    out.insert(d.node_id.clone());
    for child in d
        .attributes
        .iter()
        .flatten()
        .flat_map(|attr| attr.children.iter().flatten())
    {
        match child {
            CObject::CComplexObject(cco) => node_ids(cco, out),
            CObject::CComplexObjectProxy(p) => {
                out.insert(p.node_id.clone());
            }
            CObject::ArchetypeSlot(s) => {
                out.insert(s.node_id.clone());
            }
            _ => {}
        }
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

#[test]
fn a_pre_release_14_archetype_converts_to_at_coded_adl_2_4_keeping_its_codes() {
    let src = ckm("openEHR-EHR-EVALUATION.device_summary.v0.adl");
    let source = parse_artefact(&src, Dialect::Adl14).expect("the 1.4 source parses");
    let out = converted(&src);
    let target = data(&out);

    assert_eq!(target.adl_version.as_deref(), Some("2.4.0"));
    assert_eq!(
        target.archetype_id.physical_id(),
        "openEHR-EHR-EVALUATION.device_summary.v0.0.1-alpha"
    );
    let mut before = BTreeSet::new();
    node_ids(&data(&source).definition, &mut before);
    before.remove("");
    let mut after = BTreeSet::new();
    node_ids(&target.definition, &mut after);
    assert!(
        before.is_subset(&after),
        "every 1.4 node code stays: {:?}",
        before.difference(&after).collect::<Vec<_>>()
    );
    assert!(after.iter().all(|code| code.starts_with("at")));
    assert_eq!(target.terminology.concept_code, "at0000");
    let text = print(&out).expect("the conversion prints");
    assert_eq!(errors(&text, None), Vec::<String>::new());
}

/// `waterlow_score`'s `at0116`, a node id and a value, stays `at0116` in both
/// roles under one term (`AOM2/master07-terminology_package.adoc`: at-coded
/// node ids and values share the at-codes).
#[test]
fn a_code_used_as_node_id_and_value_stays_one_at_code() {
    let src = ckm("openEHR-EHR-OBSERVATION.waterlow_score.v0.adl");
    let out = converted(&src);
    let target = data(&out);
    let mut nodes = BTreeSet::new();
    node_ids(&target.definition, &mut nodes);
    assert!(nodes.contains("at0116"), "at0116 keeps its node");
    let original = &target.terminology.term_definitions[&target.terminology.original_language];
    assert!(original.contains_key("at0116"));
    let text = print(&out).expect("the conversion prints");
    assert_eq!(errors(&text, None), Vec::<String>::new());
}

/// `exam-foot.v0`'s redefining slot keeps its parent slot's at-code (VDSSID),
/// and the differentiated child validates against its at-coded parent.
#[test]
fn a_specialised_14_archetype_converts_to_an_at_coded_differential() {
    let parent = converted(&ckm("openEHR-EHR-CLUSTER.exam.v2.adl"));
    let mut child = converted(&ckm("openEHR-EHR-CLUSTER.exam-foot.v0.adl"));
    differentiate(&mut child, &parent);
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

#[test]
fn a_pre_release_14_archetype_is_held_under_its_full_identifier() {
    let mut repo = ArchetypeRepository::default();
    let src = ckm("openEHR-EHR-EVALUATION.device_summary.v0.adl");
    repo.insert(parse_artefact(&src, Dialect::Adl14).expect("the 1.4 source parses"));
    assert!(
        repo.resolve("openEHR-EHR-EVALUATION.device_summary.v0.0.1-alpha", None)
            .is_some(),
        "the full pre-release identifier resolves"
    );
    assert!(
        repo.resolve("openEHR-EHR-EVALUATION.device_summary.v0", None)
            .is_none(),
        "a major-version reference never reaches a pre-release"
    );
}

/// `path` without the predicates whose code is not in `kept`.
fn without_codes_outside(path: &str, kept: &BTreeSet<String>) -> String {
    let mut out = String::new();
    let mut rest = path;
    while let Some((head, tail)) = rest.split_once('[') {
        out.push_str(head);
        let (code, after) = tail.split_once(']').expect("a closed predicate");
        if kept.contains(code) {
            out.push('[');
            out.push_str(code);
            out.push(']');
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// The template, its overlay specialising the pre-release
/// `immunisation_list.v0.0.1-alpha`, compiles against the 1.4 sources and
/// against their at-coded conversions to the same paths over the 1.4 codes,
/// the codes data carries; the conversion adds only the codes it mints for
/// 1.4 objects that had none.
#[test]
fn an_overlay_over_a_pre_release_parent_compiles_alike_over_the_14_sources_and_their_conversions() {
    let template = parse_artefact(TEMPLATE, Dialect::Adl2).expect("the template parses");
    let mut sources = ArchetypeRepository::default();
    let mut conversions = ArchetypeRepository::default();
    for name in CONSTITUENTS {
        let src = ckm(name);
        sources.insert(parse_artefact(&src, Dialect::Adl14).expect("the 1.4 source parses"));
        let text = print(&converted(&src)).expect("the conversion prints");
        conversions.insert(parse_artefact(&text, Dialect::Adl2).expect("the conversion parses"));
    }
    let over_sources = create_opt(&template, &sources)
        .unwrap_or_else(|e| panic!("create_opt over the 1.4 sources: {e}"))
        .template;
    let over_conversions = create_opt(&template, &conversions)
        .unwrap_or_else(|e| panic!("create_opt over the conversions: {e}"))
        .template;
    let paths = enumerate_paths(&over_sources.definition);
    for path in [
        "/content[at0000.2]/items[at0002.1]/data[at0001]/items[at0002]",
        "/content[at0000.3]/items[at0000.2]/data[at0001]/items[at0022]/items[at0007]",
    ] {
        assert!(
            paths.iter().any(|p| p == path),
            "{path} is a path of the OPT"
        );
    }
    let codes: BTreeSet<String> = paths
        .iter()
        .flat_map(|p| p.split('[').skip(1))
        .filter_map(|tail| tail.split_once(']').map(|(code, _)| code.to_owned()))
        .collect();
    let converted_paths: Vec<String> = enumerate_paths(&over_conversions.definition)
        .iter()
        .map(|p| without_codes_outside(p, &codes))
        .collect();
    assert_eq!(paths, converted_paths);
}
