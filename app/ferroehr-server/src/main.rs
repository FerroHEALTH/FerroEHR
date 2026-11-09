// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `FerroEHR` server binary — a thin shell over [`ferroehr_server::process::main`].
//!
//! All wiring logic lives in the crate's library half so it is reachable from
//! integration tests (the Book ch12.3 split: `main` only parses and delegates).

use std::process::ExitCode;

/// The process-wide allocator: mimalloc instead of the platform `malloc`.
///
/// Under sustained load glibc's per-thread arenas kept freed memory and the
/// resident set grew run after run to the container's limit (#4288);
/// `MALLOC_ARENA_MAX=2` held it flat at a 16 % throughput cost. mimalloc
/// holds it flat and serves the same load faster
/// (<https://microsoft.github.io/mimalloc/>).
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> ExitCode {
    ferroehr_server::process::main(std::env::args_os())
}
