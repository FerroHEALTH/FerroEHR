// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The **MESSAGE** wire — **our own extension**, the whole group.
//!
//! ITS-REST 1.1.0 publishes seven API groups (overview, system, ehr,
//! demographic, query, definition, admin) and NO message / EHR-Extract / TDD
//! group in either the docs text or the OAS, while the SM devotes a whole
//! chapter to one —
//! `docs/specs/openehr/SM/docs/openehr_platform/master09-message_service.adoc`,
//! including `i_ehr_extract_service.adoc` (`export_ehrs`,
//! `export_ehr_extracts`, `import_ehr`, `import_ehr_extract`) and
//! `i_tdd_service.adoc` (`import_tdd`; `i_message_service.adoc` itself declares
//! no functions). No released ITS-REST operation covers these calls.
//!
//! These routes are the honest realization of that service basis, and are
//! **excluded from ITS-REST wire conformance**: they gate the `EhrExtract` /
//! `Tds` / `MessageApi` CAPABILITY verdicts only.
//!
//! ## Why a group of their own, mounted beside the released groups
//!
//! Every route lives under `/message/`, a resource root the release does not
//! define, so no released path shape is extended or shadowed. The RM
//! EHR-Extract content the group moves is ordinary EHR content read and written
//! by an authenticated clinical principal, and SM puts these operations in the
//! MESSAGE component rather than ADMIN — so the group carries the coarse
//! `OperationClass::Clinical` classification the rest of the clinical surface
//! does (no `/admin/` gate, no ADMIN role). NO openEHR SPEC GOVERNS THE
//! PRIVILEGE LEVEL of these routes — our own design/extension, stated here
//! because the choice is ours.
//!
//! [`extract`] realizes `I_EHR_EXTRACT_SERVICE`, [`tdd`] realizes
//! `I_TDD_SERVICE`.

pub mod extract;
pub mod tdd;

use openehr_its::rest::runtime::ApiError;

use ferroehr::ids::EhrId;

use crate::api::RequestParts;
use crate::overview::error::RestError;

/// The `{ehr_id}` path segment shared by the group's EHR-scoped routes. A
/// malformed identifier is a `400` (SM `precondition_violation`) — never a
/// panic, and never a lookup with a fabricated id.
fn path_ehr_id(parts: &RequestParts) -> Result<EhrId, RestError> {
    let raw = parts.path.get("ehr_id").ok_or_else(|| {
        RestError(ApiError::BadRequest(
            "missing path parameter 'ehr_id'".to_owned(),
        ))
    })?;
    raw.parse::<EhrId>().map_err(|e| {
        RestError(ApiError::BadRequest(format!(
            "path parameter `ehr_id` is not a well-formed EHR identifier: {e}"
        )))
    })
}
