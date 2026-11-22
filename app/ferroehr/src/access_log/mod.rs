// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR access log: the draft `ACCESS_LOG_ENTRY` as FerroEHR's stored
//! access record (#5121).
//!
//! The types are the generated [`openehr_rm::draft::access_log`] module and
//! their invariants [`openehr_rm::draft::access_log::invariants`]; an entry
//! is written once by the [`intake`] before the response that reports its
//! access, [`store`] keeps the records in the `audit` schema with their
//! content digests, the [`frontier`] and the [`seal`] seal windows of them,
//! and [`retention`] is the period each entry states, after which the store
//! replaces it by its tombstone, whose window later folds ([`gap`],
//! [`expiry`]). The draft is FerroEHR's
//! change proposal and no openEHR release, so no openEHR spec governs this
//! module: our own design.

pub mod chain;
pub mod checkpoint;
pub mod config;
pub mod definition;
#[cfg(feature = "fhir")]
pub mod details;
pub mod entry;
pub mod expiry;
pub mod frontier;
pub mod gap;
pub mod imported;
pub mod intake;
#[cfg(feature = "fhir")]
pub mod projection;
pub mod publish;
#[cfg(feature = "fhir")]
pub mod recorded;
pub mod retention;
pub mod seal;
pub mod search;
pub mod store;
pub mod stored_form;
pub mod transfer;
pub mod view;
