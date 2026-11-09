// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The public template-aware surface: instance navigation over the corpus,
//! the attribute-key table against the `master05-rm_mapping.adoc` tables, and
//! the operational-template facts a Web Template node carries.

use std::path::Path;

use openehr_rm::v1_2::paths::RmPath;
use openehr_sdt::flat::attribute_keys::{ATTRIBUTE_KEYS, KeyForm, KeySource};
use openehr_sdt::flat::navigate::Navigator;
use openehr_sdt::flat::webtemplate::model::{WebTemplateInputType, WebTemplateNode};

use crate::common;

/// The structural classes whose `master05-rm_mapping.adoc` tables name RM
/// attributes rather than data-value suffixes.
const STRUCTURAL: [&str; 14] = [
    "COMPOSITION",
    "ADMIN_ENTRY",
    "INSTRUCTION",
    "ACTION",
    "EVALUATION",
    "OBSERVATION",
    "ELEMENT",
    "CLUSTER",
    "ACTIVITY",
    "ISM_TRANSITION",
    "INSTRUCTION_DETAILS",
    "EVENT_CONTEXT",
    "INTERVAL_EVENT",
    "POINT_EVENT",
];

/// Every `(class, key, form)` the structural tables of `master05` list.
fn master05_rows(text: &str) -> Vec<(String, String, KeyForm)> {
    let mut rows = Vec::new();
    let mut section = String::new();
    let mut in_table = false;
    let mut cells: Vec<String> = Vec::new();
    let mut header_seen = false;
    for line in text.lines() {
        if let Some(name) = line.strip_prefix("== ") {
            name.trim().clone_into(&mut section);
        }
        if line.starts_with("|===") {
            in_table = !in_table;
            cells.clear();
            header_seen = false;
            continue;
        }
        let Some(cell) = line.strip_prefix('|').filter(|_| in_table) else {
            continue;
        };
        cells.push(cell.trim().to_owned());
        if cells.len() < 5 {
            continue;
        }
        if !header_seen {
            header_seen = cells.first().is_some_and(|c| c == "Flat Path");
        } else if STRUCTURAL.contains(&section.as_str()) {
            let flat = cells
                .first()
                .map(|c| c.trim_matches('`').replace("\\|", "|"))
                .unwrap_or_default();
            let (form, key) = if let Some(suffix) = flat.strip_prefix('|') {
                (KeyForm::Suffix, suffix.to_owned())
            } else if let Some(step) = flat.trim_start_matches('/').strip_suffix(":i") {
                (KeyForm::IndexedStep, step.to_owned())
            } else {
                (KeyForm::Step, flat.trim_start_matches('/').to_owned())
            };
            rows.push((section.clone(), key, form));
        }
        cells.clear();
    }
    rows
}

#[test]
fn every_master05_structural_row_has_its_attribute_key() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/specs/openehr/ITS-REST/docs/simplified_formats/master05-rm_mapping.adoc");
    let text = std::fs::read_to_string(path).expect("master05 is vendored");
    let rows = master05_rows(&text);
    assert!(
        rows.len() > 70,
        "the tables were read ({} rows)",
        rows.len()
    );
    for (class, key, form) in &rows {
        assert!(
            ATTRIBUTE_KEYS.iter().any(|row| row.key == key
                && row.form == *form
                && openehr_rm::v1_2::model::is_a(class, row.rm_class)),
            "master05 §{class} `{key}` ({form:?}) has no row"
        );
    }
    for row in ATTRIBUTE_KEYS
        .iter()
        .filter(|r| r.source == KeySource::Table)
    {
        assert!(
            rows.iter().any(|(class, key, form)| key == row.key
                && *form == row.form
                && openehr_rm::v1_2::model::is_a(class, row.rm_class)),
            "the table row {row:?} is in no master05 table"
        );
    }
}

/// Every node of a Web Template tree, the folded alternatives included.
fn nodes(root: &WebTemplateNode) -> Vec<&WebTemplateNode> {
    let mut out = Vec::new();
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        out.push(node);
        stack.extend(node.children.iter().chain(&node.alternatives));
    }
    out
}

