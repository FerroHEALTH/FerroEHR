// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Corpus outer-parse gate: every well-formed ADL2 source (`*.adls`) in the
//! vendored openEHR ADL2 reference library must outer-parse into a
//! [`openehr_adl::source::SourceArtefact`] — the sections split, the HRID
//! parsed, the ODIN sections parsed.
//!
//! Semantic (V-code) failures in the corpus still parse (the outer parser does no
//! semantic validation), so they stay included. Only *lexer/parser-level*
//! intentional-fail fixtures are excluded, each named with its reason in
//! `EXCLUSIONS` — these are files whose outer structure the parser correctly
//! rejects (missing/empty required section, malformed HRID, malformed ODIN).

// A corpus test reports its pass counts on stdout for the developer running it.
#![allow(
    clippy::print_stdout,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use openehr_adl::parse::Dialect;
use std::path::{Path, PathBuf};

/// Files the outer parser correctly *rejects* (an intentional structural FAIL
/// fixture), each with the reason. These are exactly the `FAIL_*` cases whose
/// defect is at the outer/ODIN layer the outer parser checks.
const EXCLUSIONS: &[(&str, &str)] = &[
    (
        "openEHR-TEST_PKG-ENTRY.FAIL_archetype_id_missing.v1.adls",
        "no archetype keyword/HRID — starts at the language section (SARID/SUNK)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.FAIL_archetype_id_empty.v1.adls",
        "empty archetype id line (SARID)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.FAIL_definition_missing.v1.0.0.adls",
        "no definition section (SADF)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.FAIL_definition_empty.v1.0.0.adls",
        "empty definition section (SADF)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.FAIL_terminology_missing.v1.0.0.adls",
        "no terminology section (SAON)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.FAIL_terminology_empty.v1.0.0.adls",
        "empty terminology section (SDINV)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.FAIL_terminology_extra_end_mark.v1.0.0.adls",
        "spurious `>` in the terminology ODIN (SDINV)",
    ),
    (
        "openEHR-EHR-OBSERVATION.FAIL_dadl_spurious_delimiter.v1.0.0.adls",
        "spurious delimiter in the description ODIN (SDINV)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.FAIL_terminology_term_definitions_missing.v1.0.0.adls",
        "terminology section is empty (no ODIN body) — violates `terminology_section : SYM_TERMINOLOGY odin_text` (SDINV)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.VOKU_ac_code_duplicated_in_terminology.v1.0.0.adls",
        "duplicate sibling container keys in the terminology ODIN — refused at the ODIN layer per LANG/docs/odin/master05-content §Container Objects rule VDOBU (#1376), pre-empting the AOM-level VOKU tag (SDINV)",
    ),
    (
        "openEHR-TEST_PKG-ENTRY.VOKU_at_code_duplicated_in_terminology.v1.0.0.adls",
        "duplicate sibling container keys in the terminology ODIN — refused at the ODIN layer per LANG/docs/odin/master05-content §Container Objects rule VDOBU (#1376), pre-empting the AOM-level VOKU tag (SDINV)",
    ),
];

fn corpus_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus/adl2-reference")
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
fn every_adl2_source_outer_parses() {
    let root = corpus_root();
    let mut files = Vec::new();
    collect_adls(&root, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no .adls corpus files found under {root:?}"
    );

    let mut failures = Vec::new();
    let mut parsed = 0usize;
    let mut excluded = 0usize;
    for path in &files {
        if is_excluded(path) {
            excluded += 1;
            continue;
        }
        let src = std::fs::read_to_string(path).expect("read corpus file");
        match openehr_adl::source::parse_source(&src, Dialect::Adl2) {
            Ok(_) => parsed += 1,
            Err(errs) => {
                let first = errs.first().map(ToString::to_string).unwrap_or_default();
                failures.push(format!("{}: {first}", path.display()));
            }
        }
    }

    println!(
        "corpus outer-parse: {parsed}/{} parsed, {excluded} excluded",
        files.len() - excluded
    );
    assert!(
        failures.is_empty(),
        "these .adls files failed to outer-parse:\n{}",
        failures.join("\n")
    );
}

/// Every `EXCLUSIONS` entry is an intentional-FAIL fixture the outer parser
/// must actually REJECT — an exclusion that silently starts parsing would
/// otherwise decay into dead adjudication text.
#[test]
fn every_excluded_file_is_actually_rejected() {
    let root = corpus_root();
    let mut files = Vec::new();
    collect_adls(&root, &mut files);
    for (name, reason) in EXCLUSIONS {
        let path = files
            .iter()
            .find(|p| p.file_name().and_then(std::ffi::OsStr::to_str) == Some(name))
            .unwrap_or_else(|| panic!("excluded fixture {name} missing from the corpus"));
        let src = std::fs::read_to_string(path).expect("read corpus file");
        assert!(
            openehr_adl::source::parse_source(&src, Dialect::Adl2).is_err(),
            "{name} parses but is excluded as: {reason}"
        );
    }
}
