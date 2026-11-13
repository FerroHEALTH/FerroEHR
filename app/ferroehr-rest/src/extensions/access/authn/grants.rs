// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy grants of every authenticated request: the caller's
//! capabilities resolved from this replica's copy of the grant rows, and the
//! request run inside them.
//!
//! A system client, and a verified external caller of a federation partner,
//! is resolved by the issuer of its token and its `client_id` to the `ACTOR`
//! carrying that client identifier, and a
//! caller the login binding bound to a party by that party; every other
//! caller has none and reads nothing. The copy is read only when a
//! notification or a lost listener asks for it, so a request adds no
//! transaction; a store that cannot answer is the server's failure and
//! answers `503`. A request whose own commit changed a grant reads the copy
//! again before it answers, so the caller's next request runs under the
//! change. A check reads them through [`Capabilities::current`], and the
//! admission of an external caller reads its federation agreement from them. No openEHR spec gives a
//! capability an authorization meaning: our own design.

use std::future::Future;
use std::sync::Arc;

use axum::response::Response;
use ferroehr::party_binding::grants::capabilities::{Actor, Capabilities, with_capabilities};
use ferroehr::party_binding::grants::live::tracking;
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::Outcome;

use super::{AuthError, AuthLayer, Principal};

/// The capabilities of the caller of `principal`, after reading the grant
/// rows when this replica's copy is stale.
///
/// # Errors
/// [`AuthError::PartyUnavailable`] when the copy is stale and the party store
/// cannot answer.
pub(super) async fn resolve(
    layer: &AuthLayer,
    principal: &Principal,
) -> Result<Arc<Capabilities>, AuthError> {
    let Some(caller) = principal.access.caller() else {
        return Ok(Arc::default());
    };
    // NOTE: the local party source names each party by its container's UUID,
    // so another id names no party of this store and no grant.
    let bound = caller
        .party()
        .and_then(Outcome::bound)
        .and_then(|bound| bound.party.id().parse::<uuid::Uuid>().ok());
    let actor = match (caller.kind(), caller.issuer(), caller.client_id(), bound) {
        (
            CallerKind::System
            | CallerKind::ExternalPerson(_)
            | CallerKind::ExternalOrganisation(_),
            Some(issuer),
            Some(client_id),
            _,
        ) => Actor::Client { issuer, client_id },
        (_, _, _, Some(party)) => Actor::Party(party),
        _ => return Ok(Arc::default()),
    };
    let grants = layer.service.grants();
    if let Err(error) = grants.refresh_if_stale().await {
        tracing::warn!(
            error = %ferroehr::error::ErrorChain::new(&error),
            "the policy grants could not be read"
        );
        return Err(AuthError::PartyUnavailable { retry_after: None });
    }
    Ok(Arc::new(grants.capabilities(actor, jiff::Timestamp::now())))
}

/// Resolves the capabilities of the caller of `principal` ([`resolve`]) and
/// admits it under them ([`super::declared::Declared::admit`]), so
/// `principal` carries the agreement an external caller was admitted under;
/// returns the capabilities.
///
/// # Errors
/// The response refusing a caller whose capabilities cannot be read, or an
/// external caller outside its agreement, recorded as its failed
/// authentication.
pub(super) async fn admit(
    layer: &AuthLayer,
    principal: &mut Principal,
    headers: &http::HeaderMap,
) -> Result<Arc<Capabilities>, Box<Response>> {
    let auth = &layer.authenticator;
    let granted = resolve(layer, principal)
        .await
        .map_err(|e| Box::new(super::refusal::response(auth, headers, &e)))?;
    let trust = layer.service.grants().federation();
    match auth
        .declared
        .admit(principal.access.clone(), &granted, &trust)
    {
        Ok(admitted) => {
            principal.access = admitted;
            Ok(granted)
        }
        Err(AuthError::KindRefused { code, message, .. }) => {
            let refused = AuthError::KindRefused {
                code,
                message,
                principal: Some(Box::new(principal.clone())),
            };
            Err(Box::new(super::refusal::refusal_of(
                auth, headers, &refused,
            )))
        }
        Err(e) => Err(Box::new(super::refusal::refusal_of(auth, headers, &e))),
    }
}

/// Runs `request` inside the caller's capabilities `granted`, reads the grant
/// rows again when its own commit changed one, and carries `granted` on the
/// response.
pub(super) async fn scoped<F: Future<Output = Response>>(
    layer: &AuthLayer,
    granted: Arc<Capabilities>,
    request: F,
) -> Response {
    let (mut resp, changed) = tracking(with_capabilities(Arc::clone(&granted), request)).await;
    if changed && let Err(error) = layer.service.grants().refresh().await {
        // NOTE: no specification governs this: our own design; the commit
        // notified every replica, and a copy that could not be read is read
        // again on the next request.
        tracing::warn!(
            error = %ferroehr::error::ErrorChain::new(&error),
            "the policy grants could not be read after a grant change"
        );
    }
    resp.extensions_mut().insert(granted);
    resp
}
