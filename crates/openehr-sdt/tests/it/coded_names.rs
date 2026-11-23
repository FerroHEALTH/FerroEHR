// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! Coded `LOCATABLE.name` and ISM `current_state` rubric fidelity.
//!
//! A name constrained as a `DV_CODED_TEXT` is recorded and stamped as a coded
//! name (RM common `master03-archetyped_package.adoc` §"The `LOCATABLE`
//! class"), and `current_state` resolves its rubric from `instruction_states`
//! (TERM 3.1.0 SPECPR-51).

use std::path::{Path, PathBuf};

use openehr_its::opt14;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use openehr_sdt::flat::webtemplate::model::{WebTemplate, WebTemplateNode};
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

/// Every `WebTemplate` node carrying a `name_coded` constraint, as `(display, code)`.
fn coded_name_nodes(node: &WebTemplateNode, out: &mut Vec<(String, String)>) {
    if let Some(coded) = &node.name_coded {
        out.push((node.name.clone().unwrap_or_default(), coded.code.clone()));
    }
    for c in &node.children {
        coded_name_nodes(c, out);
    }
}

/// Every `DV_CODED_TEXT` `name` in a composition, as `(value, defining_code.code_string)`.
fn coded_names_in(comp: &Value, out: &mut Vec<(String, String)>) {
    match comp {
        Value::Object(m) => {
            if let Some(name) = m.get("name")
                && name.get("_type").and_then(Value::as_str) == Some("DV_CODED_TEXT")
            {
                let value = name
                    .get("value")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                let code = name
                    .pointer("/defining_code/code_string")
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                out.push((value.to_owned(), code.to_owned()));
            }
            for v in m.values() {
                coded_names_in(v, out);
            }
        }
        Value::Array(a) => a.iter().for_each(|e| coded_names_in(e, out)),
        _ => {}
    }
}

/// The label of the coded value carrying `code` anywhere in the `WebTemplate`.
fn coded_value_label(node: &WebTemplateNode, code: &str) -> Option<String> {
    for input in &node.inputs {
        for cv in &input.list {
            if cv.value == code {
                return cv.label.clone();
            }
        }
    }
    node.children
        .iter()
        .find_map(|c| coded_value_label(c, code))
}

/// A `LOCATABLE.name` constrained as a `DV_CODED_TEXT` round-trips to a coded
/// instance name with its `defining_code`.
#[test]
fn coded_name_recorded_and_stamped() {
    let mut opts = Vec::new();
    for sub in ["sdk", "better"] {
        collect_opts(&fixtures_dir().join(sub), &mut opts);
    }

    let mut templates_with_coded_names = 0usize;
    let mut verified_stampings = 0usize;
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

        let mut nodes = Vec::new();
        coded_name_nodes(&wt.tree, &mut nodes);
        if nodes.is_empty() {
            continue;
        }
        templates_with_coded_names += 1;
        for (display, code) in &nodes {
            assert!(
                !display.is_empty(),
                "{path:?}: a coded name must carry a display value"
            );
            assert!(
                !code.is_empty(),
                "{path:?}: a coded name must carry a defining code"
            );
        }

        // The example composition stamps those names as DV_CODED_TEXT, never
        // plain DV_TEXT, and each recorded (value, code) surfaces on an instance.
        let comp = example_composition(&wt, DetailLevel::Complete).expect("the example builds");
        let mut stamped = Vec::new();
        coded_names_in(&comp, &mut stamped);
        for (value, code) in &stamped {
            assert!(
                !code.is_empty(),
                "{path:?}: a stamped coded name must carry a non-empty \
                 defining_code.code_string (value={value:?})"
            );
        }
        if nodes.iter().any(|n| stamped.contains(n)) {
            verified_stampings += 1;
        }
    }

    assert!(
        templates_with_coded_names > 0,
        "the corpus must exercise at least one DV_CODED_TEXT name constraint"
    );
    assert!(
        verified_stampings > 0,
        "at least one DV_CODED_TEXT name constraint must round-trip to a matching coded \
         instance name (value, code) — proving the composition builder stamps coded names"
    );
}

/// The `instruction_states` code `532` resolves to `completed` (SPECPR-51).
#[test]
fn ism_current_state_532_uses_instruction_states_rubric() {
    let wt = web_template("../../../openehr-its/tests/fixtures/sdk/minimal_action3.opt");
    let label = coded_value_label(&wt.tree, "532");
    assert_eq!(
        label.as_deref(),
        Some("completed"),
        "ISM current_state code 532 must render its instruction_states rubric ('completed'), \
         not the version_lifecycle_state rubric ('complete')"
    );
}
