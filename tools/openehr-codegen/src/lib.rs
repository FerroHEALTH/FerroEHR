// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
#![allow(
    clippy::format_push_string,
    clippy::too_many_lines,
    reason = "the emitters build source text by appending formatted fragments, and \
              one emit fn per generated shape is the clearest structure — splitting \
              them would scatter a single file's layout across helpers"
)]
#![allow(
    clippy::panic,
    clippy::print_stdout,
    clippy::print_stderr,
    clippy::let_underscore_must_use,
    reason = "build-time tool, never shipped in the server: the console IS its user \
              interface, a malformed vendored spec must abort loudly, and \
              `let _ = writeln!(String)` is the infallible in-memory emit idiom"
)]

//! `openehr-codegen`: generates the openEHR spec crates from the vendored BMM
//! in four stages, `load` → `analyze` → `plan` → `render`, driven by `cli`;
//! [`testsupport`] exposes them to the invariant tests.
//!
//! Usage:
//!   `openehr-codegen check`          — load + validate the vendored BMM schemas.
//!   `openehr-codegen emit [OUTDIR]`  — emit Rust into the spec crates.
//!   `openehr-codegen emit-drafts`    — emit the FerroEHR-local draft modules.

mod analyze;
mod cli;
mod draft_cli;
mod generated_tree;
mod load;
mod plan;
mod render;
pub mod testsupport;

/// Runs the codegen CLI: parses `argv`, dispatches the subcommand, and returns
/// `SUCCESS`, `FAILURE` when a stage fails, or `2` for an unknown subcommand.
///
/// # Panics
/// Panics if a vendored input breaks an emitter invariant, such as a
/// non-constructible type (`analyze::Model::assert_constructible`).
#[must_use]
pub fn run() -> std::process::ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cmd = args.first().map_or("check", String::as_str);
    let result = match cmd {
        "check" => cli::cmd_check(),
        "emit" => cli::cmd_emit(args.get(1).map(std::path::PathBuf::from)),
        "check-xsd" => cli::cmd_check_xsd(),
        "emit-xml" => cli::cmd_emit_xml(),
        "emit-json" => cli::cmd_emit_json(),
        "emit-rest" => cli::cmd_emit_rest(),
        "emit-opt" => cli::cmd_emit_opt(),
        "emit-aom2" => cli::cmd_emit_aom2(),
        "emit-rm-model" => cli::cmd_emit_rm_model(),
        "emit-validate" => cli::cmd_emit_validate(),
        "emit-drafts" => draft_cli::cmd_emit_drafts(),
        "model-query" => cli::cmd_model_query(args.get(1..).unwrap_or_default()),
        other => {
            eprintln!(
                "unknown command {other:?}; use `check`, `emit [OUTDIR]`, `check-xsd`, `emit-xml`, `emit-json`, `emit-rest`, `emit-opt`, `emit-aom2`, `emit-rm-model`, `emit-validate`, `emit-drafts`, or `model-query`"
            );
            return std::process::ExitCode::from(cli::EXIT_USAGE);
        }
    };
    if let Err(e) = result {
        eprintln!("error: {e}");
        return std::process::ExitCode::FAILURE;
    }
    std::process::ExitCode::SUCCESS
}
