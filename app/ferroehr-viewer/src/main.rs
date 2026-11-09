// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer server binary: the job the command line names
//! ([`ferroehr_viewer::cli::main`]), which installs the console from
//! `[telemetry]` once the configuration is read. Wiring only: logic lives in
//! the lib.

use std::process::ExitCode;

fn main() -> ExitCode {
    ferroehr_viewer::cli::main(std::env::args_os())
}
