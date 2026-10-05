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
    cli::run()
}
