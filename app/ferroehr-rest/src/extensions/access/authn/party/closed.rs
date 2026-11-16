// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What a login is answered when its issuer's key space is not `active`.
//!
//! A key binds only while its issuer's key space is `active`
//! ([`ferroehr::party_binding::key_space`]). No openEHR spec governs
//! resolving a login to a party: our own design.

use ferroehr::party_binding::fill::LEASE_RENEWAL;
use ferroehr::party_binding::key_space::KeySpaceClosed;
use ferroehr::party_binding::lookup::LookupError;
use ferrohealth_smart::party::source::SourceError;

use crate::extensions::access::authn::AuthError;

/// The answer to a login whose issuer's key space is closed: the `503` with a
/// `Retry-After` of one renewal period while it is pending or filling, so no
/// `PERSON` is created and no representative taken for the patient before the
/// key space is read; and the `403` `refused-unbound` while it retires or no
/// key space records it, whose keys bind no login.
pub(in crate::extensions::access::authn) fn refusal(closed: &KeySpaceClosed) -> AuthError {
    let state = closed.state.map_or("not recorded", |s| s.as_str());
    if closed.opening() {
        tracing::info!(
            issuer = closed.issuer.as_str(),
            state,
            "the login's binding-key issuer is not yet active on every replica"
        );
        return AuthError::PartyUnavailable {
            retry_after: Some(LEASE_RENEWAL),
        };
    }
    tracing::warn!(
        issuer = closed.issuer.as_str(),
        state,
        "the login's binding-key issuer binds no login"
    );
    AuthError::PartyRefused {
        code: "refused-unbound",
        principal: None,
        party: None,
        named: None,
    }
}

/// What a request is answered when the party source could not answer: the
/// [`refusal`] of a closed key space, else the `503` of a store that failed.
pub(in crate::extensions::access::authn) fn unanswered(error: &SourceError) -> AuthError {
    let closed = std::iter::successors(std::error::Error::source(error), |e| e.source())
        .find_map(|e| e.downcast_ref::<LookupError>())
        .and_then(LookupError::closed);
    if let Some(closed) = closed {
        return refusal(closed);
    }
    tracing::warn!(
        error = %ferroehr::error::ErrorChain::new(error),
        "the party source could not answer"
    );
    AuthError::PartyUnavailable { retry_after: None }
}
