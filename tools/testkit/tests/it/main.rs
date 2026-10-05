// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The crate's single integration-test binary (one binary per crate: cargo
//! compiles and links every top-level `tests/*.rs` separately —
//! <https://doc.rust-lang.org/cargo/reference/cargo-targets.html>). Each topic
//! is a module; nextest still runs every test in its own process.

mod harness;
