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

//! `openehr-codegen` — generates the openEHR spec crates from the vendored BMM
//! meta-model, structured as a four-stage pipeline:
//! `load` → `analyze` → `plan` → `render`, driven by `cli` (private modules —
//! `cargo doc --document-private-items` renders them).
//!
//! The stages are library modules, not a binary-only tree, so the emitter's
//! invariants are tested as properties over the real pipeline on the real
//! vendored inputs (`tests/emitter_invariants.rs`) through [`testsupport`].
//!
//! Usage:
//!   `openehr-codegen check`          — load + validate the vendored BMM schemas.
//!   `openehr-codegen emit [OUTDIR]`  — emit Rust into the spec crates.

mod analyze;
mod cli;
mod load;
mod plan;
mod render;
pub mod testsupport;

/// Run the codegen CLI (the binary entry point): parse `argv`, dispatch the
/// subcommand, and report the outcome as a process exit code.
///
/// Returns `ExitCode::SUCCESS` on a completed emit/check, `ExitCode::FAILURE`
/// when a stage returned an error, and `2` for an unrecognized subcommand.
///
/// # Panics
/// Panics (the loud tool-mode backstop) if a vendored input is malformed in a
/// way the emitter's invariants forbid — e.g. a non-constructible type would be
/// emitted (see `analyze::Model::assert_constructible`).
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
        "model-query" => cli::cmd_model_query(args.get(1..).unwrap_or_default()),
        other => {
            eprintln!(
                "unknown command {other:?}; use `check`, `emit [OUTDIR]`, `check-xsd`, `emit-xml`, `emit-json`, `emit-rest`, `emit-opt`, `emit-aom2`, `emit-rm-model`, `emit-validate`, or `model-query`"
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
