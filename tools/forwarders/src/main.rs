// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Lists the forwarders of the hand-written Rust, for the forwarder ratchet.
//!
//! A forwarder is a function whose whole body is one call to another,
//! `.claude/rules/rust-style.md` §No function only forwards to another.
//! `scripts/checks/forwarders.sh` runs this binary and holds its count to the
//! committed one.
//!
//! The output is one line per forwarder, sorted: its key, a tab, and its
//! place as `path:line`. The key is the path, the owner (inline modules, impl
//! type and name) and the target call, with no line number, so a change that
//! moves code inside a file leaves the key alone.
//!
//! # Examples
//!
//! ```text
//! forwarders --root .                       # every forwarder of the tree
//! forwarders --root . app/ferroehr/src/x.rs # the forwarders of one file
//! ```

mod detect;
mod walk;

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};

fn main() -> Result<()> {
    let mut root = PathBuf::from(".");
    let mut files = Vec::new();
    let mut args = std::env::args_os().skip(1);
    while let Some(arg) = args.next() {
        if arg == "--root" {
            root = PathBuf::from(args.next().context("--root takes a directory")?);
        } else if arg.to_str().is_some_and(|text| text.starts_with("--")) {
            bail!("usage: forwarders [--root DIR] [FILE...]");
        } else {
            files.push(PathBuf::from(arg));
        }
    }
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
    let wanted = files
        .iter()
        .map(|file| wanted_path(&root, file))
        .collect::<Result<Vec<_>>>()?;

    let mut out = io::stdout().lock();
    for found in walk::tree(&root, !wanted.is_empty())? {
        if !wanted.is_empty() && !wanted.contains(&found.path) {
            continue;
        }
        writeln!(
            out,
            "{}\t{}:{}",
            found.key(),
            found.path,
            found.forwarder.line
        )?;
    }
    out.flush()?;
    Ok(())
}

/// The path of `file` from `root` with `/` separators, as the scan names it.
fn wanted_path(root: &Path, file: &Path) -> Result<String> {
    let full = file
        .canonicalize()
        .with_context(|| format!("resolving {}", file.display()))?;
    walk::relative(root, &full)
}
