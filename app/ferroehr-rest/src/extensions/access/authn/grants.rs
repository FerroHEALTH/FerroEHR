// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The policy grants of every authenticated request: the caller's
//! capabilities resolved from this replica's copy of the grant rows, and the
//! request run inside them.
//!
//! A system client is resolved by the issuer of its token and its
//! `client_id` to the `ACTOR` carrying that client identifier, and a
//! caller the login binding bound to a party by that party; every other
//! caller has none and reads nothing. The copy is read only when a
//! notification or a lost listener asks for it, so a request adds no
//! transaction; a store that cannot answer is the server's failure and
//! answers `503`. A request whose own commit changed a grant reads the copy
//! again before it answers, so the caller's next request runs under the
//! change. Nothing reads the capabilities to decide a request yet; a check
//! reads them through [`Capabilities::current`]. No openEHR spec gives a
//! capability an authorization meaning: our own design.

use std::future::Future;
use std::sync::Arc;

use axum::response::Response;
use ferroehr::party_binding::grants::capabilities::{Actor, Capabilities, with_capabilities};
use ferroehr::party_binding::grants::live::tracking;
use ferroehr::party_binding::grants::operation::Operation;
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
        (CallerKind::System, Some(issuer), Some(client_id), _) => {
            Actor::Client { issuer, client_id }
        }
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

/// Whether the caller of `granted` is a client whose `ACTOR` holds a grant of
/// `operation`, as a backend client's `query_system_wide` or the
/// DEMOGRAPHIC API's `access_demographic`: a grant to a party the login is
/// bound to is no client's.
pub(super) fn client_holds(granted: &Capabilities, operation: Operation) -> bool {
    granted.client().is_some() && granted.holds(operation)
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
