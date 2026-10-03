// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! The whole-API router and its fallbacks, and the runtime under the
//! generated per-group routers (`rest::generated::<group>::server::router`).
//!
//! [`router`] merges every group's router over one implementation and
//! finishes it with [`with_fallbacks`], which gives an unserved path a `404` and
//! an unserved method a `405` with `Allow`, both with the ITS-REST `Error` body.
//!
//! The generated handlers own everything the OpenAPI declares per operation
//! (which parameter sits where, its name and type, the body type, the
//! documented answers and their headers); this module owns what every handler
//! shares: reading a path capture, a query pair, a header and a body into a
//! typed value, refusing one that is missing or unparseable with a `400` that
//! names it, and writing an answer.
//!
//! A query parameter is read as the generated client writes it: one
//! `name=value` pair per scalar, one pair per array item, and one pair per
//! member of a form-exploded object (`style: form, explode: true`). Both the
//! name and the value are percent-decoded as RFC 3986 §2.1 defines it, so a
//! `+` is a literal plus, not a space.

use std::sync::Arc;

use axum::extract::RawPathParams;
use axum::extract::rejection::RawPathParamsRejection;
use axum::response::IntoResponse as _;
use http::header::{ALLOW, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, StatusCode};

use super::decode::PathValues;
use super::generated::admin::server::AdminApi;
use super::generated::definition::server::DefinitionApi;
use super::generated::demographic::server::DemographicApi;
use super::generated::ehr::server::EhrApi;
use super::generated::query::server::QueryApi;
use super::generated::system::server::SystemApi;
use super::generated::{admin, definition, demographic, ehr, query, system};
use super::routes::{Lookup, lookup};
use super::runtime::{
    ApiError, HeaderError, Refusal, ResponseHeaders, error_response, merge_headers,
};

/// The canonical JSON media type, the only one the typed router reads and
/// writes.
const CANONICAL_JSON: &str = "application/json";

