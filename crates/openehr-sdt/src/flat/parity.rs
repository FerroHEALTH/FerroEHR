// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The typed FLAT and STRUCTURED build and flatten agree with their
//! canonical-JSON forms on every repository template and corpus composition.

#![expect(
    clippy::disallowed_types,
    reason = "the agreement test compares canonical-JSON renderings"
)]

use std::path::{Path, PathBuf};

use openehr_its::json::{from_canonical_value, to_canonical_value};
use openehr_rm::prelude::Composition;
use serde_json::Value;

use crate::flat::example::{DetailLevel, example_composition_flat};
use crate::flat::sim::{SimDocument, flat};
use crate::flat::webtemplate::model::WebTemplate;

const NOW: &str = "2026-11-18T10:00:00Z";

fn files(roots: &[&str], extension: &str) -> Vec<PathBuf> {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mut out = Vec::new();
    let mut stack: Vec<PathBuf> = roots.iter().map(|r| base.join(r)).collect();
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == extension) {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

fn web_templates() -> Vec<WebTemplate> {
    let mut seen = std::collections::BTreeSet::new();
    files(
        &[
            "corpus/templates",
            "corpus/fixtures",
            "crates/openehr-sdt/tests/fixtures",
            "crates/openehr-its/tests/fixtures",
        ],
        "opt",
    )
    .into_iter()
    .filter_map(|path| {
        let xml = std::fs::read_to_string(&path).ok()?;
        let opt = openehr_its::opt14::from_xml(&xml).ok()?;
        crate::flat::webtemplate::builder::build_web_template(&opt).ok()
    })
    .filter(|wt| seen.insert(wt.template_id.clone()))
    .collect()
}

fn compositions() -> Vec<(PathBuf, Value)> {
    files(
        &[
            "crates/openehr-its/tests/vendor",
            "crates/openehr-its/tests/fixtures",
            "corpus/fixtures",
        ],
        "json",
    )
    .into_iter()
    .filter_map(|path| {
        let text = std::fs::read_to_string(&path).ok()?;
        let value: Value = serde_json::from_str(&text).ok()?;
        (value.get("_type").and_then(Value::as_str) == Some("COMPOSITION")).then_some((path, value))
    })
    .collect()
}

/// What the comparison counts.
#[derive(Default)]
struct Tally {
    agree: usize,
    refused_by_both: usize,
    flattened: usize,
    failures: Vec<String>,
}

/// A build's typed outcome against the strict reading of its unchecked
/// projection, each as canonical JSON or the refusal.
fn compare_build(label: &str, doc: &SimDocument, wt: &WebTemplate, tally: &mut Tally) {
    let options = crate::flat::build::BuildOptions::for_template(wt);
    let typed = crate::flat::build::build_composition_with(doc, wt, NOW, &options)
        .map(|c| to_canonical_value(&c))
        .map_err(|e| e.to_string());
    let projected = crate::flat::build::composition_tree(doc, wt, NOW, &options)
        .map_err(|e| e.to_string())
        .and_then(|tree| {
            from_canonical_value::<Composition>(&crate::flat::typed::render(&*tree))
                .map_err(|e| e.to_string())
        })
        .map(|c| to_canonical_value(&c));
    match (typed, projected) {
        (Ok(typed), Ok(projected)) => {
            tally.agree += 1;
            if typed != projected {
                tally.failures.push(format!(
                    "{label}: the typed build differs from its projection read strictly\n\
                     typed {typed}\nprojected {projected}"
                ));
            }
        }
        (Err(_), Err(_)) => tally.refused_by_both += 1,
        (Ok(_), Err(e)) => tally.failures.push(format!(
            "{label}: only the projection's strict read refuses: {e}"
        )),
        (Err(e), Ok(_)) => tally
            .failures
            .push(format!("{label}: only the typed build refuses: {e}")),
    }
}

/// The two flattens of one corpus COMPOSITION, the streamed FLAT text, and
/// the build of the FLAT it flattens to.
fn compare_composition(label: &str, value: &Value, wt: &WebTemplate, tally: &mut Tally) {
    let Ok(from_value) = crate::flat::flatten::flatten_composition(value, wt) else {
        return;
    };
    tally.flattened += 1;
    if let Ok(typed) = from_canonical_value::<Composition>(value) {
        let canonical = to_canonical_value(&typed);
        let from_canonical = crate::flat::flatten::flatten_composition(&canonical, wt).ok();
        let new = crate::flat::flatten::flatten_typed(&typed, wt);
        if from_canonical.as_ref() != Some(&new) {
            tally.failures.push(format!(
                "{label}: the typed flatten differs from the canonical-JSON one\nvalue {:?}\ntyped {:?}",
                from_canonical.as_ref().map(flat::emit_flat),
                flat::emit_flat(&new)
            ));
        }
        let text = flat::write_flat(&new).ok();
        if text != serde_json::to_string(&flat::emit_flat(&new)).ok() {
            tally
                .failures
                .push(format!("{label}: the streamed FLAT text differs"));
        }
        let from_text = text
            .as_deref()
            .and_then(|text| flat::parse_flat_text(text.as_bytes()).ok()?.ok());
        if from_text != flat::parse_flat(&flat::emit_flat(&new)).ok() {
            tally.failures.push(format!(
                "{label}: the FLAT text reads to another tree than its map"
            ));
        }
    }
    match flat::parse_flat(&flat::emit_flat(&from_value)) {
        Ok(tree) => compare_build(&format!("{label} flattened"), &tree, wt, tally),
        Err(e) => tally
            .failures
            .push(format!("{label}: the flattened FLAT does not parse: {e}")),
    }
}

#[test]
fn the_typed_build_and_flatten_agree_with_the_canonical_json_forms() {
    let wts = web_templates();
    assert!(wts.len() > 100, "{} templates", wts.len());
    let mut tally = Tally::default();
    for wt in wts.iter().filter(|wt| wt.tree.rm_type == "COMPOSITION") {
        for level in [
            DetailLevel::Required,
            DetailLevel::Medium,
            DetailLevel::Complete,
        ] {
            let label = format!("{} example {level:?}", wt.template_id);
            let example = example_composition_flat(wt, level);
            let text = serde_json::to_vec(&example).unwrap_or_default();
            if flat::parse_flat_text(&text).ok().and_then(Result::ok)
                != flat::parse_flat(&example).ok()
            {
                tally.failures.push(format!(
                    "{label}: the example's text reads to another tree than its map"
                ));
            }
            match flat::parse_flat(&example) {
                Ok(tree) => compare_build(&label, &tree, wt, &mut tally),
                Err(e) => tally
                    .failures
                    .push(format!("{label}: the example does not parse: {e}")),
            }
        }
    }
    for (path, value) in compositions() {
        let tid = value
            .pointer("/archetype_details/template_id/value")
            .and_then(Value::as_str);
        if let Some(wt) = tid.and_then(|tid| wts.iter().find(|wt| wt.template_id == tid)) {
            compare_composition(&path.display().to_string(), &value, wt, &mut tally);
        }
    }
    eprintln!(
        "{} templates; {} builds agree, {} refused by both; {} compositions flattened",
        wts.len(),
        tally.agree,
        tally.refused_by_both,
        tally.flattened
    );
    assert!(
        tally.failures.is_empty(),
        "{} differences:\n{}",
        tally.failures.len(),
        tally.failures.join("\n\n")
    );
}
