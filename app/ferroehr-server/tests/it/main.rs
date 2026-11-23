// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for `ferroehr-server` — the wiring binary's testable
//! library half (`src/lib.rs` behind a thin `main.rs`, the Rust Book ch12.3
//! shape, because a bin-only crate cannot be imported from `tests/`).
//!
//! Only the seams that need no database, listener or network run here:
//! [`build_authz`](ferroehr_server::build_authz), [`Cli`](ferroehr_server::Cli)
//! parsing, the extension each `ferroehr definitions load` file is read by,
//! [`run`](ferroehr_server::process::run) and
//! [`main`](ferroehr_server::process::main), and the `[privacy]` policy the run
//! path compiles out of the shipped defaults.

mod authz_wiring;
mod definitions;
mod exit_status;
mod wiring;
