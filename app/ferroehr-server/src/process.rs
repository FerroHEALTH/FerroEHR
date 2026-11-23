// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The process run path of the `ferroehr` binary: the command line parsed,
//! the runtime started, the job run, and the exit status a failure ends the
//! process with.
//!
//! No openEHR spec governs the process: our own design, over the family's
//! `ferrohealth_server::run` with the exit codes of `sysexits(3)`.

use std::error::Error;
use std::ffi::OsString;
use std::process::ExitCode;

use ferrohealth_server::run::{EXIT_CONFIG, Failure, Run};

use crate::Cli;

/// The stack of every runtime thread, workers and the blocking pool alike.
///
/// The AOM engine's recursive walks overflow Tokio's 2 MiB default (#3062).
const RUNTIME_THREAD_STACK_BYTES: usize = 16 * 1024 * 1024;

/// Runs the `ferroehr` binary with the command line `args`, the program name
/// included, and returns the process exit code.
#[must_use]
pub fn main<I, T>(args: I) -> ExitCode
where
    I: IntoIterator<Item = T>,
    T: Into<OsString> + Clone,
{
    Run::new("ferroehr")
        .thread_stack_size(RUNTIME_THREAD_STACK_BYTES)
        .run(args, run)
}

/// Runs the job a parsed [`Cli`] names — the whole binary behind one testable
/// seam.
///
/// # Errors
///
/// Returns the [`Failure`] of the selected subcommand: a configuration the
/// binary refuses exits [`EXIT_CONFIG`] (78, `EX_CONFIG` of `sysexits(3)`),
/// every other boot, probe or serve failure exits 1.
pub async fn run(cli: Cli) -> Result<ExitCode, Failure> {
    Box::pin(crate::dispatch(cli))
        .await
        .map(|()| ExitCode::SUCCESS)
        .map_err(failure)
}

/// The failure `error` ends the process with: [`EXIT_CONFIG`] when its cause
/// chain holds a refusal of the configuration, 1 otherwise.
fn failure(error: anyhow::Error) -> Failure {
    let refused = error.chain().any(refuses_configuration);
    let failure = Failure::boxed(error);
    if refused {
        failure.with_code(EXIT_CONFIG)
    } else {
        failure
    }
}

/// Whether `cause` is a refusal of the configuration: the loader's, the
/// semantic validation's, the `[auth]` rules' or the SCIM endpoint's check.
fn refuses_configuration(cause: &(dyn Error + 'static)) -> bool {
    cause.is::<ferroehr::config::loader::ConfigErrors>()
        || cause.is::<ferroehr::config::loader::ConfigError>()
        || cause.is::<ferroehr::config::auth::AuthConfigError>()
        || cause.is::<ferroehr_rest::scim::endpoint::EndpointError>()
}
