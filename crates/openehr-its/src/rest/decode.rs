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

use http::HeaderMap;

use super::routes::RouteMatch;
use super::runtime::ApiError;

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
