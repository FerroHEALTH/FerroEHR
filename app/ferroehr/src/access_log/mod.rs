// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR access log: the draft `ACCESS_LOG_ENTRY` as FerroEHR's stored
//! access record (#5121).
//!
//! The types are the generated [`openehr_rm::draft::access_log`] module and
//! their invariants [`openehr_rm::draft::access_log::invariants`]; an entry
//! is written to the [`intake`] before the response that reports its access,
//! the [`chainer`] links it, [`store`] keeps the records in the `audit`
//! schema, chained over their content digests, and [`retention`] is the period each entry states, after which the
//! store replaces it by its tombstone, which later folds into a [`gap`] of
//! the chain ([`expiry`]). The draft is FerroEHR's
//! change proposal and no openEHR release, so no openEHR spec governs this
//! module: our own design.

pub mod chain;
pub mod chainer;
pub mod checkpoint;
pub mod config;
#[cfg(feature = "fhir")]
pub mod details;
pub mod entry;
pub mod expiry;
pub mod gap;
pub mod imported;
pub mod intake;
#[cfg(feature = "fhir")]
pub mod projection;
pub mod publish;
#[cfg(feature = "fhir")]
pub mod recorded;
pub mod retention;
pub mod schedule;
pub mod search;
pub mod store;
pub mod stored_form;
pub mod transfer;
pub mod view;
