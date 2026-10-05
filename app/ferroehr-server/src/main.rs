// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `FerroEHR` server binary — a thin shell over [`ferroehr_server::run`].
//!
//! All wiring logic lives in the crate's library half so it is reachable from
//! integration tests (the Book ch12.3 split: `main` only parses and delegates).

use clap::Parser;

/// The stack of every runtime thread, workers and the blocking pool alike.
///
/// Tokio's default is 2 MiB, which the AOM engine's recursive walks over a
/// well-stocked archetype repository crossed in a debug build (#3062). The
/// reservation is virtual: pages are committed only as a stack actually grows.
const RUNTIME_THREAD_STACK_BYTES: usize = 16 * 1024 * 1024;

fn main() -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(RUNTIME_THREAD_STACK_BYTES)
        .build()?;
    runtime.block_on(ferroehr_server::run(ferroehr_server::Cli::parse()))
}
