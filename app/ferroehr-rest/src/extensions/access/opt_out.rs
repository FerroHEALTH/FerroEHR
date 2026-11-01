// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 10 opt-out from access through the access services, at the
//! request seam.
//!
//! Regulation (EU) 2025/327 Art. 10(1) lets a Member State's law give a person
//! the right to opt out from access to their data "through the electronic
//! health data access services referred to in Articles 4 and 12", and
//! Art. 10(2) lets "a healthcare provider or health professional" reach the
//! data where processing "is necessary in order to protect the vital interests
//! of the data subject or of another natural person"
//! (`docs/law/eu/ehds/text.html`). No openEHR spec governs an opt-out: our own
//! design/extension.
//!
//! A caller holding one of `[authz.rbac] access_service_roles` is an
//! access-service caller. Its requests run with the opt-out enforced,
//! published to the platform layer through
//! `ferroehr::system_log::access_context`, where the AQL builder, the exports,
//! the subject lookup and the write guard treat an opted-out EHR as absent.
//! Here, before dispatch, a route naming an opted-out EHR, or an object in
//! one, answers `404` as an unknown one does. An access-service caller that
//! declares one of `[audit] emergency_purpose_codes` and does not act for the
//! person is not held to it, and its access to an opted-out EHR is recorded as
//! an override. Every other caller, and the controller's own `admin_*`
//! surfaces, read the record as before.

use axum::response::{IntoResponse, Response};
use ferroehr::ids::EhrId;
use ferroehr::service::status::ServiceStatus;
use uuid::Uuid;

use super::authn::{Principal, current_principal};
use crate::api::RequestParts;
use crate::overview::error::RestError;
use crate::state::AppState;

/// How the EHDS Art. 10 opt-out bears on one request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OptOut {
    /// The caller is no access-service caller, or the operation is the
    /// controller's own: the opt-out does not bear on it.
    Untouched,
    /// An access-service caller: an opted-out EHR reads as absent.
    Enforced,
    /// An access-service caller under the Art. 10(2) vital-interest override:
    /// it reads the EHR, and its access is recorded as an override.
    Overridden,
}

/// How the opt-out bears on `op` from the current caller.
pub(crate) fn applies(state: &AppState, op: &str) -> OptOut {
    if op.starts_with("admin_") {
        return OptOut::Untouched;
    }
    let Some(principal) = current_principal() else {
        return OptOut::Untouched;
    };
    if !is_access_service(state, &principal) {
        return OptOut::Untouched;
    }
    let emergency = state
        .backend()
        .is_emergency_purpose(ferroehr::system_log::access_context::current_purpose().as_deref());
    // NOTE: EHDS Art. 10(2) admits "a healthcare provider or health professional", so a
    // caller acting for the person is never admitted by the override.
    if emergency && !super::delay::acts_for_the_person(&principal) {
        OptOut::Overridden
    } else {
        OptOut::Enforced
    }
}

/// Whether `principal` holds one of `[authz.rbac] access_service_roles`.
pub(crate) fn is_access_service(state: &AppState, principal: &Principal) -> bool {
    let declared = &state.config().rbac.access_service_roles;
    principal.roles.iter().any(|held| {
        declared
            .iter()
            .map(|role| role.trim())
            .any(|role| !role.is_empty() && held.eq_ignore_ascii_case(role))
    })
}

/// Answer, as an unknown target would be answered, a request of an
/// access-service caller whose path names an opted-out EHR or an object in
/// one; under the override, record the access to that EHR instead.
///
/// `Ok(None)` lets the request proceed.
///
/// # Errors
/// A ready `500` response when the mark cannot be read, and the `503` of an
/// override record refused under `[audit] fail_mode = "closed"`: an access
/// decision never proceeds on an unknown state.
pub(crate) async fn refuse_opted_out(
    state: &AppState,
    op: &str,
    parts: &RequestParts,
    opt_out: OptOut,
) -> Result<Option<Response>, Box<Response>> {
    if opt_out == OptOut::Untouched {
        return Ok(None);
    }
    let fault = |error| Box::new(RestError::from(error).into_response());
    if let Some(ehr_id) = parts
        .path
        .get("ehr_id")
        .and_then(|raw| Uuid::parse_str(raw).ok())
        .map(EhrId)
        && state
            .backend()
            .is_ehr_opted_out(ehr_id)
            .await
            .map_err(fault)?
    {
        if opt_out == OptOut::Overridden {
            return override_recorded(state, ehr_id).map_err(fault);
        }
        let message = if op.contains("ehr_status") {
            format!("EHR_STATUS for EHR {ehr_id}")
        } else {
            format!("EHR {ehr_id}")
        };
        return Ok(Some(not_found(message)));
    }
    for (raw, vo_id) in super::restriction::named_objects(parts) {
        let Some(owner) = state
            .backend()
            .opted_out_owner(vo_id)
            .await
            .map_err(fault)?
        else {
            continue;
        };
        if opt_out == OptOut::Overridden {
            return override_recorded(state, owner).map_err(fault);
        }
        return Ok(Some(super::restriction::object_not_found(op, raw)));
    }
    Ok(None)
}

/// Record the Art. 10(2) override of the opt-out of `ehr_id` and let the
/// request proceed.
fn override_recorded(
    state: &AppState,
    ehr_id: EhrId,
) -> Result<Option<Response>, ferroehr::service::status::StatusError> {
    state.backend().record_access_opt_out_override(ehr_id)?;
    Ok(None)
}

/// The `404` an unknown EHR answers.
fn not_found(message: String) -> Response {
    RestError::from(ferroehr::service::status::StatusError::new(
        ServiceStatus::EhrIdDoesNotExist,
        message,
    ))
    .into_response()
}
