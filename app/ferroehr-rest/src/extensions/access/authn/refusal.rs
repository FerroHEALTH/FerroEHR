// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The refusals the authentication middleware answers after a caller is
//! authenticated: a SMART on openEHR requirement the grants do not meet, and
//! a patient-data request whose assurance is too low.

use axum::extract::{MatchedPath, OriginalUri, Request};
use axum::response::{IntoResponse, Response};
use http::header;
use openehr_its::rest::runtime::ApiError;

use ferroehr::system_log::event::ActorAuthentication;

use super::assurance::{PatientDataGate, PatientDataRefusal};
use super::{AuthError, AuthLayer, Principal, count_auth_failure, mechanism_label};
use crate::overview::error::RestError;
use crate::smart::routes::Need;

/// Checks the SMART on openEHR requirement of the matched route, after RBAC.
///
/// A requirement the caller's grants do not meet, or a `patient/` grant whose
/// target is not the EHR of the token's launch context, is a `403` with the
/// `insufficient_scope` challenge (RFC 6750 §3.1) that names no scope,
/// attributed to the caller. A composition operation is checked by the PEP once
/// its template is known.
///
/// The API router is nested under the base path, so the request URI this layer
/// sees has the base stripped while the matched template carries it; the path
/// parameters are read from the original URI, which lines up with the template.
pub(super) fn smart_refusal(
    layer: &AuthLayer,
    req: &Request,
    principal: &Principal,
) -> Option<Response> {
    let matched = req
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str);
    let uri = req
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| req.uri(), |original| &original.0);
    let need = layer.smart.need(req.method(), matched, uri.path());
    let Need::Now(requirement) = need else {
        return None;
    };
    let reason = match crate::smart::routes::check(
        &requirement,
        principal,
        crate::smart::routes::target_ehr(&requirement, matched, uri, req.headers()).as_deref(),
    ) {
        Ok(()) => return None,
        Err(reason) => reason,
    };
    count_auth_failure(mechanism_label(principal.method), "403");
    tracing::debug!(reason, "a SMART on openEHR requirement was not met");
    let mut resp = RestError::from(ApiError::Forbidden(
        "the credential does not permit this request".to_owned(),
    ))
    .into_response();
    resp.headers_mut().insert(
        header::WWW_AUTHENTICATE,
        layer
            .authenticator
            .challenge(Some(&AuthError::Forbidden(String::new()))),
    );
    resp.extensions_mut().insert(principal.clone());
    Some(resp)
}

/// Builds the response for a patient-data request the gate refused, attributed
/// to the caller so the audit layer records who was refused and how they
/// authenticated.
///
/// An insufficient assurance level is a `401` carrying the RFC 9470 §3 step-up
/// challenge (`insufficient_user_authentication` with the accepted
/// `acr_values`).
pub(super) fn patient_data_refusal(
    gate: &PatientDataGate,
    principal: &Principal,
    refusal: PatientDataRefusal,
    actor: ActorAuthentication,
) -> Response {
    let mechanism = mechanism_label(principal.method);
    let mut resp = match refusal {
        PatientDataRefusal::InsufficientAssurance => {
            count_auth_failure(mechanism, "401");
            tracing::warn!(
                mechanism,
                reason = "insufficient_user_authentication",
                "patient-data request refused"
            );
            let mut resp = RestError::from(ApiError::Unauthorized(
                "patient data requires a higher authentication assurance level".to_owned(),
            ))
            .into_response();
            resp.headers_mut()
                .insert(header::WWW_AUTHENTICATE, gate.step_up_challenge());
            resp
        }
    };
    resp.extensions_mut().insert(principal.clone());
    resp.extensions_mut().insert(actor);
    resp
}
