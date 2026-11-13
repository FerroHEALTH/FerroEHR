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
//! transaction, and the replica's refresher reads it on each notification for
//! the commits of every other request; a store that cannot answer is the
//! server's failure and answers `503`. A request whose own commit changed a
//! grant reads the copy again before it answers, so the caller's next request
//! runs under the change. A Basic user holds the roles and scopes its
//! `hold_roles` and `hold_scopes` grants name, and the first administrator
//! `admin_role` and the product scope ([`basic`]), resolved before any gate
//! reads them, the admission of an external caller reads its federation
//! agreement from them ([`admit`]), and a check reads the rest through
//! [`Capabilities::current`]. No openEHR spec gives a capability an
//! authorization meaning: our own design.

use std::future::Future;
use std::sync::Arc;

use axum::response::Response;
use ferroehr::party_binding::grants::capabilities::{Actor, Capabilities, with_capabilities};
use ferroehr::party_binding::grants::live::{LiveGrants, tracking};
use ferroehr::party_binding::grants::operation::{Operation, ROLE, SCOPE};
use ferrohealth_smart::kind::CallerKind;
use ferrohealth_smart::party::Outcome;
use ferrohealth_smart::resource::{Access, Via};

use super::build::RoleIssuers;
use super::{AuthError, AuthLayer, Principal, upper_cased};

/// The capabilities of the caller of `principal` under `grants`, after
/// reading the grant rows when this replica's copy is stale.
///
/// # Errors
/// [`AuthError::PartyUnavailable`] when the copy is stale and the party store
/// cannot answer.
pub(crate) async fn resolve(
    grants: &LiveGrants,
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
    if let Err(error) = grants.refresh_if_stale().await {
        tracing::warn!(
            error = %ferroehr::error::ErrorChain::new(&error),
            "the policy grants could not be read"
        );
        return Err(AuthError::PartyUnavailable { retry_after: None });
    }
    Ok(Arc::new(grants.capabilities(actor, jiff::Timestamp::now())))
}

/// The principal of `principal` under its capabilities `granted`: a Basic
/// user holding its granted roles and scopes ([`basic`]), and an external
/// caller admitted under its federation agreement ([`admit`]).
///
/// # Errors
/// The response refusing an external caller outside its agreement.
pub(super) fn bound(
    layer: &AuthLayer,
    principal: Principal,
    granted: &Capabilities,
    headers: &http::HeaderMap,
) -> Result<Principal, Box<Response>> {
    let mut principal = basic(principal, granted, &layer.authenticator.role_issuers);
    admit(layer, &mut principal, granted, headers)?;
    Ok(principal)
}

/// Admits the caller of `principal` under its capabilities `granted`
/// ([`super::declared::Declared::admit`]), so `principal` carries the
/// agreement an external caller was admitted under.
///
/// # Errors
/// The response refusing an external caller outside its agreement, recorded
/// as its failed authentication.
fn admit(
    layer: &AuthLayer,
    principal: &mut Principal,
    granted: &Capabilities,
    headers: &http::HeaderMap,
) -> Result<(), Box<Response>> {
    let auth = &layer.authenticator;
    let trust = layer.service.grants().federation();
    match auth
        .declared
        .admit(principal.access.clone(), granted, &trust)
    {
        Ok(admitted) => {
            principal.access = admitted;
            Ok(())
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
pub(crate) async fn scoped<F: Future<Output = Response>>(
    grants: &LiveGrants,
    granted: Arc<Capabilities>,
    request: F,
) -> Response {
    let (mut resp, changed) = tracking(with_capabilities(Arc::clone(&granted), request)).await;
    if changed && let Err(error) = grants.refresh().await {
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

/// The principal of the Basic user `principal` holding the roles and scopes
/// its `hold_roles` and `hold_scopes` grants in `granted` name, and, for the
/// first administrator, `admin_role` and the product scope; any other
/// principal unchanged.
///
/// A role `issuers` holds privileged is never held through a grant: a
/// privileged role comes from a `role_issuers` token, the first
/// administrator or an operation of its own, under the grant ceiling.
pub(super) fn basic(
    principal: Principal,
    granted: &Capabilities,
    issuers: &RoleIssuers,
) -> Principal {
    let texts = |operation: Operation, code: &str| -> Vec<String> {
        granted
            .of(operation)
            .flat_map(|grant| grant.credentials().texts(code))
            .map(|text| text.trim().to_owned())
            .filter(|text| !text.is_empty())
            .collect()
    };
    let Some(caller) = principal.access.caller().filter(|c| c.via() == Via::Basic) else {
        return principal;
    };
    let mut roles: Vec<String> = upper_cased(&texts(Operation::HoldRoles, ROLE))
        .into_iter()
        .filter(|role| !issuers.is_privileged(role))
        .collect();
    let mut scopes = texts(Operation::HoldScopes, SCOPE);
    if let Some(admin) = issuers.first_admin(caller) {
        roles.push(admin.role.clone());
        scopes.push(admin.scope.clone());
    }
    if roles.is_empty() && scopes.is_empty() {
        return principal;
    }
    let mut held = principal.roles.clone();
    for role in &roles {
        if !held.contains(role) {
            held.push(role.clone());
        }
    }
    let holding = caller
        .clone()
        .with_granted_scope(&scopes)
        .with_granted_roles(&roles);
    Principal::new(Access::Caller(Arc::new(holding)), held, principal.honoured)
}
