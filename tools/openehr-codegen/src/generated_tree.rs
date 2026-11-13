// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The generated tree on disk: what the emit targets wipe before they write and
//! the hand-written modules they declare in the generated anchors after.

use crate::render::emit_templates;
use std::path::{Path, PathBuf};

/// Whether a file's first line marks it as generated (`// @generated …`).
pub(crate) fn is_generated_file(path: &Path) -> bool {
    std::fs::read_to_string(path)
        .is_ok_and(|s| s.lines().next().is_some_and(|l| l.contains("@generated")))
}

/// Recursively delete every `@generated` file under `dir` (leaving hand-written
/// files + their directories in place), then prune directories left empty.
pub(crate) fn remove_generated_files(dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    remove_generated_files_except(dir, Path::new(""))
}

/// [`remove_generated_files`], leaving the subtree at `keep` untouched.
///
/// # Errors
/// A directory that cannot be read, or a file that cannot be removed.
pub(crate) fn remove_generated_files_except(
    dir: &Path,
    keep: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path == keep {
            continue;
        }
        if path.is_dir() {
            remove_generated_files_except(&path, keep)?;
            if std::fs::read_dir(&path)?.next().is_none() {
                std::fs::remove_dir(&path)?;
            }
        } else if is_generated_file(&path) {
            std::fs::remove_file(&path)?;
        }
    }
    Ok(())
}

/// For each directory whose module anchor (`lib.rs` at the crate root, else
/// `mod.rs`) is **generated**, append `pub mod <name>;` for every hand-written
/// child module not already declared — both hand-written `.rs` files
/// (`foo_impl.rs`) and hand-written module directories (a subdir whose own
/// `mod.rs` is hand-written, e.g. `odin/`). A hand-written anchor is left
/// untouched: it manages its own submodules (so we never duplicate an `odin/`
/// mod.rs's private `mod lexer;`). Modified anchors are added to `written` for
/// rustfmt.
///
/// `root_anchor` is the anchor of `src` itself (`lib.rs` for a crate, `mod.rs`
/// for a module tree such as `draft`); the subtree at `skip`, if any, is left
/// to its own target.
pub(crate) fn declare_hand_written_modules(
    src: &Path,
    root_anchor: &str,
    skip: Option<&Path>,
    written: &mut Vec<PathBuf>,
) -> Result<(), Box<dyn std::error::Error>> {
    // (dir, module-anchor file) pairs: the root anchors on `root_anchor`, each
    // subdir on its mod.rs. Collect all dirs first (sorted) for deterministic
    // output.
    let mut dirs = vec![src.to_path_buf()];
    collect_dirs(src, &mut dirs)?;
    dirs.retain(|d| skip.is_none_or(|s| !d.starts_with(s)));
    dirs.sort();
    for dir in dirs {
        let anchor = if dir == src {
            dir.join(root_anchor)
        } else {
            dir.join("mod.rs")
        };
        // Only extend a GENERATED anchor. A hand-written `mod.rs` (a hand-written
        // module directory) owns its own `mod`/`pub mod` declarations, so
        // appending here would duplicate them.
        let anchor_generated = is_generated_file(&anchor);
        if !anchor.exists() || !anchor_generated {
            continue;
        }
        let hand_written = hand_written_module_stems(&dir)?;
        if hand_written.is_empty() {
            continue;
        }
        if append_module_declarations(&anchor, &hand_written)? {
            written.push(anchor);
        }
    }
    Ok(())
}

/// The module stems in `dir` the generated module tree does not declare,
/// sorted.
///
/// A hand-written module DIRECTORY is one whose own `mod.rs` is not generated;
/// a fully-generated subpackage's `mod.rs` is already declared by the parent's
/// generated module tree and is skipped. Template-stamped copies ARE generated
/// files, but the generated module tree does not know them — they are woven
/// exactly like the hand-written siblings they replace.
///
/// # Errors
/// A directory that cannot be read.
fn hand_written_module_stems(dir: &Path) -> Result<Vec<String>, Box<dyn std::error::Error>> {
    let mut stems: Vec<String> = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let p = entry?.path();
        let Some(stem) = p.file_stem().and_then(std::ffi::OsStr::to_str) else {
            continue;
        };
        if p.is_dir() {
            let child_mod = p.join("mod.rs");
            if child_mod.exists() && !is_generated_file(&child_mod) {
                stems.push(stem.to_owned());
            }
        } else if p.extension().and_then(std::ffi::OsStr::to_str) == Some("rs")
            && !matches!(stem, "mod" | "lib" | "prelude")
            && (!is_generated_file(&p) || emit_templates::is_template_stamped(&p))
        {
            stems.push(stem.to_owned());
        }
    }
    stems.sort();
    Ok(stems)
}

/// Appends the missing `pub mod` declarations to a generated anchor,
/// reporting whether it was modified.
///
/// # Errors
/// A read or write failure on the anchor.
fn append_module_declarations(
    anchor: &Path,
    stems: &[String],
) -> Result<bool, Box<dyn std::error::Error>> {
    let mut body = std::fs::read_to_string(anchor)?;
    let mut appended = false;
    for m in stems {
        let decl = format!("pub mod {m};");
        if body.contains(&decl) {
            continue;
        }
        if !appended {
            body.push_str("\n// hand-written modules (spec behaviour), auto-declared:\n");
            appended = true;
        }
        body.push_str(&decl);
        body.push('\n');
    }
    if appended {
        std::fs::write(anchor, &body)?;
    }
    Ok(appended)
}

/// Recursively collect subdirectories of `dir` into `out`.
fn collect_dirs(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            out.push(path.clone());
            collect_dirs(&path, out)?;
        }
    }
    Ok(())
}
