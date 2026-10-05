// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one integration-test binary of `ferroehr-ext`: one module per
//! feature-gated integration, each compiled only under its feature, so
//! `--all-features` runs everything and a slim build compiles nothing here.
//!
//! These are behaviour tests over the crate's public seams — the typed
//! terminology decoders, the `EventPublisher` contract, the multimedia engine
//! over a real (in-memory) object store. The broker-backed and wiremock-backed
//! journeys stay in `app/ferroehr/tests/it`, where the platform wires the
//! integrations in; nothing here repeats them.

#![allow(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::panic,
    reason = "integration-test assertions and fixture plumbing outside #[test] fns, which the clippy.toml allow-*-in-tests scoping does not reach"
)]
#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule (.claude/rules/testing.md §Test-fixture construction): a FHIR response body and a canonical composition are bytes a server or a client sends, and building them through a typed model would test the model instead of the decoder"
)]

#[cfg(feature = "events")]
mod events_contract;
#[cfg(feature = "multimedia")]
mod multimedia_engine;
#[cfg(feature = "fhir")]
mod terminology_decode;
