// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 3(3) delay of the person's own access, at the request seam.
//!
//! Regulation (EU) 2025/327 Art. 3(3) lets a Member State delay a person's
//! access to their data "for a limited period of time until a health
//! professional is able to properly communicate and explain" it
//! (`docs/law/eu/ehds/text.html`). On a deployment whose access-delay register
//! is on, a request that acts for the person runs with the delay
//! enforced, published to the platform layer through
//! `ferroehr::system_log::access_context`, where the version reads, the AQL
//! builder, the exports and the write guard treat withheld data as absent.
//! Here, before dispatch, a route naming a withheld object answers `404` as
//! an unknown one does. Every other caller, a professional's request among
//! them, reads the record as before. No openEHR spec governs a delay of the
//! person's access: our own design/extension.

use axum::response::{IntoResponse, Response};
use ferrohealth_smart::kind::CallerKind;

use super::authn::{Principal, current_principal};
use crate::api::RequestParts;
use crate::overview::error::RestError;
use crate::state::AppState;

/// Whether the EHDS Art. 3(3) delay holds for `op` from the current caller.
///
/// It holds on a deployment whose access-delay register is on, for a caller
/// that acts for the person, on every operation but the controller's own
/// (`admin_*`).
pub(crate) fn applies(state: &AppState, op: &str) -> bool {
    if op.starts_with("admin_") || !state.backend().access_delay_enabled() {
        return false;
    }
    current_principal().is_some_and(|principal| acts_for_the_person(&principal))
}

/// Whether `principal` may be the person's own access: a caller
/// `ferrohealth-smart` classifies as the patient, or one representing the
/// person, through a `persons` or a `patients` issuer.
pub(crate) fn acts_for_the_person(principal: &Principal) -> bool {
    // NOTE: EHDS Art. 3(3), the owner's ruling of 2026-10-31: a mixed token is withheld
    // from, so the person never reads held data early; the rule is the family crate's.
    let portal = principal
        .represented
        .as_ref()
        .is_some_and(|represented| represented.portal.is_some());
    portal
        || principal.access.caller().is_some_and(|caller| {
            caller.is_the_person() || matches!(caller.kind(), CallerKind::Representative { .. })
        })
}

/// Answer, as an unknown object would be answered, a request whose path names
/// a versioned object withheld from the person.
///
/// `Ok(None)` lets the request proceed.
///
/// # Errors
/// A ready `500` response when the mark cannot be read: an access decision
/// never proceeds on an unknown state.
pub(crate) async fn refuse_withheld(
    state: &AppState,
    op: &str,
    parts: &RequestParts,
) -> Result<Option<Response>, Box<Response>> {
    for (raw, vo_id) in super::ehr_access::named_objects(parts) {
        let withheld = state
            .backend()
            .is_object_withheld(vo_id)
            .await
            .map_err(|error| Box::new(RestError::from(error).into_response()))?;
        if withheld {
            return Ok(Some(super::ehr_access::object_not_found(op, raw)));
        }
    }
    Ok(None)
}
