// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Reading a whole request into the [`RequestParts`] snapshot an operation
//! handler consumes, refusing what cannot be read.

use axum::extract::{FromRequestParts, RawPathParams};
use axum::response::{IntoResponse, Response};
use indexmap::IndexMap;
use openehr_its::rest::routes::{Lookup, lookup};

use crate::api::RequestParts;

/// Decompose a whole axum [`Request`](axum::extract::Request) into the
/// [`RequestParts`] snapshot an operation handler consumes.
///
/// Every native `utoipa-axum` handler takes `State` + the whole `Request` and
/// funnels through here: taking the entire request (rather than the individual
/// `RawPathParams`/`RawQuery`/`HeaderMap`/`Bytes` extractors) is what keeps the
/// `#[utoipa::path]` argument introspection happy — `bytes::Bytes` has no
/// `ToSchema`, so a bare body extractor cannot appear in a documented handler
/// signature. The upstream body-limit middleware has already buffered and
/// bounded the body.
///
/// # Errors
///
/// [`UnreadableRequest`] when a path parameter or the body cannot be read; the
/// operation must not run on what was read instead.
pub(crate) async fn into_parts(
    request: axum::extract::Request,
) -> Result<RequestParts, UnreadableRequest> {
    let (mut parts, body) = request.into_parts();
    let path: IndexMap<String, String> = RawPathParams::from_request_parts(&mut parts, &())
        .await
        .map_err(UnreadableRequest::Path)?
        .iter()
        .map(|(k, v)| (k.to_owned(), v.to_owned()))
        .collect();
    let query = parts.uri.query().map(str::to_owned);
    // The router nests this API under the base path, so `parts.uri` is already
    // group-relative, which is the path `lookup` matches.
    let route = match lookup(&parts.method, parts.uri.path()) {
        Lookup::Matched(route) => Some(route),
        Lookup::MethodNotAllowed { .. } | Lookup::NotFound => None,
    };
    let headers = parts.headers;
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .map_err(UnreadableRequest::from_body)?;
    Ok(RequestParts {
        path,
        query,
        headers,
        body,
        route,
    })
}

/// A request [`into_parts`] cannot snapshot, answered with its own status.
///
/// No openEHR spec governs reading a request: our own design. The statuses are
/// RFC 9110's, admitted by ITS-REST `Requests_and_responses.md` §HTTP status
/// codes.
#[derive(Debug, thiserror::Error)]
pub(crate) enum UnreadableRequest {
    /// A path parameter is not valid UTF-8 (`400`), or the handler runs
    /// outside a route, which records none (`500`).
    #[error("the request path parameters cannot be read")]
    Path(#[source] axum::extract::rejection::RawPathParamsRejection),
    /// The body is over its limit (`413`, RFC 9110 §15.5.14).
    #[error("the request body exceeds its limit")]
    TooLarge(#[source] axum::Error),
    /// The body failed to read (`400`, RFC 9110 §15.5.1).
    #[error("the request body cannot be read")]
    Body(#[source] axum::Error),
}

impl UnreadableRequest {
    /// Classifies a body read failure: over the limit or unreadable.
    fn from_body(error: axum::Error) -> Self {
        if crate::limits::exceeds_limit(&error) {
            Self::TooLarge(error)
        } else {
            Self::Body(error)
        }
    }
}

impl IntoResponse for UnreadableRequest {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            Self::Path(rejection) => (rejection.status(), rejection.body_text()),
            Self::TooLarge(_) => (
                http::StatusCode::PAYLOAD_TOO_LARGE,
                "request body exceeds its limit".to_owned(),
            ),
            Self::Body(_) => (
                http::StatusCode::BAD_REQUEST,
                "request body cannot be read".to_owned(),
            ),
        };
        crate::overview::error::status_error_response(status, &message)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use bytes::Bytes;

    /// The status `into_parts` answers for `body` sent to `uri` through a
    /// router that records the `{id}` path parameter.
    async fn snapshot_status(uri: &str, body: axum::body::Body) -> http::StatusCode {
        use tower::ServiceExt as _;
        let router = Router::new().route(
            "/x/{id}",
            axum::routing::any(|request: axum::extract::Request| async move {
                match into_parts(request).await {
                    Ok(_) => http::StatusCode::OK.into_response(),
                    Err(unreadable) => unreadable.into_response(),
                }
            }),
        );
        let request = http::Request::builder()
            .uri(uri)
            .body(body)
            .expect("the request builds");
        router
            .oneshot(request)
            .await
            .expect("the router answers")
            .status()
    }

    #[tokio::test]
    async fn a_readable_request_is_snapshotted() {
        let status = snapshot_status("/x/1", axum::body::Body::from("{}")).await;
        assert_eq!(status, http::StatusCode::OK);
    }

    #[tokio::test]
    async fn a_body_that_fails_to_read_is_a_400() {
        let failing = axum::body::Body::from_stream(futures::stream::iter([
            Ok(Bytes::from_static(b"{")),
            Err(std::io::Error::other("connection reset")),
        ]));
        assert_eq!(
            snapshot_status("/x/1", failing).await,
            http::StatusCode::BAD_REQUEST
        );
    }

    #[tokio::test]
    async fn a_body_over_its_limit_is_a_413() {
        let limited = axum::body::Body::new(http_body_util::Limited::new(
            axum::body::Body::from(vec![0_u8; 16]),
            4,
        ));
        assert_eq!(
            snapshot_status("/x/1", limited).await,
            http::StatusCode::PAYLOAD_TOO_LARGE
        );
    }

    #[tokio::test]
    async fn a_path_parameter_that_is_not_utf8_is_a_400() {
        let status = snapshot_status("/x/%FF", axum::body::Body::empty()).await;
        assert_eq!(status, http::StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_request_outside_a_route_is_a_500() {
        let Err(unreadable) =
            into_parts(axum::extract::Request::new(axum::body::Body::empty())).await
        else {
            panic!("a request no route matched recorded no path parameters to read");
        };
        assert_eq!(
            unreadable.into_response().status(),
            http::StatusCode::INTERNAL_SERVER_ERROR
        );
    }
}
