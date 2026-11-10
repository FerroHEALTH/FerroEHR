// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The status each refusal outcome answers with.

use crate::extensions::access::authn::AuthError;

/// RFC 9110 §15.6.4: an unreachable issuer means the server cannot decide,
/// which is a 503 — never a 401 telling a caller its valid token was
/// rejected.
#[test]
fn unreachable_issuer_is_503_not_401() {
    let api = AuthError::KeyResolution.to_api_error();
    assert_eq!(api.status(), http::StatusCode::SERVICE_UNAVAILABLE);
}

/// A malformed header is a request defect (400), not a credential decision
/// (401) — RFC 6750 §3.1 `invalid_request`.
#[test]
fn malformed_authorization_header_is_400_invalid_request() {
    let api = AuthError::MalformedRequest("bad".to_owned()).to_api_error();
    assert_eq!(api.status(), http::StatusCode::BAD_REQUEST);
}

#[test]
fn verification_unavailable_is_500_not_401() {
    // A failed blocking verification is a SERVER fault: it must surface as
    // 500 Internal, never as 401 — a server failure misreported as a client
    // error is the silent-wrong-answer class this mapping exists to prevent.
    let err = AuthError::VerificationUnavailable("task panicked".to_owned());
    assert_eq!(
        err.to_api_error().status(),
        http::StatusCode::INTERNAL_SERVER_ERROR
    );
}
