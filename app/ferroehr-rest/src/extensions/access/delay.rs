// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 3(3) delay of the person's own access, at the request seam.
//!
//! Regulation (EU) 2025/327 Art. 3(3) lets a Member State delay a person's
//! access to their data "for a limited period of time until a health
//! professional is able to properly communicate and explain" it
//! (`docs/law/eu/ehds/text.html`). On a deployment that enables
//! `[access_delay]`, a patient-context request runs with the delay enforced,
//! published to the platform layer through
//! `ferroehr::system_log::access_context`, where the version reads, the AQL
//! builder, the exports and the write guard treat withheld data as absent.
//! Here, before dispatch, a route naming a withheld object answers `404` as
//! an unknown one does. Every other caller, a professional's request among
//! them, reads the record as before. No openEHR spec governs a delay of the
//! person's access: our own design/extension.

use axum::response::{IntoResponse, Response};

use super::authn::{Principal, current_principal};
use crate::api::RequestParts;
use crate::overview::error::RestError;
use crate::state::AppState;

/// Whether the EHDS Art. 3(3) delay holds for `op` from the current caller.
///
/// It holds on a deployment that enables `[access_delay]`, for a
/// patient-context caller, on every operation but the controller's own
/// (`admin_*`).
pub(crate) fn applies(state: &AppState, op: &str) -> bool {
    if op.starts_with("admin_") || !state.backend().access_delay_enabled() {
        return false;
    }
    current_principal().is_some_and(|principal| is_patient_context(state, &principal))
}

/// Whether `principal` is the person's own access: a SMART `patient/` grant
/// (master08 §Resource Scopes), or the configured ABAC patient claim.
///
/// A token carrying either is judged the person's, whatever else it carries.
fn is_patient_context(state: &AppState, principal: &Principal) -> bool {
    // NOTE: EHDS Art. 3(3), our own design: a token that names the patient is read as the
    // person's own access, so a mixed token is withheld from rather than shown too early.
    if principal
        .scopes
        .iter()
        .any(|scope| scope.starts_with("patient/"))
    {
        return true;
    }
    let authz = state.authz();
    let claim = authz
        .as_deref()
        .and_then(|handle| handle.abac())
        .and_then(|abac| abac.patient_claim.as_deref());
    claim.is_some_and(|claim| {
        ferrohealth_smart::roles::claim_text(&principal.claims, claim).is_some()
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
    for (raw, vo_id) in super::restriction::named_objects(parts) {
        let withheld = state
            .backend()
            .is_object_withheld(vo_id)
            .await
            .map_err(|error| Box::new(RestError::from(error).into_response()))?;
        if withheld {
            return Ok(Some(super::restriction::object_not_found(op, raw)));
        }
    }
    Ok(None)
}
