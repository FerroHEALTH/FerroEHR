// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! A constrained structural attribute with no content is synthesised with its
//! constrained node (AOM 1.4 `master04-constraint_model_package.adoc`
//! §`Valid_value`); an unconstrained one keeps the `at0001` "Any" placeholder
//! (ADL 1.4 `master05-cadl.adoc` §"Any" Constraints). Oracle: `ips.v0.opt`.

use std::path::{Path, PathBuf};

use openehr_its::opt14;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
use openehr_sdt::rm_instance::{ValidationKind, validate_composition};
use serde_json::Value;

fn fixtures_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures")
}

fn web_template(rel: &str) -> WebTemplate {
    let path = fixtures_dir().join(rel);
    let name = path.display();
    let xml = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {name}: {e}"));
    let opt = opt14::from_xml(&xml).unwrap_or_else(|e| panic!("parse {name}: {e}"));
    build_web_template(&opt).unwrap_or_else(|e| panic!("build {name}: {e}"))
}

/// Collect every ACTION object in a composition.
fn actions(v: &Value, out: &mut Vec<Value>) {
    match v {
        Value::Object(m) => {
            if m.get("_type").and_then(Value::as_str) == Some("ACTION") {
                out.push(v.clone());
            }
            m.values().for_each(|c| actions(c, out));
        }
        Value::Array(a) => a.iter().for_each(|e| actions(e, out)),
        _ => {}
    }
}

/// `Unexpected`-kind messages — the defect's signature ("unexpected node …").
fn unexpected(comp: &Value, wt: &WebTemplate) -> Vec<String> {
    validate_composition(comp, wt)
        .into_iter()
        .filter(|m| m.kind == ValidationKind::Unexpected)
        .map(|m| format!("{}: {}", m.path, m.message))
        .collect()
}

/// Recursively edit the `WebTemplate` tree in place.
fn edit_tree(node: &mut WebTemplateNode, f: &impl Fn(&mut WebTemplateNode)) {
    f(node);
    for child in &mut node.children {
        edit_tree(child, f);
    }
}

/// A synthesised `ACTION.description` carries the constrained `at0017` and its
/// name, and the composition validates clean.
#[test]
fn constrained_action_description_synthesises_the_constrained_node_id() {
    let wt = web_template("../../../openehr-its/tests/fixtures/sdk/ips.v0.opt");
    let comp = example_composition(&wt, DetailLevel::Required).expect("the example builds");

    let mut found = Vec::new();
    actions(&comp, &mut found);
    assert!(!found.is_empty(), "the IPS template has an ACTION");

    let description = found[0]
        .get("description")
        .expect("ACTION.description synthesised");
    assert_eq!(
        description.get("_type").and_then(Value::as_str),
        Some("ITEM_TREE"),
        "description is the constrained ITEM_TREE"
    );
    assert_eq!(
        description.get("archetype_node_id").and_then(Value::as_str),
        Some("at0017"),
        "the constrained node id is stamped, not the at0001 placeholder"
    );
    assert!(
        description
            .pointer("/name/value")
            .and_then(Value::as_str)
            .is_some_and(|n| !n.is_empty()),
        "the synthesised node carries a name"
    );
    assert!(
        unexpected(&comp, &wt).is_empty(),
        "the generated composition validates without 'unexpected node' errors"
    );
}

/// Without the structural stub the synthesised `at0001` placeholder is refused.
#[test]
fn clearing_the_stub_reintroduces_the_rejected_placeholder() {
    let mut wt = web_template("../../../openehr-its/tests/fixtures/sdk/ips.v0.opt");
    // Drop the stubs and existence constraints, keeping the closed-archetype one.
    edit_tree(&mut wt.tree, &|n| {
        n.structural_stubs.clear();
        n.existence.clear();
    });

    let comp = example_composition(&wt, DetailLevel::Required).expect("the example builds");
    let mut found = Vec::new();
    actions(&comp, &mut found);
    assert_eq!(
        found[0]
            .get("description")
            .and_then(|d| d.get("archetype_node_id"))
            .and_then(Value::as_str),
        Some("at0001"),
        "without the stub the builder falls back to the placeholder"
    );
    let errs = unexpected(&comp, &wt);
    assert!(
        errs.iter()
            .any(|e| e.contains("at0001") && e.contains("description")),
        "the closed-archetype walk rejects the placeholder under 'description': {errs:?}"
    );
}

