// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Shared corpus plumbing for the Simplified Formats and validation gates:
//! the corpus walker over the canonical-JSON compositions `openehr-its`
//! vendors (reached by relative path, the twins adjudication honoured) and
//! the OPT pairing helpers.

use std::fs;
use std::path::{Path, PathBuf};

/// Every `.json` under `tests/vendor/` **and** `tests/fixtures/twins/`, sorted
/// for determinism.
///
/// The twins directory carries the repo-authored VALID half of each adjudicated
/// defective vendored fixture (`fixture_twins.rs`). It joins the corpus walk on
/// purpose: excluding a defective vendored document without admitting its
/// corrected twin would silently narrow what these gates cover, and the twins
/// rule exists precisely so a spec-correct refusal costs no coverage.
pub(crate) fn corpus_files() -> Vec<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut out = Vec::new();
    let mut stack = vec![
        manifest.join("../openehr-its/tests/vendor"),
        manifest.join("../openehr-its/tests/fixtures/twins"),
    ];
    while let Some(dir) = stack.pop() {
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                let p = e.path();
                if p.is_dir() {
                    stack.push(p);
                } else if p.extension().is_some_and(|x| x == "json") {
                    out.push(p);
                }
            }
        }
    }
    out.sort();
    out
}

/// The corpus-relative key of `path`: the path with its corpus root
/// (`tests/vendor/` or `tests/fixtures/`) stripped and separators normalized.
///
/// One derivation for every gate, so an absolute, machine-dependent path can
/// never reach a snapshot manifest or an exclusion lookup.
pub(crate) fn corpus_rel(path: &Path) -> String {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let rel = [
        manifest.join("../openehr-its/tests/vendor"),
        manifest.join("tests/fixtures"),
    ]
    .iter()
    .find_map(|root| path.strip_prefix(root).ok())
    .unwrap_or(path);
    rel.display().to_string().replace('\\', "/")
}

/// The file a gate should actually read for the vendored fixture at `path`:
/// its repo-authored VALID TWIN when one exists, else `path` itself.
///
/// A gate that walks a vendored directory DIRECTLY (rather than through
/// [`corpus_files`]) still has to honour the twins adjudication,
/// or it re-reads a document the corpus gates already refused. Routing through
/// here keeps one substitution rule for every gate.
pub(crate) fn twinned(path: &Path) -> PathBuf {
    let Some(stem) = path.file_stem().and_then(std::ffi::OsStr::to_str) else {
        return path.to_path_buf();
    };
    let twin = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../openehr-its/tests/fixtures/twins")
        .join(format!("{stem}.valid.json"));
    if twin.is_file() {
        twin
    } else {
        path.to_path_buf()
    }
}

// ── Simplified-formats corpus pairing (flat.rs / structured.rs / webtemplate.rs) ──

/// The canonical-JSON composition corpus vendored in this crate.
pub(crate) fn composition_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../openehr-its/tests/vendor/openehr_sdk/composition/canonical_json")
}

/// The shared service fixture corpus (`corpus/fixtures/service`), resolved
/// from this crate's manifest directory.
pub(crate) fn service_corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/fixtures/service")
}

/// All directories that hold `.opt` operational templates for pairing.
pub(crate) fn opt_dirs() -> Vec<PathBuf> {
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    vec![
        manifest.join("../openehr-its/tests/fixtures/sdk"),
        manifest.join("tests/fixtures/better"),
        service_corpus_dir(),
    ]
}

/// Every `.opt` under `dir`, recursively, sorted for determinism.
pub(crate) fn opt_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            out.extend(opt_files(&path));
        } else if path.extension().is_some_and(|e| e == "opt") {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// Build `templateId → WebTemplate` for every OPT the `opt14` parser can read.
pub(crate) fn web_templates()
-> std::collections::BTreeMap<String, openehr_sdt::flat::webtemplate::model::WebTemplate> {
    let mut out = std::collections::BTreeMap::new();
    for dir in opt_dirs() {
        for path in opt_files(&dir) {
            let Ok(xml) = fs::read_to_string(&path) else {
                continue;
            };
            let Ok(opt) = openehr_its::opt14::from_xml(&xml) else {
                continue;
            };
            if let Ok(wt) = openehr_sdt::flat::webtemplate::builder::build_web_template(&opt) {
                out.entry(wt.template_id.clone()).or_insert(wt);
            }
        }
    }
    out
}

/// Load every canonical COMPOSITION (with its file name + template id) from
/// the corpus, through the adjudicated-twin substitution.
pub(crate) fn compositions() -> Vec<(String, String, serde_json::Value)> {
    let mut out = Vec::new();
    let Ok(entries) = fs::read_dir(composition_dir()) else {
        return out;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "json") {
            continue;
        }
        let Ok(text) = fs::read_to_string(twinned(&path)) else {
            continue;
        };
        let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
            continue;
        };
        if value.get("_type").and_then(serde_json::Value::as_str) != Some("COMPOSITION") {
            continue;
        }
        let Some(tid) = value
            .pointer("/archetype_details/template_id/value")
            .and_then(serde_json::Value::as_str)
        else {
            continue;
        };
        let Some(name) = path.file_name().map(|n| n.to_string_lossy().into_owned()) else {
            continue;
        };
        out.push((name, tid.to_owned(), value));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}
