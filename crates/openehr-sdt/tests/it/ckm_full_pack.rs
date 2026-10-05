// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![expect(clippy::panic, reason = "test assertions/diagnostics/fixtures")]
//! Breadth gate over the FULL vendored openEHR CKM template library
//! (`corpus/templates/ckm/full/`, vendored by
//! `scripts/vendor/ckm-templates.sh`): every OPT the public CKM publishes
//! must parse into the generated `opt14::types::OperationalTemplate`, build a
//! WebTemplate, and generate RM-shape-valid examples at every detail level.
//!
//! Why the whole library and not a curated handful: real-world OPTs carry
//! shapes a hand-written fixture never produces — multi-thousand-node
//! `CLUSTER` trees, wide repeats, deep `protocol` subtrees, exotic
//! constraint spellings. The curated journey pack (the parent directory,
//! gated by `example_rm_validity`) is the measured workload; this pack is
//! the breadth net.
//!
//! Corpus discipline: the pack is exercised 100%, and a file our conformant
//! reader rejects is either fixed upstream or listed in `ADJUDICATED` below
//! with the spec clause it violates — never a silent exclusion
//! (`.claude/rules/vendored-corpora.md`, `.claude/rules/testing.md`).

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use openehr_its::opt14;
use openehr_sdt::flat::example::{DetailLevel, example_composition};
use openehr_sdt::flat::webtemplate::builder::build_web_template;
use serde_json::Value;

/// Templates the conformant reader rejects for a spec-cited reason.
///
/// Each entry is `(file name, the spec clause the artefact violates)`. An
/// entry is an ADJUDICATION, not a skip list: the file stays vendored so the
/// refusal itself is pinned, and a reader that starts accepting it fails this
/// gate.
const ADJUDICATED: &[(&str, &str)] = &[];

/// The RM structure types that must never appear as an `ELEMENT.value`
/// (`RM data_structures`: `ELEMENT.value: DATA_VALUE [0..1]`).
const STRUCTURES: [&str; 5] = [
    "ITEM_TREE",
    "ITEM_LIST",
    "ITEM_TABLE",
    "ITEM_SINGLE",
    "CLUSTER",
];

fn pack_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/templates/ckm/full")
}

fn opt_files(dir: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) => panic!("read pack dir {}: {e}", dir.display()),
    };
    let mut out: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|e| e == "opt"))
        .collect();
    out.sort();
    out
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("?")
        .to_owned()
}

fn adjudication(name: &str) -> Option<&'static str> {
    ADJUDICATED
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, reason)| *reason)
}

fn collect_element_value_defects(node: &Value, path: &str, out: &mut Vec<String>) {
    match node {
        Value::Object(map) => {
            if map.get("_type").and_then(Value::as_str) == Some("ELEMENT")
                && let Some(value) = map.get("value")
                && let Some(value_type) = value.get("_type").and_then(Value::as_str)
                && STRUCTURES.contains(&value_type)
            {
                out.push(format!("{path}: ELEMENT.value is {value_type}"));
            }
            for (key, child) in map {
                collect_element_value_defects(child, &format!("{path}/{key}"), out);
            }
        }
        Value::Array(items) => {
            for (i, child) in items.iter().enumerate() {
                collect_element_value_defects(child, &format!("{path}[{i}]"), out);
            }
        }
        _ => {}
    }
}

/// What one packed template's parse + example generation produced.
enum TemplateOutcome {
    /// The file could not be read at all.
    ReadFailed(String),
    /// Parsed, built and generated defect-free examples.
    Clean,
    /// The OPT parse or the WebTemplate build refused it.
    Refused(String),
    /// Examples were generated but carry RM-shape defects.
    Defects(Vec<String>),
}

/// Parses one packed OPT and checks the examples it generates at every level.
fn examine_template(path: &Path, name: &str) -> TemplateOutcome {
    let xml = match std::fs::read_to_string(path) {
        Ok(xml) => xml,
        Err(e) => return TemplateOutcome::ReadFailed(format!("{name}: read failed: {e}")),
    };
    let opt = match opt14::from_xml(&xml) {
        Ok(opt) => opt,
        Err(e) => return TemplateOutcome::Refused(format!("{name}: OPT parse failed: {e}")),
    };
    let wt = match build_web_template(&opt) {
        Ok(wt) => wt,
        Err(e) => {
            return TemplateOutcome::Refused(format!("{name}: WebTemplate build failed: {e}"));
        }
    };
    let mut defects = Vec::new();
    for level in [
        DetailLevel::Required,
        DetailLevel::Medium,
        DetailLevel::Complete,
    ] {
        let example = example_composition(&wt, level);
        collect_element_value_defects(&example, &format!("{name}@{level:?}"), &mut defects);
    }
    if defects.is_empty() {
        TemplateOutcome::Clean
    } else {
        TemplateOutcome::Defects(defects)
    }
}

/// Every OPT of the full CKM library parses, builds a WebTemplate, and
/// generates RM-shape-valid examples at every detail level.
#[test]
fn full_ckm_pack_parses_and_generates_valid_examples() {
    let dir = pack_dir();
    let files = opt_files(&dir);
    assert!(
        files.len() >= 300,
        "the full CKM pack is missing: found {} OPTs in {} — re-run \
         scripts/vendor/ckm-templates.sh",
        files.len(),
        dir.display()
    );

    let mut findings = String::new();
    let mut parsed = 0_usize;
    let mut adjudicated = 0_usize;
    let mut unexpectedly_accepted = Vec::new();

    for path in &files {
        let name = file_name(path);
        let verdict = adjudication(&name);
        match examine_template(path, &name) {
            // A read failure is never adjudicable: the corpus itself is broken.
            TemplateOutcome::ReadFailed(message) => {
                let _ = writeln!(findings, "{message}");
            }
            TemplateOutcome::Clean => {
                if verdict.is_some() {
                    unexpectedly_accepted.push(name.clone());
                }
                parsed += 1;
            }
            TemplateOutcome::Refused(message) if verdict.is_none() => {
                let _ = writeln!(findings, "{message}");
            }
            TemplateOutcome::Defects(defects) if verdict.is_none() => {
                let _ = writeln!(findings, "{}", defects.join("\n"));
            }
            TemplateOutcome::Refused(_) | TemplateOutcome::Defects(_) => adjudicated += 1,
        }
    }

    println!(
        "full CKM pack: {} files, {parsed} clean, {adjudicated} adjudicated",
        files.len()
    );

    assert!(
        unexpectedly_accepted.is_empty(),
        "these templates are listed in ADJUDICATED but now pass — remove the \
         entries (a stale adjudication hides a real regression):\n{}",
        unexpectedly_accepted.join("\n")
    );
    assert!(
        findings.is_empty(),
        "unadjudicated defects across the full CKM template pack:\n{findings}"
    );
}
