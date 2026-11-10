// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The purposes of use and the professional roles an access token states, and
//! the refusal of a request whose declared purpose the token contradicts.
//!
//! An issuer's `[[auth.attributes]]` table reads them from the token: the IHE
//! IUA `subject_role` and `purpose_of_use` claims under `extensions.ihe_iua`
//! (IHE ITI TF-2 §3.71.4.2.2.1.1) and the `purpose_of_use` of each RFC 9396
//! `authorization_details` entry (RFC 9396 §2). The authorization server
//! attested them, so where the token and the `[audit] purpose_header` both
//! state a purpose and the header's is not among the token's, the request is
//! refused: our own design (#5323), as no openEHR spec governs the purpose of
//! use.

use axum::extract::Request;
use axum::response::Response;
use ferroehr::system_log::event::{Coding, TokenAttributes};
use ferrohealth_smart::federation::Code;

use super::{AuthError, AuthLayer, Principal};

/// What the access token of `principal` states, or `None` for a caller whose
/// issuer has no `[[auth.attributes]]` table or whose token states neither a
/// purpose nor a role.
#[must_use]
pub(crate) fn token_attributes(principal: &Principal) -> Option<TokenAttributes> {
    let attributes = principal.access.caller()?.attributes()?;
    let coding = |code: &Code| Coding {
        system: code.system.clone(),
        code: code.code.clone(),
    };
    let token = TokenAttributes {
        purposes: attributes.purposes().iter().map(coding).collect(),
        roles: attributes.roles().iter().map(coding).collect(),
    };
    (!token.purposes.is_empty() || !token.roles.is_empty()).then_some(token)
}

/// The refusal of a request whose `[audit] purpose_header` names a purpose of
/// use the access token, which states purposes, does not state; `None` when
/// the token states none, the header is absent, or the token states the
/// header's.
///
/// The refusal names the caller, so its failed-authentication record does.
pub(super) fn mismatch(
    layer: &AuthLayer,
    principal: &Principal,
    req: &Request,
) -> Option<Response> {
    let token = token_attributes(principal)?;
    if token.purposes.is_empty() {
        return None;
    }
    let header = layer.service.audit_purpose_header()?;
    let declared = req.headers().get(header)?;
    // NOTE: a header that is not visible ASCII states no code, so it can agree with none.
    let agrees = declared
        .to_str()
        .is_ok_and(|declared| token.states_purpose(declared.trim()));
    if agrees {
        return None;
    }
    let refusal = AuthError::Forbidden(format!(
        "the purpose of use the {header} header states is not one the access token states"
    ));
    Some(super::refusal::refused(
        &layer.authenticator,
        req.headers(),
        &refusal,
        Some(principal.clone()),
    ))
}
