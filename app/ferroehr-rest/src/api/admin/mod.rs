// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST **admin API** (Release-1.1.0, DEVELOPMENT status) —
//! `docs/specs/openehr/ITS-REST/specifications/docs/admin/` + the
//! `admin-*.openapi.yaml` OAS group (generated into `openehr_its::rest`).
//!
//! `operations` implements the generated operation contract over the
//! `FerroEhrService` methods it calls.
//!
//! Four further groups sit beside it under the same `/admin/` prefix and the
//! same gates — **our own extensions, governed by no ITS-REST operation**.
//! They are [`archive`] (archival storage), [`report`] (the four activity
//! statistics) and [`dump_load`] (file-system export and load), and
//! [`integrity`], which sweeps the storage's two content copies
//! for disagreement, and no openEHR spec governs storage mechanics.
//! [`access_log`] seals, verifies and exports the access log (#5129). Beside
//! them, [`marks`] keeps the legal marks, [`access_delay`] the EHDS Art. 3(3)
//! delay of the person's own access, [`ehr_access`] the controller's commit
//! and read of an EHR's `EHR_ACCESS` settings, which hold the EHDS Art. 8
//! restriction and the Art. 10 opt-out as directives, [`access_log_erasure`] the
//! erasure of an access-log entry a law orders, [`identifiers`] serves the audited
//! read of a party's protected national identifiers, [`binding_key`] the re-key of a
//! `PERSON`'s binding key, and [`outbox`] the feed of
//! commit events to the readers the `read_change_feed` grants name, and
//! [`registers`] the operational-policy registers (the audit category map, the
//! access delay and the Cedar policy set), and [`access_policy`] the
//! controller's versioned `ACCESS_POLICY`. Each carries its own
//! spec-silence flag and register citation.

pub mod access_delay;
pub mod access_log;
pub mod access_log_erasure;
pub mod access_policy;
pub mod access_repair;
pub mod archive;
pub mod binding_key;
mod body;
pub mod dump_load;
pub mod ehr_access;
pub mod identifiers;
pub mod integrity;
pub mod marks;
pub(crate) mod openapi_routes;
pub(crate) mod operations;
pub mod outbox;
pub mod registers;
pub mod report;
