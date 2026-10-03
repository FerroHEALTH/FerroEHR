// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Request decoding for the generated `*Params` structs: a path parameter, a
//! query pair and a header read into a typed value, a missing or unparseable
//! one refused with a `400` that names it.
//!
//! The generated axum handlers and each struct's public `from_request` run
//! this one decoding, so an intermediary that matched an operation with
//! [`super::routes::lookup`] reads its parameters exactly as the router does.
//! A query parameter is read as the generated client writes it: one
//! `name=value` pair per scalar, one pair per array item, and one pair per
//! member of a form-exploded object (`style: form, explode: true`). Both the
//! name and the value are percent-decoded as RFC 3986 §2.1 defines it, so a
//! `+` is a literal plus, not a space.

use std::fmt;
use std::str::FromStr;

use http::{HeaderMap, HeaderValue};

use super::routes::RouteMatch;
use super::runtime::{ApiError, Payload};

/// The canonical JSON media type.
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

/// The path parameter values of one matched request, percent-decoded and
/// keyed by the parameter names the template declares.
pub(crate) struct PathValues(Vec<(&'static str, String)>);

impl PathValues {
    /// The values `pairs` already carries, keyed by parameter name.
    #[cfg(feature = "rest-server")]
    pub(crate) fn new(pairs: Vec<(&'static str, String)>) -> Self {
        Self(pairs)
    }

    /// The path parameters of `matched`, each segment percent-decoded (RFC
    /// 3986 §2.1) as a router decodes a path capture.
    ///
    /// # Errors
    /// Returns [`ApiError::BadRequest`] when a segment does not decode to
    /// UTF-8 text.
    pub(crate) fn from_route(matched: &RouteMatch) -> Result<Self, ApiError> {
        matched
            .path_params
            .iter()
            .map(|param| {
                param
                    .decoded()
                    .map(|value| (param.name, value))
                    .map_err(|_not_utf8| {
                        ApiError::BadRequest(format!(
                            "the path parameter `{}` does not percent-decode to UTF-8 text",
                            param.name
                        ))
                    })
            })
            .collect::<Result<Vec<_>, ApiError>>()
            .map(Self)
    }

    /// The value of the path parameter `name`.
    ///
    /// # Errors
    /// Returns [`ApiError::BadRequest`] naming the parameter when it is absent
    /// or does not parse as a `T`.
    pub(crate) fn value<T>(&self, name: &str) -> Result<T, ApiError>
    where
        T: FromStr,
        T::Err: fmt::Display,
    {
        let raw = self
            .0
            .iter()
            .find_map(|(key, value)| (*key == name).then_some(value))
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
fn admit_json(content_type: Option<&HeaderValue>) -> Result<(), ApiError> {
    let Some(value) = content_type else {
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
    content_type: Option<&HeaderValue>,
    body: &[u8],
) -> Result<T, ApiError> {
    admit_json(content_type)?;
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

/// The Simplified Flat media type (ITS-REST overview `Resources.md`
/// §Simplified Formats).
const SIMPLIFIED_FLAT: &str = "application/openehr.wt.flat+json";

/// The Simplified Structured media type (ITS-REST overview `Resources.md`
/// §Simplified Formats).
const SIMPLIFIED_STRUCTURED: &str = "application/openehr.wt.structured+json";

/// The request body of an operation whose `Content-Type` admits the Simplified
/// Formats, in the representation the header selects; no `Content-Type` reads
/// as canonical JSON, like [`json_body`].
///
/// # Errors
/// Returns [`ApiError::UnsupportedMediaType`] under any other media type, and
/// [`ApiError::BadRequest`] for an absent body or one that is not the
/// selected representation.
pub(crate) fn payload_body<C, S>(
    content_type: Option<&HeaderValue>,
    body: &[u8],
) -> Result<Payload<C, S>, ApiError>
where
    C: serde::de::DeserializeOwned,
    S: serde::de::DeserializeOwned,
{
    let media = content_type
        .and_then(|value| value.to_str().ok())
        .map(|text| {
            text.split(';')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase()
        });
    let simplified = |body: &[u8]| -> Result<S, ApiError> {
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
    };
    match media.as_deref() {
        Some(SIMPLIFIED_FLAT) => simplified(body).map(Payload::Flat),
        Some(SIMPLIFIED_STRUCTURED) => simplified(body).map(Payload::Structured),
        _ => json_body(content_type, body).map(Payload::Canonical),
    }
}

/// The request body of an operation whose `Content-Type` admits the Simplified
/// Formats, or `None` when the request sends none.
///
/// # Errors
/// As [`payload_body`].
pub(crate) fn payload_body_optional<C, S>(
    content_type: Option<&HeaderValue>,
    body: &[u8],
) -> Result<Option<Payload<C, S>>, ApiError>
where
    C: serde::de::DeserializeOwned,
    S: serde::de::DeserializeOwned,
{
    if is_blank(body) {
        return Ok(None);
    }
    payload_body(content_type, body).map(Some)
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
