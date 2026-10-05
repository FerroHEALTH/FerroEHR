// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hand-written `*_impl.rs` siblings a generated crate already carries.
//!
//! A generated type file may declare in its banner that hand-written spec
//! behaviour lives beside it. That is a fact about the tree on disk, not about
//! the BMM, so the crate's `src/` is scanned once and the render stage is handed
//! the resulting set; otherwise the banner would point at files that do not
//! exist.
//!
//! The scan uses the same rule `crate::cli` uses for module wiring: a `.rs` file
//! whose first line does not mark it `@generated` is hand-written.

use std::collections::BTreeSet;
use std::path::Path;

/// The crate-relative paths (`common/generic/attestation_impl.rs`) of the
/// hand-written `*_impl.rs` files found under one generated crate's `src/`.
#[derive(Debug, Default, Clone)]
pub(crate) struct SiblingImpls {
    paths: BTreeSet<String>,
}

impl SiblingImpls {
    /// Scan `src` for hand-written `*_impl.rs` files, recursively.
    ///
    /// A missing directory (a crate emitted for the first time) yields an empty
    /// set — no sibling exists yet, which is exactly what the banner should
    /// then say.
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

/// Recursively add every hand-written `*_impl.rs` under `dir` to `out`, keyed
/// by its path relative to the scan root (`prefix` is that relative path of
/// `dir` itself, `""` at the root).
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