/// The path captures axum extracted for a route, renamed through `names`
/// (`(capture, parameter)` pairs: the generated handler's positional captures
/// `p2`, `p3`) to the operation's parameter names.
///
/// # Errors
/// Returns [`ApiError::BadRequest`] when a capture does not decode to UTF-8
/// text.
pub(crate) fn path_captures(
    captured: Result<RawPathParams, RawPathParamsRejection>,
    names: &[(&str, &'static str)],
) -> Result<PathValues, ApiError> {
    let captured = captured.map_err(|rejection| {
        ApiError::BadRequest(format!("a path segment is not valid: {rejection}"))
    })?;
    Ok(PathValues::new(
        captured
            .iter()
            .filter_map(|(capture, value)| {
                names
                    .iter()
                    .find(|(c, _)| *c == capture)
                    .map(|(_, name)| (*name, value.to_owned()))
            })
            .collect(),
    ))
}

/// Whether the request's `Content-Type` admits a canonical-JSON body.
///
/// ITS-REST: "A client MAY use the header `Content-Type: application/json` in
/// the requests to specify the JSON payload format"
/// (`ITS-REST/specifications/docs/overview/Resources.md` §JSON Format), so an
/// absent header admits it.
///
/// # Errors
/// Returns [`ApiError::UnsupportedMediaType`] for any other media type.
fn admit_json(headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(value) = headers.get(CONTENT_TYPE) else {
        return Ok(());
    };
    let refused = || {
        ApiError::UnsupportedMediaType(format!(
            "this operation reads canonical JSON, not `{}`",
            String::from_utf8_lossy(value.as_bytes())
        ))
    };
    let text = value.to_str().map_err(|_opaque| refused())?;
    let media = text.split(';').next().unwrap_or_default().trim();
    if media.eq_ignore_ascii_case(CANONICAL_JSON) {
        Ok(())
    } else {
        Err(refused())
    }
}

/// Whether `body` carries no content at all.
fn is_blank(body: &[u8]) -> bool {
    body.iter().all(u8::is_ascii_whitespace)
}

/// The body of a request as UTF-8 text.
fn utf8(body: &[u8]) -> Result<&str, ApiError> {
    std::str::from_utf8(body)
        .map_err(|_not_utf8| ApiError::BadRequest("the request body is not UTF-8 text".to_owned()))
}

/// The canonical-JSON request body as a `T`, which the operation requires.
///
/// The body decodes through the type's own strict reader
/// ([`crate::json::from_canonical_json`]): an undeclared or repeated member is
/// a refusal, which ITS-REST classes as `400` ("syntactically invalid
/// content", `Requests_and_responses.md` §HTTP status codes).
///
/// # Errors
/// Returns [`ApiError::UnsupportedMediaType`] under another `Content-Type`,
/// and [`ApiError::BadRequest`] for an absent body or one that is not a `T`.
pub(crate) fn json_body<T: serde::de::DeserializeOwned>(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<T, ApiError> {
    admit_json(headers)?;
    if is_blank(body) {
        return Err(ApiError::BadRequest(
            "this operation requires a request body".to_owned(),
        ));
    }
    crate::json::from_canonical_json(utf8(body)?).map_err(|error| {
        ApiError::BadRequest(format!(
            "the request body is not the documented shape: {error}"
        ))
    })
}

/// The canonical-JSON request body as a `T`, or `None` when the request
/// sends none.
///
/// # Errors
/// As [`json_body`], for a body that is present.
pub(crate) fn json_body_optional<T: serde::de::DeserializeOwned>(
    headers: &HeaderMap,
    body: &[u8],
) -> Result<Option<T>, ApiError> {
    if is_blank(body) {
        return Ok(None);
    }
    json_body(headers, body).map(Some)
}

/// The request body as text (an OPT 1.4 XML upload, an ADL 2 archetype),
/// which the operation requires.
///
/// # Errors
/// Returns [`ApiError::BadRequest`] for an absent body or one that is not
/// UTF-8.
pub(crate) fn text_body(body: &[u8]) -> Result<String, ApiError> {
    if body.is_empty() {
        return Err(ApiError::BadRequest(
            "this operation requires a request body".to_owned(),
        ));
    }
    utf8(body).map(str::to_owned)
}

/// One answer under construction: a status, headers and a body.
pub(crate) struct Reply {
    status: StatusCode,
    headers: HeaderMap,
    body: Vec<u8>,
}

impl Reply {
    /// An answer with `status`, no headers and no body.
    pub(crate) fn new(status: StatusCode) -> Self {
        Self {
            status,
            headers: HeaderMap::new(),
            body: Vec::new(),
        }
    }

    /// Sets `body` serialized as canonical JSON, with `Content-Type:
    /// application/json` (`Resources.md` §JSON Format: "Proper header
    /// `Content-Type: application/json` MUST be present in the response").
    ///
    /// # Errors
    /// Returns [`ApiError::Internal`] when `body` does not serialize.
    pub(crate) fn json<B: serde::Serialize + ?Sized>(&mut self, body: &B) -> Result<(), ApiError> {
        self.body = match serde_json::to_vec(body) {
            Ok(bytes) => bytes,
            Err(_) => {
                return Err(ApiError::Internal(
                    "the answer body does not serialize".to_owned(),
                ));
            }
        };
        self.headers
            .insert(CONTENT_TYPE, HeaderValue::from_static(CANONICAL_JSON));
        Ok(())
    }

    /// Sets `body` as given; its `Content-Type` is a header of the answer.
    pub(crate) fn raw(&mut self, body: Vec<u8>) {
        self.body = body;
    }

    /// Sets the answer's declared `headers`, each replacing a same-named
    /// header set before.
    ///
    /// # Errors
    /// Returns a [`HeaderError`] when a value is not legal on the wire.
    pub(crate) fn headers(&mut self, headers: impl ResponseHeaders) -> Result<(), HeaderError> {
        merge_headers(&mut self.headers, headers.into_header_map()?);
        Ok(())
    }

    /// The finished axum response.
    pub(crate) fn finish(self) -> axum::response::Response {
        let mut response = axum::response::Response::new(axum::body::Body::from(self.body));
        *response.status_mut() = self.status;
        *response.headers_mut() = self.headers;
        response
    }
}

/// The response of one served request: the answer, or the refusal rendered as
/// the ITS-REST `Error` body with its headers.
pub(crate) fn respond(
    served: Result<axum::response::Response, Refusal>,
) -> axum::response::Response {
    match served {
        Ok(response) => response,
        Err(refusal) => refusal.into_response(),
    }
}

/// The axum router serving every operation of every API group over `api`,
/// finished with the fallbacks of [`with_fallbacks`].
///
/// It merges the six generated group routers
/// (`rest::generated::<group>::server::router`); mount it at the API base with
/// `axum::Router::nest` (`Router::new().nest("/openehr/v1", router(api))`). A
/// server that implements only some groups merges their routers itself and
/// finishes them with [`with_fallbacks`].
pub fn router<S>(api: Arc<S>) -> axum::Router
where
    S: AdminApi + DefinitionApi + DemographicApi + EhrApi + QueryApi + SystemApi,
    S: Send + Sync + 'static,
{
    with_fallbacks(
        admin::server::router(Arc::clone(&api))
            .merge(definition::server::router(Arc::clone(&api)))
            .merge(demographic::server::router(Arc::clone(&api)))
            .merge(ehr::server::router(Arc::clone(&api)))
            .merge(query::server::router(Arc::clone(&api)))
            .merge(system::server::router(api)),
    )
}

/// Finishes a router of merged group routers with the ITS-REST fallbacks.
///
/// A path no route serves answers `404 Not Found`, and a served path under a
/// method none of its routes takes answers `405 Method Not Allowed` with an
/// `Allow` header listing the methods the route tables declare for that path
/// ([`super::routes::lookup`]); both carry the ITS-REST `Error` body
/// (`ITS-REST/specifications/docs/overview/Requests_and_responses.md` §HTTP
/// status codes lists both statuses).
///
/// Call it once, after the last merge: axum applies the `405` fallback to the
/// routes registered so far, and refuses (panics on) a merge of two routers
/// that both carry a fallback, which is why the group routers carry none. The
/// fallbacks read the request path as the router sees it, so mount the
/// finished router at the API base with `axum::Router::nest`, or serve it at
/// the root of a server whose API base is `/`.
pub fn with_fallbacks(router: axum::Router) -> axum::Router {
    router
        .fallback(not_found)
        .method_not_allowed_fallback(method_not_allowed)
}

/// The `404` of a path no route serves.
async fn not_found(uri: http::Uri) -> axum::response::Response {
    error_response(
        StatusCode::NOT_FOUND,
        format!("no ITS-REST operation has the path `{}`", uri.path()),
        Vec::new(),
    )
}

/// The `405` of a served path under a method none of its routes takes.
async fn method_not_allowed(method: http::Method, uri: http::Uri) -> axum::response::Response {
    let mut response = error_response(
        StatusCode::METHOD_NOT_ALLOWED,
        format!("the operations at `{}` do not take {method}", uri.path()),
        Vec::new(),
    );
    // A path the caller routed beyond the route tables keeps the `Allow`
    // header axum computes from its own routes.
    if let Lookup::MethodNotAllowed { allowed } = lookup(&method, uri.path())
        && let Ok(value) = HeaderValue::from_str(&allowed.join(", "))
    {
        response.headers_mut().insert(ALLOW, value);
    }
    response
}
