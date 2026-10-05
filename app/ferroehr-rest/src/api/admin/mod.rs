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
//! Three realize SM admin interfaces the release never surfaced: [`archive`]
//! (`I_ADMIN_ARCHIVE`), [`report`] (the four `I_ADMIN_SERVICE` statistics
//! calls) and [`dump_load`] (`I_ADMIN_DUMP_LOAD`). The fourth, [`integrity`],
//! realizes no SM interface at all: it sweeps the storage's two content copies
//! for disagreement, and no openEHR spec governs storage mechanics. Each
//! carries its own spec-silence flag and register citation.

pub mod archive;
pub mod dispatch;
pub mod dump_load;
pub mod integrity;
pub mod marks;
pub(crate) mod openapi_routes;
pub mod report;
