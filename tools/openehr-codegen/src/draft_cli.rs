// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `emit-drafts` subcommand: write the files `render::emit_drafts` renders
//! into each draft's application module, after wiping the module's previously
//! generated files.

use crate::cli::{remove_generated_files, rustfmt, write_emitted};
use crate::plan::drafts::DRAFTS;
use crate::render::emit_drafts::{module_dir, render_draft_files, workspace_root};

/// Regenerate every draft module of `plan::drafts::DRAFTS`.
///
/// # Errors
/// Returns an error if a draft does not compose, or a file cannot be removed,
/// written or formatted.
pub(crate) fn cmd_emit_drafts() -> Result<(), Box<dyn std::error::Error>> {
    let files = render_draft_files()?;
    let root = workspace_root();
    for draft in DRAFTS {
        let dir = root.join(module_dir(draft));
        if dir.exists() {
            remove_generated_files(&dir)?;
        }
    }
    let written = write_emitted(&root, &files)?;
    rustfmt(&written)?;
    println!("emitted {} draft files", written.len());
    Ok(())
}
