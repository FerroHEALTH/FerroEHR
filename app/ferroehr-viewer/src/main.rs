// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer server binary: the job the command line names
//! ([`ferroehr_viewer::cli::run`]), which installs the console from
//! `[telemetry]` once the configuration is read. Wiring only: logic lives in
//! the lib.

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = <ferroehr_viewer::cli::Cli as clap::Parser>::parse();
    ferroehr_viewer::cli::run(cli).await
}
