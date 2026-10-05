// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Corpus gate: every ADL2 corpus source with a `rules` section must parse that
//! section cleanly into the AM-level statement set. Sources with no `rules`
//! section yield `None`; a source that does not outer-parse is skipped here (the
//! outer/definition parse is covered by the sibling corpus gates).

#![allow(
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

use openehr_adl::parse::Dialect;
use openehr_adl::rules::parse_artefact_rules;
use openehr_adl::source::parse_source;

/// Recursively collect every `.adls` source under `dir`.
fn adls_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            adls_files(&path, out);
        } else if path.extension().and_then(std::ffi::OsStr::to_str) == Some("adls") {
            out.push(path);
        }
    }
}

#[test]
fn every_corpus_rules_section_parses() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/corpus");
    let mut files = Vec::new();
    adls_files(&corpus, &mut files);
    files.sort();
    assert!(
        !files.is_empty(),
        "no corpus .adls files found under {corpus:?}"
    );

    let mut with_rules = 0usize;
    let mut statements = 0usize;
    for path in &files {
        let Ok(src) = std::fs::read_to_string(path) else {
            continue;
        };
        // Only files that outer-parse are relevant to the rules gate.
        let Ok(artefact) = parse_source(&src, Dialect::Adl2) else {
            continue;
        };
        if artefact.rules.is_none() {
            continue;
        }
        match parse_artefact_rules(&artefact, &src) {
            Ok(Some(set)) => {
                with_rules += 1;
                statements += set.statement.as_ref().map_or(0, Vec::len);
                assert!(
                    !set.statement.as_ref().is_none_or(Vec::is_empty),
                    "{}: rules section parsed to zero statements",
                    path.display()
                );
            }
            Ok(None) => panic!("{}: has a rules span but parsed to None", path.display()),
            Err(errs) => panic!("{}: rules parse failed: {errs:?}", path.display()),
        }
    }

    // The vendored corpus carries rules sections (the aom_structures/rules set +
    // the alternatives dependency_choice fixture); the gate must exercise them.
    assert!(
        with_rules >= 4,
        "expected at least 4 corpus files with parseable rules sections, found {with_rules}"
    );
    assert!(
        statements >= with_rules,
        "every rules section has ≥1 statement"
    );
}
