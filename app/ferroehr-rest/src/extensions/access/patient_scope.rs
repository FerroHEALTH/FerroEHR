// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ABAC patient scope on the EHRs an operation names in its body rather
//! than its path. No openEHR spec governs the patient scope: our own
//! extension.

use axum::response::Response;
use ferroehr::ids::EhrId;

use crate::extensions::access::authn::current_principal;
use crate::extensions::access::pep::{
    forbidden_unauthenticated, resolve_patient_claim, subject_gate,
};
use crate::state::AppState;

/// Hold every EHR an operation names in its body, such as the manifest of
/// `POST /message/export`, to the caller's patient claim, before any of their
/// content is read: the pre-check reads only the path `ehr_id`, so the first
/// EHR outside the claim refuses the whole operation with the `403` its path
/// route would answer.
///
/// # Errors
/// A ready `403` on a refusal or a missing claim, and the `503` or `500` of a
/// failed subject lookup.
pub(crate) async fn each(state: &AppState, ehr_ids: &[EhrId]) -> Result<(), Box<Response>> {
    let Some(handle) = state.authz() else {
        return Ok(());
    };
    let Some(abac) = handle.abac() else {
        return Ok(());
    };
    let principal = current_principal().ok_or_else(|| {
        Box::new(forbidden_unauthenticated(
            "authentication required for attribute-based access control",
        ))
    })?;
    let patient = resolve_patient_claim(abac, &principal).map_err(Box::new)?;
    for ehr_id in ehr_ids {
        subject_gate(
            abac,
            &principal,
            patient.as_deref(),
            Some(&ehr_id.to_string()),
        )
        .await
        .map_err(Box::new)?;
    }
    Ok(())
}
