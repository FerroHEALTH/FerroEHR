// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Generates FerroEHR's configuration reference from the JSON Schema of its
//! configuration tree (#5316): the shipped default `ferroehr.toml`, holding
//! every key with its default under its description, which the book's
//! reference page includes. `--check` writes nothing and fails when the
//! committed file differs from what the types generate.
//!
//! # Examples
//!
//! ```text
//! cargo run -p config-reference
//! cargo run -p config-reference -- --check
//! ```

#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "build-time tool, never shipped in the server: the console is its \
              user interface, and a drifted file must say so"
)]

mod model;
mod render;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The shipped default `ferroehr.toml`, relative to the workspace root.
const TEMPLATE: &str = "app/ferroehr/assets/ferroehr.default.toml";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let check = match args.as_slice() {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => {
            eprintln!("usage: config-reference [--check]");
            return ExitCode::from(2);
        }
    };
    match run(&root(), check) {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(error) => {
            eprintln!("config-reference: {error}");
            ExitCode::FAILURE
        }
    }
}

/// The workspace root, two levels above this tool's manifest.
fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Generates the template under `root`, writing it, or with `check`
/// comparing it; returns whether the committed file matches.
///
/// # Errors
/// A message when the schema cannot be read or a file cannot be read or
/// written.
fn run(root: &Path, check: bool) -> Result<bool, String> {
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<ferroehr::config::FerroEhrConfig>();
    let sections = model::sections(schema.as_value())?;
    let files = [(TEMPLATE, render::template(&sections))];
    let mut clean = true;
    for (path, text) in files {
        let file = root.join(path);
        if check {
            let committed = std::fs::read_to_string(&file).unwrap_or_default();
            if committed != text {
                eprintln!(
                    "config-reference: {path} differs from what the configuration types \
                     generate; run `cargo run -p config-reference` and commit the result"
                );
                clean = false;
            }
        } else {
            std::fs::write(&file, text).map_err(|e| format!("{path}: {e}"))?;
            println!("  wrote {path}");
        }
    }
    Ok(clean)
}
