// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The **MESSAGE** wire — **our own extension**, the whole group.
//!
//! ITS-REST 1.1.0 publishes seven API groups (overview, system, ehr,
//! demographic, query, definition, admin) and NO message / EHR-Extract
//! group in either the docs text or the OAS. The group exports and imports
//! RM EHR-Extracts (`export_ehrs`, `export_ehr_extracts`, `import_ehr`,
//! `import_ehr_extract`); Template Data Documents are not served (#4199).
//!
//! These routes are **excluded from ITS-REST wire conformance**: they gate the `EhrExtract` /
//! `MessageApi` CAPABILITY verdicts only.
//!
//! ## Why a group of their own, mounted beside the released groups
//!
//! Every route lives under `/message/`, a resource root the release does not
//! define, so no released path shape is extended or shadowed. The RM
//! EHR-Extract content the group moves is ordinary EHR content read and written
//! by an authenticated clinical principal, so the group carries the coarse
//! `OperationClass::Clinical` classification the rest of the clinical surface
//! does (no `/admin/` gate, no ADMIN role). NO openEHR SPEC GOVERNS THE
//! PRIVILEGE LEVEL of these routes — our own design/extension, stated here
//! because the choice is ours.
//!
//! [`extract`] holds the export and import routes.

pub mod extract;

use openehr_its::rest::runtime::ApiError;

use ferroehr::ids::EhrId;

use crate::api::RequestParts;
use crate::overview::error::RestError;

/// The `{ehr_id}` path segment shared by the group's EHR-scoped routes. A
/// malformed identifier is a `400` — never a
/// panic, and never a lookup with a fabricated id.
fn path_ehr_id(parts: &RequestParts) -> Result<EhrId, RestError> {
    let raw = parts.path.get("ehr_id").ok_or_else(|| {
        RestError::from(ApiError::BadRequest(
            "missing path parameter 'ehr_id'".to_owned(),
        ))
    })?;
    raw.parse::<EhrId>().map_err(|e| {
        RestError::from(ApiError::BadRequest(format!(
            "path parameter `ehr_id` is not a well-formed EHR identifier: {e}"
        )))
    })
}
