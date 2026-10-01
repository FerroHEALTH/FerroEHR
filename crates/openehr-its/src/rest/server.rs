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

use std::fmt;
use std::str::FromStr;
use std::sync::Arc;

use axum::extract::RawPathParams;
use axum::extract::rejection::RawPathParamsRejection;
use axum::response::IntoResponse as _;
use http::header::{ALLOW, CONTENT_TYPE};
use http::{HeaderMap, HeaderValue, StatusCode};

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

/// Parses `raw`, the text of parameter `name` at `location`, as a `T`.
fn parse<T>(location: &str, name: &str, raw: &str) -> Result<T, ApiError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    raw.parse::<T>().map_err(|error| {
        ApiError::BadRequest(format!(
            "the {location} parameter `{name}` is not a valid value: {error}"
        ))
    })
}

/// The refusal for a required parameter the request does not carry.
fn missing(location: &str, name: &str) -> ApiError {
    ApiError::BadRequest(format!(
        "the required {location} parameter `{name}` is missing"
    ))
}

/// The path captures of one matched route, percent-decoded by axum.
pub(crate) struct PathCaptures(RawPathParams);

impl PathCaptures {
    /// The captures axum extracted for the route.
    ///
    /// # Errors
    /// Returns [`ApiError::BadRequest`] when a capture does not decode to
    /// UTF-8 text.
    pub(crate) fn new(
        captured: Result<RawPathParams, RawPathParamsRejection>,
    ) -> Result<Self, ApiError> {
        captured.map(Self).map_err(|rejection| {
            ApiError::BadRequest(format!("a path segment is not valid: {rejection}"))
        })
    }

    /// The value of the path parameter `name`, captured as `capture`.
    ///
    /// # Errors
    /// Returns [`ApiError::BadRequest`] naming the parameter when it is absent
    /// or does not parse as a `T`.
    pub(crate) fn value<T>(&self, capture: &str, name: &str) -> Result<T, ApiError>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        let raw = self
            .0
            .iter()
            .find_map(|(key, value)| (key == capture).then_some(value))
            .ok_or_else(|| missing("path", name))?;
        parse("path", name, raw)
    }
}

/// The query pairs of one request, in order, each name and value decoded.
pub(crate) struct QueryPairs(Vec<(String, String)>);

impl QueryPairs {
    /// The pairs of `query`, the request's query string without its `?`.
    ///
    /// # Errors
    /// Returns [`ApiError::BadRequest`] when a name or value does not decode
    /// to UTF-8 text.
    pub(crate) fn parse(query: Option<&str>) -> Result<Self, ApiError> {
        let mut pairs = Vec::new();
        for pair in query.unwrap_or_default().split('&') {
            if pair.is_empty() {
                continue;
            }
            let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
            let decode = |text: &str| {
                urlencoding::decode(text)
                    .map(std::borrow::Cow::into_owned)
                    .map_err(|_not_utf8| {
                        ApiError::BadRequest(
                            "the query string does not percent-decode to UTF-8 text".to_owned(),
                        )
                    })
            };
            pairs.push((decode(name)?, decode(value)?));
        }
        Ok(Self(pairs))
    }

    /// Every value given for `name`, in order.
    fn values<'a>(&'a self, name: &'a str) -> impl Iterator<Item = &'a str> + 'a {
        self.0
            .iter()
            .filter(move |(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }

    /// The scalar query parameter `name`, when the request gives it.
    ///
    /// # Errors
    /// Returns [`ApiError::BadRequest`] naming the parameter when it is given
    /// more than once (a scalar `form` parameter is one pair) or does not parse
    /// as a `T`.
    pub(crate) fn optional<T>(&self, name: &str) -> Result<Option<T>, ApiError>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        let mut values = self.values(name);
        let Some(first) = values.next() else {
            return Ok(None);
        };
        if values.next().is_some() {
            return Err(ApiError::BadRequest(format!(
                "the query parameter `{name}` is given more than once"
            )));
        }
        parse("query", name, first).map(Some)
    }

