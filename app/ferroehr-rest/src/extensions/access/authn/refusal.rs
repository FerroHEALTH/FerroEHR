// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The response a refused authentication answers with: the problem and its
//! challenge, the auth-failure count, the log line, and the refused caller
//! and binding record the access log reads.

use axum::response::{IntoResponse, Response};
use http::{HeaderValue, StatusCode, header};

use super::{AuthError, Authenticator, Principal, record};
use crate::overview::error::RestError;

/// Counts one refusal, tagged by the mechanism that produced it and the status
/// the caller receives.
pub(super) fn count_auth_failure(mechanism: &'static str, status: &'static str) {
    ferroehr::telemetry::metrics::metrics().auth_failures.add(
        1,
        &[
            opentelemetry::KeyValue::new("mechanism", mechanism),
            opentelemetry::KeyValue::new("status", status),
        ],
    );
}

/// The response refusing a request whose authentication failed with `e`,
/// naming the caller a login-binding or caller-kind refusal refused.
pub(crate) fn refusal_of(
    auth: &Authenticator,
    headers: &header::HeaderMap,
    e: &AuthError,
) -> Response {
    let principal = match e {
        AuthError::PartyRefused {
            principal: Some(principal),
            ..
        }
        | AuthError::KindRefused {
            principal: Some(principal),
            ..
        } => Some(Principal::clone(principal)),
        _ => None,
    };
    refused(auth, headers, e, principal)
}

/// The response refusing `principal` with `e`, carrying the refused caller
/// and the binding record of a login-binding refusal for the access log.
pub(super) fn refused(
    auth: &Authenticator,
    headers: &header::HeaderMap,
    e: &AuthError,
    principal: Option<Principal>,
) -> Response {
    let mut resp = response(auth, headers, e);
    let record = match e {
        AuthError::PartyRefused {
            code, party, named, ..
        } => record::refused(code, party.as_deref(), named.as_ref(), principal.as_ref()),
        AuthError::KindRefused { code, .. } => {
            record::refused(code, None, None, principal.as_ref())
        }
        _ => None,
    };
    if let Some(record) = record {
        resp.extensions_mut().insert(record);
    }
    if let Some(principal) = principal {
        resp.extensions_mut().insert(principal);
    }
    resp
}

/// Builds the response for a refused authentication, metered and logged.
///
/// A refusal that presented no credential is routine; a presented-and-rejected
/// one is the operator's signal. Neither record carries the token or a claim
/// value. ITS-REST §Authentication and authorization makes the
/// `WWW-Authenticate` challenge a MUST on a `401`, and RFC 6750 §3 carries one
/// on a bearer `400` and `403` too, where it names what the client did wrong.
pub(super) fn response(
    auth: &Authenticator,
    headers: &header::HeaderMap,
    e: &AuthError,
) -> Response {
    let api = e.to_api_error();
    let status = api.status();
    let mechanism = scheme_label(headers);
    count_auth_failure(
        mechanism,
        if status == StatusCode::FORBIDDEN {
            "403"
        } else {
            "401"
        },
    );
    let reason = e.label();
    if matches!(e, AuthError::MissingCredentials) {
        tracing::debug!(mechanism, reason, "authentication refused");
    } else {
        tracing::warn!(
            mechanism,
            reason,
            detail = %ferroehr::error::ErrorChain::new(e),
            "authentication refused"
        );
    }
    let mut resp = RestError::from(api).into_response();
    resp.extensions_mut()
        .insert(crate::system_log::refusals::Reason(reason));
    if let AuthError::PartyUnavailable {
        retry_after: Some(after),
    } = e
    {
        resp.headers_mut()
            .insert(header::RETRY_AFTER, HeaderValue::from(after.as_secs()));
    }
    if matches!(
        status,
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN | StatusCode::BAD_REQUEST
    ) {
        resp.headers_mut()
            .insert(header::WWW_AUTHENTICATE, auth.challenge(Some(e)));
    }
    resp
}

/// The `mechanism` metric label derived from the request's `Authorization`
/// scheme (for failures where no principal was established).
fn scheme_label(headers: &http::HeaderMap) -> &'static str {
    match headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.split_whitespace().next())
        .map(str::to_ascii_lowercase)
        .as_deref()
    {
        Some("basic") => "basic",
        Some("bearer") => "bearer",
        _ => "none",
    }
}
