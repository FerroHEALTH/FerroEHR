// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The canonical-JSON build and flatten the typed ones replace (#6003), kept
//! only for the old-against-new equality test until the typed paths are
//! proven equal on every corpus composition and template example.

#![allow(
    dead_code,
    unreachable_pub,
    unfulfilled_lint_expectations,
    clippy::all,
    clippy::pedantic,
    reason = "a frozen copy read only by the equality test, deleted once it passes"
)]

pub(crate) mod build;
pub(crate) mod ctx;
pub(crate) mod flatten;
pub(crate) mod identity;
pub(crate) mod map;
pub(crate) mod stubbed;

/// The typed build and flatten give what the canonical-JSON ones give, on the
/// examples of every operational template in the repository at each detail
/// level, on every canonical COMPOSITION the corpora hold, and on the FLAT
/// document each of those flattens to (#6003).
///
/// A build is compared as the canonical JSON of the typed value (the old
/// build's value read by the strict reader, the new one as built), and a
/// refusal on one side must be a refusal on the other; a flatten is compared
/// as its simplified tree.
#[cfg(test)]
mod parity {
    #![allow(
        clippy::disallowed_types,
        reason = "the equality test compares canonical-JSON renderings"
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
            (value.get("_type").and_then(Value::as_str) == Some("COMPOSITION"))
                .then_some((path, value))
        })
        .collect()
    }

    /// The outcome of one build: the canonical JSON of the typed value, or
    /// the refusal.
    fn old_build(doc: &SimDocument, wt: &WebTemplate) -> Result<Value, String> {
        let built = super::build::build_composition(doc, wt, NOW).map_err(|e| e.to_string())?;
        let typed: Composition = from_canonical_value(&built).map_err(|e| e.to_string())?;
        Ok(to_canonical_value(&typed))
    }

    fn new_build(doc: &SimDocument, wt: &WebTemplate) -> Result<Value, String> {
        crate::flat::build::build_composition(doc, wt, NOW)
            .map(|c| to_canonical_value(&c))
            .map_err(|e| e.to_string())
    }

    /// `value` without its `null` members, which the typed tree never holds.
    fn without_nulls(value: &Value) -> Value {
        match value {
            Value::Object(m) => Value::Object(
                m.iter()
                    .filter(|(_, v)| !v.is_null())
                    .map(|(k, v)| (k.clone(), without_nulls(v)))
                    .collect(),
            ),
            Value::Array(a) => Value::Array(a.iter().map(without_nulls).collect()),
            other => other.clone(),
        }
    }

    /// What one comparison counts.
    #[derive(Default)]
    struct Tally {
        equal: usize,
        refused_by_both: usize,
        /// Inputs whose canonical-JSON build the strict reader refuses and
        /// whose typed tree refuses where the unchecked JSON tree did not.
        refused_earlier: Vec<String>,
        failures: Vec<String>,
    }

    fn compare_build(label: &str, doc: &SimDocument, wt: &WebTemplate, tally: &mut Tally) {
        let strict = (old_build(doc, wt), new_build(doc, wt));
        let options = crate::flat::build::BuildOptions::for_template(wt);
        let old_value = super::build::build_composition(doc, wt, NOW).map(|v| without_nulls(&v));
        let new_value = crate::flat::build::composition_tree(doc, wt, NOW, &options)
            .map(|tree| crate::flat::typed::render(&*tree));
        match (&strict.0, &old_value, &new_value) {
            (Ok(_), Ok(old), Ok(new)) if old != new => tally.failures.push(format!(
                "{label}: the canonical-JSON projections differ\nold {old}\nnew {new}"
            )),
            (Ok(_), Ok(_), Err(e)) => tally
                .failures
                .push(format!("{label}: only the typed tree refuses: {e}")),
            (Err(_), Ok(_), Err(e)) => tally.refused_earlier.push(format!("{label}: {e}")),
            (_, Err(e), Ok(_)) => tally
                .failures
                .push(format!("{label}: only the JSON tree refuses: {e}")),
            _ => {}
        }
        match strict {
            (Ok(old), Ok(new)) => {
                tally.equal += 1;
                if old != new {
                    tally.failures.push(format!(
                        "{label}: built values differ\nold {old}\nnew {new}"
                    ));
                }
            }
            (Err(_), Err(_)) => tally.refused_by_both += 1,
            (Ok(_), Err(e)) => tally
                .failures
                .push(format!("{label}: only the typed build refuses: {e}")),
            (Err(e), Ok(_)) => tally
                .failures
                .push(format!("{label}: only the JSON build refuses: {e}")),
        }
    }

    #[test]
    fn the_typed_build_and_flatten_equal_the_canonical_json_ones() {
        let wts = web_templates();
        assert!(wts.len() > 100, "{} templates", wts.len());
        let mut tally = Tally::default();
        let mut flattened = 0usize;

        for wt in &wts {
            if wt.tree.rm_type != "COMPOSITION" {
                continue;
            }
            for level in [
                DetailLevel::Required,
                DetailLevel::Medium,
                DetailLevel::Complete,
            ] {
                let doc = example_composition_flat(wt, level);
                let label = format!("{} example {level:?}", wt.template_id);
                match flat::parse_flat(&doc) {
                    Ok(tree) => compare_build(&label, &tree, wt, &mut tally),
                    Err(e) => tally
                        .failures
                        .push(format!("{label}: the example does not parse: {e}")),
                }
            }
        }

        for (path, value) in compositions() {
            let Some(tid) = value
                .pointer("/archetype_details/template_id/value")
                .and_then(Value::as_str)
            else {
                continue;
            };
            let Some(wt) = wts.iter().find(|wt| wt.template_id == tid) else {
                continue;
            };
            let label = path.display().to_string();
            let Ok(old) = super::flatten::flatten_composition(&value, wt) else {
                continue;
            };
            flattened += 1;
            match crate::flat::flatten::flatten_composition(&value, wt) {
                Ok(new) if new == old => {}
                Ok(new) => tally.failures.push(format!(
                    "{label}: the JSON flatten differs\nold {:?}\nnew {:?}",
                    flat::emit_flat(&old),
                    flat::emit_flat(&new)
                )),
                Err(e) => tally
                    .failures
                    .push(format!("{label}: the JSON flatten refuses: {e}")),
            }
            if let Ok(typed) = from_canonical_value::<Composition>(&value) {
                let canonical = to_canonical_value(&typed);
                let from_canonical = super::flatten::flatten_composition(&canonical, wt).ok();
                let new = crate::flat::flatten::flatten_typed(&typed, wt);
                let streamed = flat::write_flat(&new).ok();
                let through_map = serde_json::to_string(&flat::emit_flat(&new)).ok();
                if streamed != through_map {
                    tally
                        .failures
                        .push(format!("{label}: the streamed FLAT text differs"));
                }
                if from_canonical.as_ref() != Some(&new) {
                    tally.failures.push(format!(
                        "{label}: the typed flatten differs\nold {:?}\nnew {:?}",
                        from_canonical.as_ref().map(flat::emit_flat),
                        flat::emit_flat(&new)
                    ));
                }
            }
            let flat_doc = flat::emit_flat(&old);
            match flat::parse_flat(&flat_doc) {
                Ok(tree) => compare_build(&format!("{label} flattened"), &tree, wt, &mut tally),
                Err(e) => tally
                    .failures
                    .push(format!("{label}: the flattened FLAT does not parse: {e}")),
            }
        }

        eprintln!(
            "{} templates; {} builds equal, {} refused by both; {} compositions flattened; \
             {} RM-invalid inputs refused by the typed tree where the unchecked JSON one \
             passed:\n{}",
            wts.len(),
            tally.equal,
            tally.refused_by_both,
            flattened,
            tally.refused_earlier.len(),
            tally.refused_earlier.join("\n")
        );
        assert!(
            tally.failures.is_empty(),
            "{} differences:\n{}",
            tally.failures.len(),
            tally.failures.join("\n\n")
        );
    }
}
