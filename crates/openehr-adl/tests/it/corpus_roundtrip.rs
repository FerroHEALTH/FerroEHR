// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Corpus round-trip gate: for every corpus `.adls` that fully
//! assembles, `parse_artefact → print → parse_artefact` must reconstruct a
//! **structurally equal** [`openehr_adl::assemble`] `Archetype`.
//!
//! The generated `v2_4` model derives `PartialEq`, so the two artefacts are
//! compared directly. The inclusion set mirrors the definition-parse gate: a
//! file whose intended failure is a cADL-syntax fixture (or a semantic
//! `V*`-code file whose *assembly* legitimately errors) is excluded with a
//! reason; there are no silent skips.

#![allow(
    clippy::print_stdout,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::assemble::parse_artefact;
use openehr_adl::parse::Dialect;
use openehr_adl::print::print;

/// Files excluded from the round-trip gate, each with a reason. These mirror the
/// definition-parse exclusions (intentional cADL-syntax FAIL fixtures) plus the
/// FAIL/`SA**`/`V*` files whose assembly deliberately does not yield an artefact.
const EXCLUSIONS: &[(&str, &str)] = &[
    (
        "openEHR-TEST_PKG-ENTRY.SCAS_attribute_empty.v1.0.0.adls",
        "empty attribute body `value matches {}` (SCAS) — definition does not parse",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.SCOAT_object_empty.v1.0.0.adls",
        "empty object body `ELEMENT[id2] matches {}` (SCOAT) — definition does not parse",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.SEXLU_attribute_wrong_existence.v1.0.0.adls",
        "intentional bad existence `{1..0}` (SEXLU2) — definition does not parse",
    ),
    (
        "openehr-ehr-ACTION.medication_precise.v0.0.1.adls",
        "attribute tuple with complex-object members — beyond the cADL parser (definition does not parse)",
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

#[test]
fn every_assembled_artefact_round_trips() {
    let root = corpus_root();
    let mut files = Vec::new();
    collect_adls(&root, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no .adls corpus files found under {root:?}"
    );

    let mut round_tripped = 0usize;
    let mut not_assembled = 0usize;
    let mut excluded = 0usize;
    let mut mismatches = Vec::new();

    for path in &files {
        if is_excluded(path) {
            excluded += 1;
            continue;
        }
        let src = std::fs::read_to_string(path).expect("read corpus file");
        let Ok(first) = parse_artefact(&src, Dialect::Adl2) else {
            // A file whose definition/terminology does not fully assemble is not
            // a round-trip concern (the parse-level gates govern it).
            not_assembled += 1;
            continue;
        };
        let printed = print(&first).expect("print the assembled artefact");
        match parse_artefact(&printed, Dialect::Adl2) {
            Ok(second) if first == second => round_tripped += 1,
            Ok(_) => mismatches.push(format!(
                "{}: printed form re-parsed to a DIFFERENT artefact",
                path.display()
            )),
            Err(errs) => mismatches.push(format!(
                "{}: printed form failed to re-parse: {}",
                path.display(),
                errs.first().map(ToString::to_string).unwrap_or_default()
            )),
        }
    }

    println!(
        "corpus round-trip: {round_tripped} round-tripped, {not_assembled} not-assembled (parse-gate concern), {excluded} excluded (of {} files)",
        files.len()
    );
    assert!(
        mismatches.is_empty(),
        "these artefacts failed the print→re-parse round trip ({} of {round_tripped} assembled):\n{}",
        mismatches.len(),
        mismatches.join("\n")
    );
    assert!(round_tripped > 0, "no artefacts round-tripped");
}