    /// The scalar query parameter `name`, which the operation requires.
    ///
    /// # Errors
    /// Returns [`ApiError::BadRequest`] naming the parameter when it is absent,
    /// repeated, or does not parse as a `T`.
    pub(crate) fn required<T>(&self, name: &str) -> Result<T, ApiError>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        self.optional(name)?.ok_or_else(|| missing("query", name))
    }

    /// The members of the form-exploded object parameter `name`: every pair
    /// whose name is not one of the operation's `declared` query parameters,
    /// or `None` when there is none.
    ///
    /// A value is read the way the generated client writes it: a value that is
    /// JSON text other than a JSON string (a number, a boolean, an object, an
    /// array) is that value, and anything else is the text itself.
    ///
    /// # Errors
    /// Returns [`ApiError::BadRequest`] naming the parameter when a member is
    /// given more than once.
    pub(crate) fn members<M, V>(&self, name: &str, declared: &[&str]) -> Result<Option<M>, ApiError>
    where
        M: FromIterator<(String, V)>,
        V: serde::de::DeserializeOwned + From<String>,
    {
        let mut seen: Vec<&str> = Vec::new();
        let mut members: Vec<(String, V)> = Vec::new();
        for (key, value) in &self.0 {
            if declared.contains(&key.as_str()) {
                continue;
            }
            if seen.contains(&key.as_str()) {
                return Err(ApiError::BadRequest(format!(
                    "the member `{key}` of the query parameter `{name}` is given more than once"
                )));
            }
            seen.push(key);
            members.push((key.clone(), member_value(value)));
        }
        Ok((!members.is_empty()).then(|| members.into_iter().collect()))
    }
}

/// One form-exploded object member's value, read as the client writes it.
fn member_value<V>(raw: &str) -> V
where
    V: serde::de::DeserializeOwned + From<String>,
{
    if raw.starts_with('"') {
        return V::from(raw.to_owned());
    }
    // NOTE: no openEHR spec governs a form-exploded member's text, our own design;
    // text that is not JSON is legitimately the member's plain string value.
    serde_json::from_str::<V>(raw).unwrap_or_else(|_not_json| V::from(raw.to_owned()))
}

/// The text of header `name`, its field lines joined with `, ` as RFC 9110
/// §5.3 lets a recipient combine them, or `None` when absent.
fn header_text(headers: &HeaderMap, name: &str) -> Result<Option<String>, ApiError> {
    let mut joined: Option<String> = None;
    for line in header_lines(headers, name)? {
        match joined.as_mut() {
            Some(text) => {
                text.push_str(", ");
                text.push_str(&line);
            }
            None => joined = Some(line),
        }
    }
    Ok(joined)
}

/// Every field line of header `name`, in order.
fn header_lines(headers: &HeaderMap, name: &str) -> Result<Vec<String>, ApiError> {
    headers
        .get_all(name)
        .iter()
        .map(|value| {
            value.to_str().map(str::to_owned).map_err(|_opaque| {
                ApiError::BadRequest(format!("the header `{name}` is not visible ASCII text"))
            })
        })
        .collect()
}

/// The header parameter `name`, when the request sends it.
///
/// # Errors
/// Returns [`ApiError::BadRequest`] naming the header when it is not text or
/// does not parse as a `T`.
pub(crate) fn header_optional<T>(headers: &HeaderMap, name: &str) -> Result<Option<T>, ApiError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    header_text(headers, name)?
        .map(|text| parse("header", name, &text))
        .transpose()
}

/// The header parameter `name`, which the operation requires.
///
/// # Errors
/// Returns [`ApiError::BadRequest`] naming the header when it is absent, not
/// text, or does not parse as a `T`.
pub(crate) fn header_required<T>(headers: &HeaderMap, name: &str) -> Result<T, ApiError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    header_optional(headers, name)?.ok_or_else(|| missing("header", name))
}

/// The list header parameter `name` (`style: simple, explode: true`), one
/// item per field line as the client sends it, or `None` when absent.
///
/// # Errors
/// Returns [`ApiError::BadRequest`] naming the header when a field line is not
/// text or does not parse as a `T`.
pub(crate) fn header_list<T>(headers: &HeaderMap, name: &str) -> Result<Option<Vec<T>>, ApiError>
where
    T: FromStr,
    T::Err: fmt::Display,
{
    let lines = header_lines(headers, name)?;
    if lines.is_empty() {
        return Ok(None);
    }
    lines
        .iter()
        .map(|line| parse("header", name, line))
        .collect::<Result<Vec<T>, ApiError>>()
        .map(Some)
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
        self.body = serde_json::to_vec(body).map_err(|error| {
            ApiError::Internal(format!("the answer body does not serialize: {error}"))
        })?;
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
