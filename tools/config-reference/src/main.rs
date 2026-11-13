// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Generates FerroEHR's configuration reference from the JSON Schema of its
//! configuration tree (#5316): the shipped default `ferroehr.toml`, holding
//! every key with its default under its description, which the book's
//! reference page includes. The file opens with the keys a deployment that
//! holds real data must set, which are its only live lines, and every key
//! carries its class (#5455); a key the types leave unclassified fails both
//! modes ([`class`]). `--check`
//! writes nothing and fails when the committed file differs from what the
//! types generate.
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

mod class;
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

/// The JSON Schema of the configuration tree.
fn schema() -> schemars::Schema {
    schemars::generate::SchemaSettings::draft2020_12()
        .into_generator()
        .into_root_schema_for::<ferroehr::config::FerroEhrConfig>()
}

/// Generates the template under `root`, writing it, or with `check`
/// comparing it; returns whether the committed file matches.
///
/// # Errors
/// A message when the schema cannot be read or a file cannot be read or
/// written.
fn run(root: &Path, check: bool) -> Result<bool, String> {
    let sections = model::sections(schema().as_value())?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_key_of_the_configuration_has_a_class() {
        let sections = model::sections(schema().as_value()).expect("every key is classified");
        let must_set: Vec<String> = sections
            .iter()
            .flat_map(|section| {
                section
                    .keys
                    .iter()
                    .filter(|key| key.must_set)
                    .map(move |key| format!("{}.{}", section.path, key.name))
            })
            .collect();
        assert!(
            must_set.contains(&"record.subject_index".to_owned()),
            "{must_set:?}"
        );
        assert!(
            must_set.contains(&"database.url".to_owned()),
            "{must_set:?}"
        );
    }

    #[test]
    fn the_template_sets_only_must_set_keys() {
        let sections = model::sections(schema().as_value()).expect("every key is classified");
        let must_set: Vec<String> = sections
            .iter()
            .flat_map(|section| section.keys.iter().filter(|key| key.must_set))
            .map(|key| model::bare(&key.name))
            .collect();
        let template = render::template(&sections);
        let live: Vec<&str> = template
            .lines()
            .filter(|line| !line.starts_with('#') && !line.starts_with('[') && !line.is_empty())
            .filter_map(|line| line.split_once(" = ").map(|(name, _)| name))
            .collect();
        assert!(!live.is_empty(), "the template sets no key at all");
        for name in &live {
            assert!(
                must_set.iter().any(|key| key == name),
                "{name} is a live line but no must-set key: {live:?}"
            );
        }
    }
}
