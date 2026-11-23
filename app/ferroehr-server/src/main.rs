// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `FerroEHR` server binary — a thin shell over [`ferroehr_server::process::main`].

use std::process::ExitCode;

/// The process-wide allocator: mimalloc instead of the platform `malloc`.
///
/// mimalloc holds the resident set flat under sustained load (#4288,
/// <https://microsoft.github.io/mimalloc/>).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    ferroehr_server::process::main(std::env::args_os())
}
