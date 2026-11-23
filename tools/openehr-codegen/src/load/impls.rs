// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hand-written `*_impl.rs` siblings a generated crate carries: a `.rs`
//! file whose first line is not `@generated`, so a type's banner names only a
//! sibling that exists.

use std::collections::BTreeSet;
use std::path::Path;

/// The crate-relative paths (`common/generic/attestation_impl.rs`) of the
/// hand-written `*_impl.rs` files found under one generated crate's `src/`.
#[derive(Debug, Default, Clone)]
pub(crate) struct SiblingImpls {
    paths: BTreeSet<String>,
}

impl SiblingImpls {
    /// Scans `src` recursively for hand-written `*_impl.rs` files; a missing
    /// directory yields an empty set.
    pub(crate) fn scan(src: &Path) -> Self {
        let mut paths = BTreeSet::new();
        collect(src, "", &mut paths);
        Self { paths }
    }

    /// Whether the class emitted at module `chain` has a hand-written sibling —
    /// i.e. whether `<chain>_impl.rs` exists on disk.
    pub(crate) fn has_sibling(&self, chain: &[String]) -> bool {
        self.paths.contains(&format!("{}_impl.rs", chain.join("/")))
    }
}

/// Adds every hand-written `*_impl.rs` under `dir` to `out`, keyed by its path
/// relative to the scan root.
fn collect(dir: &Path, prefix: &str, out: &mut BTreeSet<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        let relative = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        if path.is_dir() {
            collect(&path, &relative, out);
        } else if name.ends_with("_impl.rs") && !is_generated(&path) {
            out.insert(relative);
        }
    }
}

/// Whether a file's first line marks it as generated (`// @generated …`) — the
/// same test the module-wiring pass applies.
fn is_generated(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .is_ok_and(|s| s.lines().next().is_some_and(|l| l.contains("@generated")))
}
