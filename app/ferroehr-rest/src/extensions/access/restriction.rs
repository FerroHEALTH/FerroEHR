// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 8 restriction of access by health professionals, at the
//! request seam.
//!
//! Regulation (EU) 2025/327 Art. 8 lets a person restrict the access of health
//! professionals and healthcare providers to all or parts of their data, and
//! keeps the fact of the restriction "not visible to healthcare providers";
//! Art. 11(5) lets restricted data be reached where necessary to protect the
//! vital interests of the person (`docs/law/eu/ehds/text.html`). No openEHR
//! spec governs restriction of access — our own design/extension.
//!
//! Every operation but the controller's own (`admin_*`) runs with the
//! restriction enforced, published to the platform layer through
//! `ferroehr::system_log::access_context`, where the version reads, the AQL
//! builder, the exports and the write guard treat restricted data as absent.
//! Here, before dispatch, a route that names a restricted EHR or object
//! answers `404` as an unknown one does. A caller holding
//! `[authz.rbac] emergency_access_role` that declares one of
//! `[audit] emergency_purpose_codes` is not held to it, and the access record
//! carries the emergency mark the declared purpose gives it.

use axum::response::{IntoResponse, Response};
use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::status::{ServiceStatus, StatusError};
use uuid::Uuid;

use crate::api::RequestParts;
use crate::extensions::access::authn::current_principal;
use crate::overview::error::RestError;
use crate::state::AppState;

/// The path parameters that name a versioned object, alone or as the head of
/// an `OBJECT_VERSION_ID`.
const OBJECT_PARAMS: [&str; 3] = ["versioned_object_uid", "uid_based_id", "version_uid"];

/// Whether the EHDS Art. 8 restriction holds for `op` from the current caller.
///
/// The controller's own surfaces (`admin_*`) are not a professional's access.
/// An emergency access needs both the configured role and an emergency
/// purpose declared for the request; either alone does not lift the
/// restriction.
pub(crate) fn applies(state: &AppState, op: &str) -> bool {
    if op.starts_with("admin_") {
        return false;
    }
    !emergency_override(state)
}

/// Whether the caller holds `[authz.rbac] emergency_access_role` and declared
/// an emergency purpose of use for this request (EHDS Art. 11(5)).
fn emergency_override(state: &AppState) -> bool {
    let Some(role) = state.config().rbac.emergency_access_role.as_deref() else {
        return false;
    };
    let holds = current_principal().is_some_and(|principal| {
        principal
            .roles
            .iter()
            .any(|held| held.eq_ignore_ascii_case(role))
    });
    holds
        && state.backend().is_emergency_purpose(
            ferroehr::system_log::access_context::current_purpose().as_deref(),
        )
}

/// Answer, as an unknown target would be answered, a request whose path names
/// an EHR or a versioned object under an EHDS Art. 8 access restriction.
///
/// `Ok(None)` lets the request proceed. A refusal is `404` with the status an
/// unknown target gets, and nothing in it names the restriction.
///
/// # Errors
/// A ready `500` response when the marks cannot be read: an access decision
/// never proceeds on an unknown state.
pub(crate) async fn refuse_restricted(
    state: &AppState,
    op: &str,
    parts: &RequestParts,
) -> Result<Option<Response>, Box<Response>> {
    if let Some(ehr_id) = parts
        .path
        .get("ehr_id")
        .and_then(|raw| Uuid::parse_str(raw).ok())
    {
        let restricted = state
            .backend()
            .is_ehr_access_restricted(EhrId(ehr_id))
            .await
            .map_err(|error| Box::new(RestError::from(error).into_response()))?;
        if restricted {
            let message = if op.contains("ehr_status") {
                format!("EHR_STATUS for EHR {ehr_id}")
            } else {
                format!("EHR {ehr_id}")
            };
            return Ok(Some(not_found(ServiceStatus::EhrIdDoesNotExist, message)));
        }
    }
    for (raw, vo_id) in named_objects(parts) {
        let restricted = state
            .backend()
            .is_object_access_restricted(vo_id)
            .await
            .map_err(|error| Box::new(RestError::from(error).into_response()))?;
        if restricted {
            return Ok(Some(object_not_found(op, raw)));
        }
    }
    Ok(None)
}

/// The versioned objects the request path names, each with the raw parameter
/// that named it; a parameter that is no UUID names none.
pub(super) fn named_objects(parts: &RequestParts) -> Vec<(&str, VoId)> {
    OBJECT_PARAMS
        .iter()
        .filter_map(|param| parts.path.get(*param))
        .filter_map(|raw| {
            let head = raw.split("::").next().unwrap_or(raw);
            Uuid::parse_str(head)
                .ok()
                .map(|vo_id| (raw.as_str(), VoId(vo_id)))
        })
        .collect()
}

/// The `404` an unknown versioned object of `op`'s kind answers.
pub(super) fn object_not_found(op: &str, raw: &str) -> Response {
    if op.contains("composition") {
        not_found(
            ServiceStatus::CompositionDoesNotExist,
            format!("COMPOSITION {raw}"),
        )
    } else if op.contains("ehr_status") {
        not_found(
            ServiceStatus::VersionedObjectDoesNotExist,
            format!("EHR_STATUS {raw}"),
        )
    } else if op.contains("directory") || op.contains("folder") {
        not_found(
            ServiceStatus::VersionedObjectDoesNotExist,
            format!("DIRECTORY {raw}"),
        )
    } else {
        not_found(
            ServiceStatus::VersionedObjectDoesNotExist,
            format!("versioned object {raw}"),
        )
    }
}

/// A `404` carrying `status` and `message`, built the way the dispatchers
/// build theirs.
fn not_found(status: ServiceStatus, message: String) -> Response {
    RestError::from(StatusError::new(status, message)).into_response()
}
