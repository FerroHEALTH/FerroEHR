// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The gates the authentication middleware runs over an authenticated
//! caller before the handler: RBAC, then the SMART on openEHR requirement of
//! the matched route, and the response of a patient-data request whose
//! assurance is too low. Role semantics are our own design; the SMART grammar is
//! the ITS-REST SMART on openEHR draft (`crate::smart::routes`).

use axum::extract::{MatchedPath, OriginalUri, Request};
use axum::response::{IntoResponse, Response};
use http::header;
use openehr_its::rest::runtime::ApiError;

use crate::extensions::access::authz::AuthzHandle;
use crate::extensions::access::authz::classify::OperationClass;
use crate::extensions::access::authz::roles::RbacDecision;
use crate::overview::error::RestError;
use crate::smart::routes::Need;

use ferroehr::system_log::event::ActorAuthentication;

use super::assurance::{PatientDataGate, PatientDataRefusal};
use super::{AuthError, AuthLayer, AuthMethod, Principal, count_auth_failure, mechanism_label};

/// Runs the RBAC gate over an authenticated caller: the matched operation's
/// class against the caller's roles, then the read-only restriction.
///
/// A bearer token whose SMART on openEHR resource scopes are all `system/`
/// scopes passes the clinical class without a role: its scopes decide access,
/// in [`smart_refusal`]. A `user/` or `patient/` token belongs to a person and
/// keeps the role gate, as does a Basic user.
///
/// A principal carrying the configured read-only role is refused on every write
/// operation, overriding any grant. Both denials produce the same `403`,
/// attributed to the caller so the ATNA audit layer records it. No openEHR spec
/// governs role semantics — our own design/extension.
pub(super) fn rbac_refusal(
    layer: &AuthLayer,
    req: &Request,
    principal: &Principal,
) -> Option<Response> {
    let rbac = layer.authz.as_deref().and_then(AuthzHandle::rbac)?;
    let matched = req
        .extensions()
        .get::<MatchedPath>()
        .map(|m| m.as_str().to_owned());
    let class = rbac.class_for(req.method(), matched.as_deref());
    let backend = principal.method == AuthMethod::Bearer
        && principal
            .access
            .caller()
            .is_some_and(|caller| crate::smart::holds_only_system_scopes(&caller.openehr_scopes()));
    let gated = if backend && class == OperationClass::Clinical {
        RbacDecision::Allow
    } else {
        rbac.decide(class, &principal.roles)
    };
    let decision = match gated {
        RbacDecision::Deny(reason) => RbacDecision::Deny(reason),
        RbacDecision::Allow => {
            let is_write = rbac.is_write_for(req.method(), matched.as_deref());
            rbac.decide_readonly(is_write, &principal.roles)
        }
    };
    let RbacDecision::Deny(reason) = decision else {
        return None;
    };
    count_auth_failure(mechanism_label(principal.method), "403");
    let mut resp = RestError::from(ApiError::Forbidden(reason)).into_response();
    resp.extensions_mut().insert(principal.clone());
    Some(resp)
}

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
    let (matched, uri) = route(req);
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

/// The route `req` matched, as its full template, and the URI it was sent
/// to, with the base path the nested API router strips.
pub(super) fn route(req: &Request) -> (Option<&str>, &http::Uri) {
    let matched = req
        .extensions()
        .get::<MatchedPath>()
        .map(MatchedPath::as_str);
    let uri = req
        .extensions()
        .get::<OriginalUri>()
        .map_or_else(|| req.uri(), |original| &original.0);
    (matched, uri)
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
