// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Per-route-family request-body limits.
//!
//! The `tower-http` `RequestBodyLimitLayer` on the outer stack is a single
//! ceiling for the whole tree. This module adds the tier the ceiling cannot
//! express: the ordinary clinical surface accepts
//! [`BodyLimits::body_bytes`](ferroehr::config::server::BodyLimits::body_bytes),
//! while the routes that accept bulk by design — operational-template upload,
//! EHR-Extract import, TDD import — accept
//! [`BodyLimits::bulk_body_bytes`](ferroehr::config::server::BodyLimits::bulk_body_bytes).
//! Nesting cannot do this: an outer layer's tighter limit would already have
//! refused the request before an inner, more permissive one saw it. So the
//! ceiling stays outermost at the widest tier, and this middleware applies the
//! narrower tier per matched path.
//!
//! It also buffers. Buffering here rather than at the extractor is what makes
//! the refusal a real `413`: an over-limit body without a `Content-Length` (a
//! chunked upload) can only be detected while reading it, and the dispatcher
//! that reads it downstream has no seam to answer a status from. Nothing is
//! paid for this — every group dispatcher already collects the whole body into
//! `Bytes` before handling it.
//!
//! No openEHR spec bounds a request body — our own design. The status is
//! RFC 9110 §15.5.14's, admitted by ITS-REST
//! `specifications/docs/overview/Requests_and_responses.md` §HTTP status codes
//! as an additional, non-conflicting code.

use axum::body::Body;
use axum::extract::Request;
use axum::extract::State;
use axum::middleware::Next;
use axum::response::Response;
use ferroehr::config::server::BodyLimits;
use http::StatusCode;
use http::header::CONTENT_LENGTH;

use crate::overview::error::status_error_response;
use crate::state::AppState;

/// The path fragments that select the bulk tier, matched against the request
/// path after the ITS-REST base path.
///
/// Template upload is included because an operational template is authored
/// per deployment and the published corpus is only evidence of typical size,
/// not a bound; the two `/message` families import a whole EHR or a batch of
/// documents.
const BULK_PATH_FRAGMENTS: [&str; 3] = ["/definition/template", "/message/import", "/message/tdd"];

/// Returns the body limit that applies to `path`.
fn limit_for(path: &str, limits: &BodyLimits) -> usize {
    if BULK_PATH_FRAGMENTS
        .iter()
        .any(|fragment| path.contains(fragment))
    {
        limits.bulk_body_bytes
    } else {
        limits.body_bytes
    }
}

/// Refuses a request whose body exceeds its route family's limit.
///
/// A declared `Content-Length` over the limit is refused without reading a
/// byte. Otherwise the body is collected under the limit and handed on already
/// buffered, so an over-limit chunked upload is refused here instead of
/// arriving downstream as a silently truncated body.
pub(crate) async fn middleware(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Response {
    let limits = state.config().server.limits;
    let limit = limit_for(request.uri().path(), &limits);

    let declared = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.parse::<usize>().ok());
    if declared.is_some_and(|length| length > limit) {
        return too_large(limit);
    }

    let (parts, body) = request.into_parts();
    match axum::body::to_bytes(body, limit).await {
        Ok(bytes) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        Err(_) => too_large(limit),
    }
}

/// The `413` refusal in the openEHR `{ error, message }` shape.
///
/// The message names the limit and the configuration key that raises it, and
/// nothing else — an operator can act on it and a caller learns no internal
/// detail (the OWASP REST Security Cheat Sheet's error-hygiene control).
fn too_large(limit: usize) -> Response {
    status_error_response(
        StatusCode::PAYLOAD_TOO_LARGE,
        &format!(
            "request body exceeds the {limit}-byte limit for this route \
             (raise `server.limits.body_bytes` or `server.limits.bulk_body_bytes`)"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clinical_paths_take_the_standard_tier() {
        let limits = BodyLimits::default();
        assert_eq!(
            limit_for("/ferroehr/rest/openehr/v1/ehr/x/composition", &limits),
            limits.body_bytes
        );
        assert_eq!(
            limit_for("/ferroehr/rest/openehr/v1/query/aql", &limits),
            limits.body_bytes
        );
    }

    #[test]
    fn bulk_paths_take_the_bulk_tier() {
        let limits = BodyLimits::default();
        for path in [
            "/ferroehr/rest/openehr/v1/definition/template/adl1.4",
            "/ferroehr/rest/openehr/v1/message/import",
            "/ferroehr/rest/openehr/v1/message/import/abc",
            "/ferroehr/rest/openehr/v1/message/tdd/abc/batch",
        ] {
            assert_eq!(
                limit_for(path, &limits),
                limits.bulk_body_bytes,
                "{path} must take the bulk tier"
            );
        }
    }

    /// The outer ceiling must never be tighter than any tier it fronts, or a
    /// bulk request would be refused before the bulk tier applied.
    #[test]
    fn the_ceiling_covers_every_tier() {
        let limits = BodyLimits::default();
        assert!(limits.ceiling() >= limits.body_bytes);
        assert!(limits.ceiling() >= limits.bulk_body_bytes);

        let inverted = BodyLimits {
            body_bytes: 99,
            bulk_body_bytes: 1,
        };
        assert_eq!(inverted.ceiling(), 99);
    }
}
