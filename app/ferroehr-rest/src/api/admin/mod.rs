// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST **admin API** (Release-1.1.0, DEVELOPMENT status) —
//! `docs/specs/openehr/ITS-REST/specifications/docs/admin/` + the
//! `admin-*.openapi.yaml` OAS group (generated into `openehr_its::rest`).
//!
//! [`dispatch`] implements the generated operation contract over the
//! `ferroehr-sm` native API.
//!
//! Four further groups sit beside it under the same `/admin/` prefix and the
//! same gates — **our own extensions, governed by no ITS-REST operation**.
//! They are [`archive`] (archival storage), [`report`] (the four activity
//! statistics) and [`dump_load`] (file-system export and load), and
//! [`integrity`], which sweeps the storage's two content copies
//! for disagreement, and no openEHR spec governs storage mechanics. Beside
//! them, [`marks`] keeps the legal marks, [`access_delay`] the EHDS Art. 3(3)
//! delay of the person's own access, [`opt_out`] the EHDS Art. 10 opt-out from
//! access through the access services, and [`identifiers`] serves the audited
//! read of a party's protected national identifiers. Each carries its own
//! spec-silence flag and register citation.

pub mod access_delay;
pub mod archive;
mod body;
pub mod dispatch;
pub mod dump_load;
pub mod identifiers;
pub mod integrity;
pub mod marks;
pub(crate) mod openapi_routes;
pub mod opt_out;
pub mod report;
