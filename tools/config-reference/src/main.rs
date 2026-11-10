// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Generates FerroEHR's configuration reference from the JSON Schema of its
//! configuration tree (#5316): the reference page of the book, the reference
//! TOML holding every key, and the shipped default `ferroehr.toml` holding the
//! keys an operator usually sets. `--check` writes nothing and fails when a
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
mod operator;
mod render;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

/// The reference page of the book, relative to the workspace root.
const PAGE: &str = "website/book/src/installation/config-keys.md";
/// The reference TOML holding every key.
const REFERENCE: &str = "app/ferroehr/assets/ferroehr.reference.toml";
/// The shipped default `ferroehr.toml`.
const DEFAULTS: &str = "app/ferroehr/assets/ferroehr.default.toml";

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

/// Generates the three files under `root`, writing them, or with `check`
/// comparing them; returns whether every committed file matches.
///
/// # Errors
/// A message when the schema cannot be read, an operator key is not in it,
/// or a file cannot be read or written.
fn run(root: &Path, check: bool) -> Result<bool, String> {
    let schema = schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<ferroehr::config::FerroEhrConfig>();
    let sections = model::sections(schema.as_value())?;
    let files = [
        (PAGE, render::page(&sections)),
        (REFERENCE, render::reference(&sections)),
        (DEFAULTS, render::defaults(&sections, operator::KEYS)?),
    ];
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
