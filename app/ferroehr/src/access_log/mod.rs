// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The openEHR access log: the draft `ACCESS_LOG_ENTRY` as FerroEHR's stored
//! access record (#5121).
//!
//! The types are the generated [`openehr_rm::draft::access_log`] module and
//! their invariants [`openehr_rm::draft::access_log::invariants`]; [`store`]
//! keeps the records in the `audit` schema, chained over their content
//! digests, and [`retention`] is the period each entry states, after which the
//! store replaces it by its tombstone ([`expiry`]). The draft is FerroEHR's
//! change proposal and no openEHR release, so no openEHR spec governs this
//! module: our own design.

pub mod chain;
pub mod checkpoint;
pub mod config;
#[cfg(feature = "fhir")]
pub mod details;
#[cfg(feature = "fhir")]
pub mod entry;
pub mod expiry;
#[cfg(feature = "fhir")]
pub mod projection;
#[cfg(feature = "fhir")]
pub mod recorded;
pub mod retention;
pub mod schedule;
pub mod search;
pub mod store;
pub mod transfer;
pub mod view;
