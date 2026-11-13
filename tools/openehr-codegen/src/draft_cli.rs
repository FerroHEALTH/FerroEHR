// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `emit-drafts` subcommand: write the files `render::emit_drafts` renders
//! into the `draft` module of each receiving openEHR crate, after wiping that
//! module's previously generated files, and declare the hand-written modules
//! beside them.

use crate::cli::{declare_hand_written_modules, remove_generated_files, rustfmt, write_emitted};
use crate::render::emit_drafts::{draft_roots, render_draft_files, workspace_root};

/// Regenerate every draft module of `plan::drafts::DRAFTS`.
///
/// # Errors
/// Returns an error if a draft does not compose, or a file cannot be removed,
/// written or formatted.
pub(crate) fn cmd_emit_drafts() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_draft_files()?;
    let root = workspace_root();
    let roots: Vec<_> = draft_roots().into_iter().map(|r| root.join(r)).collect();
    for dir in &roots {
        if dir.exists() {
            remove_generated_files(dir)?;
        }
    }
    let mut written = write_emitted(&root, &files)?;
    for dir in &roots {
        declare_hand_written_modules(dir, "mod.rs", None, &mut written)?;
    }
    written.sort();
    written.dedup();
    rustfmt(&written)?;
    println!("emitted {} draft files", files.len());
    Ok(())
}
