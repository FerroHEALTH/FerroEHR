// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer server binary: the console log, then the job the command line
//! names ([`ferroehr_viewer::cli::run`]). Wiring only — logic lives in the
//! lib.

#[cfg(not(target_arch = "wasm32"))]
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Console logs: env-filtered (RUST_LOG), stderr. Without a subscriber
    // every server-side error is invisible — an operational defect.
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let cli = <ferroehr_viewer::cli::Cli as clap::Parser>::parse();
    ferroehr_viewer::cli::run(cli).await
}

#[cfg(target_arch = "wasm32")]
fn main() {
    // The binary is the server half alone; the WASM client enters through
    // `lib.rs::hydrate`. cargo-leptos builds the bin for the host only.
}
