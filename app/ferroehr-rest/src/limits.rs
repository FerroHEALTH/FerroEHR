// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Per-route-family request-body limits.
//!
//! The family stack's `server.body_limit` is a single ceiling for the
//! whole tree. This module adds the tier the ceiling cannot express: the
//! routes that accept bulk by design (operational-template upload and EHR-Extract
//! import) accept the ceiling, while the ordinary clinical surface
//! accepts the narrower
//! [`ApiConfig::body_limit`](ferroehr::config::rest::ApiConfig::body_limit)
//! when one is set. Nesting cannot do this: an outer layer's tighter limit
//! would already have refused the request before an inner, more permissive one
//! saw it. So the ceiling stays outermost at the widest tier, and this
//! middleware applies the narrower tier per matched path.
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
use http::StatusCode;
use http::header::CONTENT_LENGTH;

use crate::overview::error::status_error_response;
use crate::state::AppState;

/// The path fragments that select the bulk tier, matched against the request
/// path after the ITS-REST base path.
///
/// Template upload is included because an operational template is authored
/// per deployment and the published corpus is only evidence of typical size,
/// not a bound; `/message/import` imports a whole EHR.
const BULK_PATH_FRAGMENTS: [&str; 2] = ["/definition/template", "/message/import"];

/// Returns the body limit that applies to `path`: the `ceiling` on a bulk
/// route or when no clinical tier is set, the clinical tier otherwise.
fn limit_for(path: &str, clinical: Option<usize>, ceiling: usize) -> usize {
    let bulk = BULK_PATH_FRAGMENTS
        .iter()
        .any(|fragment| path.contains(fragment));
    match clinical {
        Some(tier) if !bulk => tier.min(ceiling),
        _ => ceiling,
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
    let config = state.config();
    let limit = limit_for(
        request.uri().path(),
        // NOTE: `FerroEhrConfig::validate` refuses a tier that does not fit `usize`.
        config
            .api
            .body_limit
            .and_then(|tier| usize::try_from(tier.bytes()).ok()),
        config.server.body_limit_bytes(),
    );

    let declared = request
        .headers()
        .get(CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|text| text.parse::<usize>().ok());
    if declared.is_some_and(|length| length > limit) {
        return too_large(limit);
    }

    let (parts, body) = request.into_parts();
    match buffer(body, limit).await {
        Ok(bytes) => {
            next.run(Request::from_parts(parts, Body::from(bytes)))
                .await
        }
        Err(refusal) => *refusal,
    }
}

/// Collects `body` under `limit`: over it is the `413`, any other read failure
/// the `400` (RFC 9110 §15.5.1), never an empty or truncated body.
async fn buffer(body: Body, limit: usize) -> Result<bytes::Bytes, Box<Response>> {
    axum::body::to_bytes(body, limit).await.map_err(|error| {
        Box::new(if exceeds_limit(&error) {
            too_large(limit)
        } else {
            status_error_response(StatusCode::BAD_REQUEST, "request body cannot be read")
        })
    })
}

/// Whether a body read failed because the body is over its limit: an
/// [`http_body_util::LengthLimitError`] anywhere in the cause chain.
pub(crate) fn exceeds_limit(error: &axum::Error) -> bool {
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(error);
    while let Some(current) = cause {
        if current.is::<http_body_util::LengthLimitError>() {
            return true;
        }
        cause = current.source();
    }
    false
}

/// The `413` refusal as problem details.
///
/// The message names the limit and the configuration key that raises it, and
/// nothing else — an operator can act on it and a caller learns no internal
/// detail (the OWASP REST Security Cheat Sheet's error-hygiene control).
fn too_large(limit: usize) -> Response {
    status_error_response(
        StatusCode::PAYLOAD_TOO_LARGE,
        &format!(
            "request body exceeds the {limit}-byte limit for this route \
             (raise `api.body_limit` or `server.body_limit`)"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A body whose stream fails after `prefix` bytes.
    fn failing_body(prefix: &'static [u8]) -> Body {
        Body::from_stream(futures::stream::iter([
            Ok(bytes::Bytes::from_static(prefix)),
            Err(std::io::Error::other("connection reset")),
        ]))
    }

    #[tokio::test]
    async fn a_body_over_the_limit_is_a_413() {
        let refusal = buffer(Body::from(vec![0_u8; CEILING + 1]), CEILING)
            .await
            .expect_err("over the limit");
        assert_eq!(refusal.status(), StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn a_body_that_fails_to_read_is_a_400_not_a_413() {
        let refusal = buffer(failing_body(b"{"), CEILING)
            .await
            .expect_err("the read fails");
        assert_eq!(refusal.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_body_within_the_limit_is_kept_whole() {
        let bytes = buffer(Body::from("{}"), CEILING)
            .await
            .expect("within the limit");
        assert_eq!(bytes.as_ref(), b"{}");
    }

    const CLINICAL: usize = 16;
    const CEILING: usize = 64;

    #[test]
    fn clinical_paths_take_the_clinical_tier() {
        for path in [
            "/ferroehr/rest/openehr/v1/ehr/x/composition",
            "/ferroehr/rest/openehr/v1/query/aql",
        ] {
            assert_eq!(limit_for(path, Some(CLINICAL), CEILING), CLINICAL, "{path}");
        }
    }

    #[test]
    fn bulk_paths_take_the_ceiling() {
        for path in [
            "/ferroehr/rest/openehr/v1/definition/template/adl1.4",
            "/ferroehr/rest/openehr/v1/message/import",
            "/ferroehr/rest/openehr/v1/message/import/abc",
        ] {
            assert_eq!(
                limit_for(path, Some(CLINICAL), CEILING),
                CEILING,
                "{path} must take the ceiling"
            );
        }
    }

    /// Without a clinical tier every route takes the ceiling, and a tier above
    /// the ceiling never widens it.
    #[test]
    fn the_ceiling_covers_every_tier() {
        let clinical = "/ferroehr/rest/openehr/v1/ehr";
        assert_eq!(limit_for(clinical, None, CEILING), CEILING);
        assert_eq!(limit_for(clinical, Some(99), CEILING), CEILING);
    }
}
