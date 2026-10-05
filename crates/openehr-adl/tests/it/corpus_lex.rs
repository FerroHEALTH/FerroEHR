// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Corpus lex gate: every ADL2 source (`*.adls`) in the vendored openEHR ADL2
//! reference library must lex without producing an error token.
//!
//! ADL 1.4 sources (`*.adl`) are the 1.4→2 conversion input and are
//! skipped. The library encodes semantic (V-code) failures in file names —
//! those still lex; only genuinely lexer-level FAIL fixtures may be excluded,
//! and each such exclusion is named with its reason in `EXCLUSIONS` below.

// A corpus test reports its pass counts on stdout for the developer running it.
#![allow(
    clippy::print_stdout,
    reason = "integration-test assertions, diagnostics and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

use std::path::{Path, PathBuf};

/// Files that legitimately fail at the *lexer* level (not a parser bug).
/// Each entry names the file and the lexical reason. Empty unless the corpus
/// contains an intentionally lexically-malformed fixture.
const EXCLUSIONS: &[(&str, &str)] = &[];

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
fn every_adl2_source_lexes() {
    let root = corpus_root();
    let mut files = Vec::new();
    collect_adls(&root, &mut files);
    assert!(
        !files.is_empty(),
        "no .adls corpus files found under {root:?}"
    );

    let mut failures = Vec::new();
    let mut lexed = 0usize;
    let mut excluded = 0usize;
    for path in &files {
        if is_excluded(path) {
            excluded += 1;
            continue;
        }
        let src = std::fs::read_to_string(path).expect("read corpus file");
        match openehr_lang::v1_1::lexer::lex_adl(&src) {
            Ok(_) => lexed += 1,
            Err(e) => failures.push(format!("{}: {e}", path.display())),
        }
    }

    println!(
        "corpus lex: {lexed}/{} lexed, {excluded} excluded",
        files.len() - excluded
    );
    assert!(
        failures.is_empty(),
        "these .adls files failed to lex:\n{}",
        failures.join("\n")
    );
}
