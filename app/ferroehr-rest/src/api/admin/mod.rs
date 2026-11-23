// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITS-REST **admin API** (Release-1.1.0, DEVELOPMENT status) —
//! `docs/specs/openehr/ITS-REST/specifications/docs/admin/` + the
//! `admin-*.openapi.yaml` OAS group (generated into `openehr_its::rest`).
//!
//! Further groups under the same `/admin/` prefix and gates are our own
//! extensions, governed by no ITS-REST operation; each module carries its own
//! spec-silence flag.

pub mod access_delay;
pub mod access_log;
pub mod access_log_erasure;
pub mod access_policy;
pub mod access_repair;
pub mod archive;
pub mod audit_delivery;
pub mod binding_key;
mod body;
pub mod dump_load;
pub mod ehr_access;
pub mod integrity;
pub mod marks;
pub(crate) mod openapi_routes;
pub(crate) mod operations;
pub mod outbox;
pub mod pmir_messages;
pub mod registers;
pub mod report;
