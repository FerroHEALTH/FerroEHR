// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Corpus definition-parse gate: every `.adls` file that outer-parses must
//! also have its `definition` section (and every template overlay's
//! definition) cADL-parse into an AOM2 `CComplexObject` tree.
//!
//! Discipline (mirrors the outer-parse gate): a file whose *intended*
//! failure is at the cADL syntax level (an `S*` rule-code name in the file,
//! or a structural malformation the cADL parser correctly rejects) is excluded from
//! the clean-parse assertion, each with a reason. Files whose intended failure
//! is semantic (`V*`-code names) MUST still parse — semantic validation is a
//! validation harness. Files that do not outer-parse are skipped here (they
//! are the outer-parse gate's concern).

// A corpus test reports its pass counts on stdout for the developer running it.
#![allow(
    clippy::print_stdout,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::parse::Dialect;
use openehr_adl::source::{SourceArtefact, parse_source};

/// Files whose `definition` section is an intentional cADL-syntax FAIL fixture
/// (the cADL parser correctly rejects them), each with the reason + expected code.
const EXCLUSIONS: &[(&str, &str)] = &[
    (
        "openEHR-TEST_PKG-ENTRY.SCAS_attribute_empty.v1.0.0.adls",
        "empty attribute body `value matches {}` (SCAS)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.SCOAT_object_empty.v1.0.0.adls",
        "empty object body `ELEMENT[id2] matches {}` (SCOAT)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.SEXLU_attribute_wrong_existence.v1.0.0.adls",
        "intentional bad existence `{1..0}` (SEXLU2) — a syntax-level fixture",
    ),
    (
        "openehr-ehr-ACTION.medication_precise.v0.0.1.adls",
        "attribute tuple with complex-object members; the pinned `cadl2.g4` \
         `c_primitive_tuple_item` and the generated `CPrimitiveObject` model \
         only admit primitive tuple members — beyond the cADL parser",
    ),
];

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus")
}

fn collect_adls(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_adls(&path, out);
        } else if path.extension().is_some_and(|e| e == "adls") {
            out.push(path);
        }
    }
}

fn is_excluded(path: &Path) -> bool {
    let name = path
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or_default();
    EXCLUSIONS.iter().any(|(f, _)| *f == name)
}

/// Parse the root definition and every overlay definition of `art`, given the
/// whole source text. Returns the first cADL error string, if any.
fn definitions_parse(art: &SourceArtefact, src: &str) -> Result<(), String> {
    for a in std::iter::once(art).chain(art.overlays.iter()) {
        if let Some(def) = a.definition.as_ref() {
            let body = src.get(def.bytes.clone()).unwrap_or_default();
            openehr_adl::parse::parse_definition_body(body, Dialect::Adl2)
                .map_err(|errs| errs.first().map(ToString::to_string).unwrap_or_default())?;
        }
    }
    Ok(())
}

#[test]
fn every_definition_cadl_parses() {
    let root = corpus_root();
    let mut files = Vec::new();
    collect_adls(&root, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no .adls corpus files found under {root:?}"
    );

    let mut parsed = 0usize;
    let mut excluded = 0usize;
    let mut outer_failed = 0usize;
    let mut failures = Vec::new();
    for path in &files {
        if is_excluded(path) {
            excluded += 1;
            continue;
        }
        let src = std::fs::read_to_string(path).expect("read corpus file");
        let Ok(art) = parse_source(&src, Dialect::Adl2) else {
            // Not a definition-parse concern: the outer-parse gate governs
            // outer-parse failures.
            outer_failed += 1;
            continue;
        };
        match definitions_parse(&art, &src) {
            Ok(()) => parsed += 1,
            Err(first) => failures.push(format!("{}: {first}", path.display())),
        }
    }

    println!(
        "corpus definition-parse: {parsed} parsed, {excluded} excluded, {outer_failed} outer-parse-skipped (of {} files)",
        files.len()
    );
    assert!(
        failures.is_empty(),
        "these .adls definitions failed to cADL-parse:\n{}",
        failures.join("\n")
    );
}
