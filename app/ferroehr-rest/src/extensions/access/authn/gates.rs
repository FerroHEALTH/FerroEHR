// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The gates the authentication middleware runs over an authenticated
//! caller before the handler: RBAC, then the SMART on openEHR requirement of
//! the matched route, and the response of a patient-data request whose
//! assurance is too low. Role semantics are our own design; the SMART grammar is
//! the ITS-REST SMART on openEHR draft (`crate::smart::routes`).

use axum::extract::{MatchedPath, OriginalUri, Request};
use axum::response::{IntoResponse, Response};
use ferroehr::party_binding::grants::capabilities::Capabilities;
use ferroehr::party_binding::grants::operation::Operation;
use http::header;
use openehr_its::rest::runtime::ApiError;

use crate::extensions::access::authz::AuthzHandle;
use crate::extensions::access::authz::classify::OperationClass;
use crate::extensions::access::authz::roles::RbacDecision;
use crate::overview::error::RestError;
use crate::smart::routes::Need;

use ferroehr::system_log::event::ActorAuthentication;

use super::assurance::{PatientDataGate, PatientDataRefusal};
use super::refusal::count_auth_failure;
use super::{AuthError, AuthLayer, AuthMethod, Principal};

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
/// its template is known. The DEMOGRAPHIC API admits a client whose `ACTOR`
/// the CDR grants `access_demographic` (`granted`), and otherwise needs the
/// product scope; the write of a standalone relationship also admits the
/// patient's own credential ([`writes_own_representation`]).
///
/// The API router is nested under the base path, so the request URI this layer
/// sees has the base stripped while the matched template carries it; the path
/// parameters are read from the original URI, which lines up with the template.
pub(super) fn smart_refusal(
    layer: &AuthLayer,
    req: &Request,
    principal: &Principal,
    granted: &Capabilities,
) -> Option<Response> {
    let (matched, uri) = route(req);
    let requirement = match layer.smart.need(req.method(), matched, uri.path()) {
        Need::Now(requirement) => requirement,
        Need::Demographic(_) | Need::Representation(_)
            if super::grants::client_holds(granted, Operation::AccessDemographic) =>
        {
            return None;
        }
        Need::Representation(_) if writes_own_representation(principal) => return None,
        Need::Demographic(product) | Need::Representation(product) => product,
        Need::Composition(_) => return None,
    };
    let reason = match crate::smart::routes::check(&requirement, principal) {
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

/// Whether `principal` is the patient themself on a credential naming the
/// EHR of their launch context: such a caller writes the standalone
/// representation of themselves that authorises a proxy, which the party
/// protection holds it to.
pub(super) fn writes_own_representation(principal: &Principal) -> bool {
    let portal = principal
        .represented
        .as_ref()
        .is_some_and(|represented| represented.portal.is_some());
    !portal && principal.access.ehr_id().is_some() && crate::smart::acts_as_person(principal)
}

/// The `PERSON` the patient of `principal` is, when the route of `req`
/// writes a standalone relationship and the patient writes it themself: the
/// subject of the EHR of their launch context, read through `layer`'s
/// service.
///
/// What it reads of `req` is read before it awaits anything, so the future
/// holds no borrow of the request.
///
/// # Errors
/// [`AuthError::PartyUnavailable`] when the clinical store cannot answer.
pub(super) fn patient_person<'a>(
    layer: &'a AuthLayer,
    req: &Request,
    principal: &'a Principal,
) -> impl Future<Output = Result<Option<uuid::Uuid>, AuthError>> + Send + 'a {
    let (matched, uri) = route(req);
    let writes = matches!(
        layer.smart.need(req.method(), matched, uri.path()),
        Need::Representation(_)
    ) && writes_own_representation(principal);
    async move {
        let ehr = principal.access.ehr_id().map(str::parse::<uuid::Uuid>);
        let (true, Some(Ok(ehr))) = (writes, ehr) else {
            return Ok(None);
        };
        layer
            .service
            .ehr_subject_person(ferroehr::ids::EhrId(ehr))
            .await
            .map_err(|error| {
                tracing::warn!(
                    error = %ferroehr::error::ErrorChain::new(&error),
                    "the subject of the patient's EHR could not be read"
                );
                AuthError::PartyUnavailable { retry_after: None }
            })
    }
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

/// The `mechanism` metric label for a known authenticated principal.
fn mechanism_label(method: AuthMethod) -> &'static str {
    match method {
        AuthMethod::Basic => "basic",
        AuthMethod::Bearer => "bearer",
    }
}