/// When the attribute is genuinely unconstrained (no stub and no closed-archetype
/// constraint), the spec-legal `at0001` "Any" placeholder is kept and the
/// composition still validates — the "no constraint ≡ anything allowed" rule.
#[test]
fn unconstrained_structural_attribute_keeps_the_any_placeholder() {
    let mut wt = web_template("../../../openehr-its/tests/fixtures/sdk/ips.v0.opt");
    edit_tree(&mut wt.tree, &|n| {
        n.structural_stubs.clear();
        n.closed_attributes.clear();
        n.existence.clear();
    });

    let comp = example_composition(&wt, DetailLevel::Required).expect("the example builds");
    let mut found = Vec::new();
    actions(&comp, &mut found);
    assert_eq!(
        found[0]
            .get("description")
            .and_then(|d| d.get("archetype_node_id"))
            .and_then(Value::as_str),
        Some("at0001"),
        "unconstrained attribute keeps the at0001 'Any' placeholder"
    );
    assert!(
        unexpected(&comp, &wt).is_empty(),
        "an unconstrained attribute admits the placeholder (no closed-world rejection)"
    );
}

/// Corpus OPTs whose own constraints contradict, so no committable example
/// validates; pinned as still failing. `None` means every committable level.
///
/// `section_cardinality.opt`: `COMPOSITION.content` cardinality `1..1` against
/// SECTION occurrences summing to at least 8 (AOM 1.4
/// `master04-constraint_model_package.adoc` §cardinality, §occurrences).
const CONTRADICTORY_FIXTURES: &[(&str, Option<DetailLevel>)] = &[("section_cardinality.opt", None)];

/// The `Required` and `Medium` examples of every vendored template pass
/// [`validate_composition`] with no message; [`CONTRADICTORY_FIXTURES`] still
/// fail.
#[test]
fn committable_example_of_every_fixture_fully_validates() {
    let mut opts = Vec::new();
    for sub in ["sdk", "better"] {
        collect_opts(&fixtures_dir().join(sub), &mut opts);
    }
    assert!(
        opts.len() > 40,
        "the corpus is present ({} OPTs)",
        opts.len()
    );

    let mut checked = 0;
    for path in &opts {
        let Ok(xml) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(opt) = opt14::from_xml(&xml) else {
            continue;
        };
        let Ok(wt) = build_web_template(&opt) else {
            continue;
        };
        let name = path
            .file_name()
            .and_then(std::ffi::OsStr::to_str)
            .unwrap_or_default();
        for level in [DetailLevel::Required, DetailLevel::Medium] {
            let contradictory = CONTRADICTORY_FIXTURES
                .iter()
                .any(|(n, l)| *n == name && l.is_none_or(|l| l == level));
            let comp = example_composition(&wt, level).expect("the example builds");
            let msgs = validate_composition(&comp, &wt);
            if contradictory {
                assert!(
                    !msgs.is_empty(),
                    "{name} @ {level:?} is adjudicated spec-contradictory \
                     (CONTRADICTORY_FIXTURES) but now validates clean — the OPT/validator \
                     defect appears resolved, so remove it from the adjudication list"
                );
            } else {
                assert!(
                    msgs.is_empty(),
                    "{name} @ {level:?} must fully validate but produced: {msgs:?}"
                );
            }
        }
        checked += 1;
    }
    assert!(checked > 40, "checked {checked} templates");
}

/// A multi-archetype template's `Medium` example validates.
#[test]
fn issue_94_multi_archetype_medium_validates_fully() {
    let wt = web_template("better/ZN - Vital Functions Encounter.opt");
    let comp = example_composition(&wt, DetailLevel::Medium).expect("the example builds");
    let msgs = validate_composition(&comp, &wt);
    assert!(
        msgs.is_empty(),
        "the multi-archetype medium example must fully validate (issue #94): {msgs:?}"
    );
}

/// The generator is deterministic (no randomness, no wall-clock): two runs of
/// each level over the whole corpus are byte-identical.
#[test]
fn example_generation_is_byte_deterministic() {
    let mut opts = Vec::new();
    for sub in ["sdk", "better"] {
        collect_opts(&fixtures_dir().join(sub), &mut opts);
    }
    for path in &opts {
        let Ok(xml) = std::fs::read_to_string(path) else {
            continue;
        };
        let Ok(opt) = opt14::from_xml(&xml) else {
            continue;
        };
        let Ok(wt) = build_web_template(&opt) else {
            continue;
        };
        for level in [
            DetailLevel::Required,
            DetailLevel::Medium,
            DetailLevel::Complete,
        ] {
            let a = serde_json::to_string(
                &example_composition(&wt, level).expect("the example builds"),
            );
            let b = serde_json::to_string(
                &example_composition(&wt, level).expect("the example builds"),
            );
            assert_eq!(
                a.ok(),
                b.ok(),
                "two runs must be byte-identical: {path:?} @ {level:?}"
            );
        }
    }
}

fn collect_opts(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.is_dir() {
            collect_opts(&p, out);
        } else if p.extension().and_then(std::ffi::OsStr::to_str) == Some("opt") {
            out.push(p);
        }
    }
}