#[test]
fn every_corpus_instance_path_and_pointer_name_the_instance() {
    let templates = common::web_templates();
    let mut reached = 0_usize;
    for (name, template_id, composition) in common::compositions() {
        let Some(template) = templates.get(&template_id) else {
            continue;
        };
        let navigator = Navigator::new(template);
        for node in nodes(&template.tree) {
            let Ok(path) = node.aql_path.parse::<RmPath>() else {
                continue;
            };
            for instance in navigator.instances(&composition, &path) {
                reached += 1;
                let found = openehr_rm::v1_2::paths::items_at_path(&composition, &instance.path());
                assert!(
                    matches!(found.as_slice(), [one] if std::ptr::eq(*one, instance.value())),
                    "{name}: {} names its instance alone",
                    instance.path()
                );
                assert!(
                    composition
                        .pointer(&instance.pointer())
                        .is_some_and(|at| std::ptr::eq(at, instance.value())),
                    "{name}: {} points at its instance",
                    instance.pointer()
                );
            }
        }
    }
    assert!(
        reached > 1000,
        "the corpus reaches its instances ({reached})"
    );
}

/// A navigator whose template is gone reaches exactly the instances one
/// built over the live template reaches, over every corpus composition.
#[test]
fn a_navigator_that_outlived_its_template_reaches_the_same_instances() {
    let templates = common::web_templates();
    let mut compared = 0_usize;
    for (name, template_id, composition) in common::compositions() {
        let Some(template) = templates.get(&template_id) else {
            continue;
        };
        let live = Navigator::new(template);
        let detached = Navigator::new(&template.clone());
        for node in nodes(&template.tree) {
            let Ok(path) = node.aql_path.parse::<RmPath>() else {
                continue;
            };
            let pointers = |navigator: &Navigator| -> Vec<String> {
                navigator
                    .instances(&composition, &path)
                    .iter()
                    .map(openehr_sdt::flat::navigate::Instance::pointer)
                    .collect()
            };
            let reached = pointers(&live);
            compared += reached.len();
            assert_eq!(reached, pointers(&detached), "{name}: {path}");
        }
    }
    assert!(
        compared > 1000,
        "the corpus reaches its instances ({compared})"
    );
}

#[test]
fn an_opt14_root_carries_the_terms_its_archetype_defines() {
    for (template_id, template) in common::web_templates() {
        let root = &template.tree;
        let code = root.name_code.as_deref().unwrap_or("at0000");
        assert!(
            root.facts.archetype_terms.contains_key(code),
            "{template_id}: the root's own code {code} is among its archetype's terms"
        );
        for node in nodes(root) {
            let coded = node.inputs.iter().filter(|input| {
                input.input_type == WebTemplateInputType::CodedText
                    && input
                        .suffix
                        .as_deref()
                        .is_none_or(|suffix| suffix == "code")
            });
            for option in coded.flat_map(|input| &input.list) {
                assert!(
                    option.terminology.is_some(),
                    "{template_id}: {} names the terminology of {}",
                    node.aql_path,
                    option.value
                );
            }
        }
    }
}

#[test]
fn an_embedded_template_id_rides_on_its_archetype_root() {
    let path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../corpus/templates/ckm/congenital-syphilis-case-investigation.opt");
    let xml = std::fs::read_to_string(path).expect("the CKM template is vendored");
    let opt = openehr_its::opt14::from_xml(&xml).expect("an OPT 1.4");
    let template =
        openehr_sdt::flat::webtemplate::builder::build_web_template(&opt).expect("builds");
    let embedded: Vec<&WebTemplateNode> = nodes(&template.tree)
        .into_iter()
        .filter(|node| node.facts.embedded_template_id.is_some())
        .collect();
    assert!(!embedded.is_empty(), "the template embeds others");
    for node in embedded {
        assert!(
            node.node_id
                .as_deref()
                .is_some_and(|id| id.starts_with("openEHR-")),
            "{} is an archetype root",
            node.aql_path
        );
    }
}
