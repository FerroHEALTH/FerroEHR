// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer server binary: the job the command line names
//! ([`ferroehr_viewer::cli::run`]), which installs the console from
//! `[telemetry]` once the configuration is read. Wiring only: logic lives in
//! the lib.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let cli = <ferroehr_viewer::cli::Cli as clap::Parser>::parse();
    ferroehr_viewer::cli::run(cli).await
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // The binary is the server half alone; the WASM client enters through
    // `lib.rs::hydrate`. cargo-leptos builds the bin for the host only.
}
