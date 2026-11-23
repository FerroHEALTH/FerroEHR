// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one integration-test binary of `ferroehr-ext`: one module per
//! feature-gated integration, each compiled only under its feature.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

#[cfg(feature = "multimedia")]
mod multimedia_engine;
