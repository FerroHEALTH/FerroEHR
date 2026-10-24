// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one integration-test binary of `ferroehr-ext`: one module per
//! feature-gated integration, each compiled only under its feature, so
//! `--all-features` runs everything and a slim build compiles nothing here.
//!
//! These are behaviour tests over the crate's public seams: the `EventPublisher`
//! contract and the multimedia engine over a real (in-memory) object store.
//! The broker-backed and wiremock-backed journeys stay in
//! `app/ferroehr/tests/it`, where the platform wires the integrations in;
//! nothing here repeats them.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]

#[cfg(feature = "events")]
mod events_contract;
#[cfg(feature = "multimedia")]
mod multimedia_engine;
