// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written ITS-REST runtime: the errors a REST handler answers with.
//!
//! [`ApiError`] is the error; [`Refusal`], the error plus the headers its
//! answer carries, is what the generated server traits return;
//! [`ResponseHeaders`] is the seam the generated headers structs implement;
//! and under `rest-server` both errors render as the ITS-REST `Error` body.
//!
//! The DTOs, per-group server traits, routers and route tables are generated
//! (`emit-rest`) into [`super::generated`].

use std::fmt;

use http::{HeaderMap, HeaderName, HeaderValue, StatusCode};

/// A single semantic-validation violation, keyed by the RM path of the
/// offending node.
///
/// Carried by [`ApiError::ValidationFailed`] so the REST layer can render the
/// ITS-REST error body (`schemas/others/Error.yaml`: `{ message,
/// validationErrors[] }`).
#[derive(Debug, Clone)]
pub struct ValidationError {
    /// The RM path (archetype `aqlPath` or RM instance path) of the violation.
    pub path: String,
    /// A human-readable description of the violation.
    pub message: String,
}

/// A request body in the representation its `Content-Type` selects, for an
/// operation whose `Content-Type` parameter admits the Simplified Formats.
///
/// `C` is the canonical-JSON body; `S` is the body whose versioned content is
/// FLAT or STRUCTURED (ITS-REST overview `Resources.md` §Simplified Formats),
/// which for a CONTRIBUTION keeps the envelope canonical ("Only the inner
/// versioned payload - each `versions[i].data` … is serialized in the chosen
/// FLAT or STRUCTURED form", `operations/contribution_create.yaml`).
#[derive(Debug, Clone)]
pub enum Payload<C, S> {
    /// `application/json`, or no `Content-Type`.
    Canonical(C),
    /// `application/openehr.wt.flat+json`.
    Flat(S),
    /// `application/openehr.wt.structured+json`.
    Structured(S),
}

/// The error a REST handler may return; carries the HTTP status the openEHR
/// ITS-REST contract prescribes.
#[derive(Debug, thiserror::Error)]
pub enum ApiError {
    /// `400 Bad Request` — a malformed request the server will not process.
    #[error("bad request: {0}")]
    BadRequest(String),
    /// `401 Unauthorized` — the request carried no usable credentials.
    #[error("unauthorized: {0}")]
    Unauthorized(String),
    /// `403 Forbidden` — authenticated but not permitted this operation.
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// `404 Not Found` — the addressed resource does not exist.
    #[error("not found: {0}")]
    NotFound(String),
    /// `409 Conflict` — the resource already exists, or the request conflicts
    /// with its current state.
    #[error("conflict: {0}")]
    Conflict(String),
    /// `412 Precondition Failed` — an `If-Match` precondition did not hold.
    #[error("precondition failed: {0}")]
    PreconditionFailed(String),
    /// `422 Unprocessable Content` — a well-formed payload the server cannot
    /// process, without a per-path violation list.
    #[error("unprocessable entity: {0}")]
    Unprocessable(String),
    /// A well-formed payload that failed semantic (template/RM/terminology)
    /// validation: an ITS-REST `422 Unprocessable Entity` with a structured
    /// list of per-path violations (ITS-REST `422.yaml`).
    #[error("{} validation error(s)", .0.len())]
    ValidationFailed(Vec<ValidationError>),
    /// `415 Unsupported Media Type` — the request `Content-Type` is not served.
    #[error("unsupported media type: {0}")]
    UnsupportedMediaType(String),
    /// `406 Not Acceptable` — no representation satisfies the request `Accept`.
    #[error("not acceptable: {0}")]
    NotAcceptable(String),
    /// `501 Not Implemented` — the operation is part of the contract but this
    /// server does not provide it.
    #[error("not implemented")]
    NotImplemented,
    /// The server is temporarily unable to handle the request. Used by the
    /// application's ingress overload-shedding layer (RFC 9110 §15.6.4 —
    /// `503 Service Unavailable` is the status for a server that is
    /// temporarily overloaded). No openEHR spec governs server overload
    /// semantics — this is our own design/extension.
    #[error("service unavailable: {0}")]
    ServiceUnavailable(String),
    /// `500 Internal Server Error` — an unexpected server-side failure.
    ///
    /// The text is sent to the client as the `Error` body's `message`, so it
    /// names the failure without internal detail (no SQL, paths or payloads).
    #[error("internal error: {0}")]
    Internal(String),
}

impl ApiError {
    /// The HTTP status for this error.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        match self {
            ApiError::BadRequest(_) => StatusCode::BAD_REQUEST,
            ApiError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            ApiError::Forbidden(_) => StatusCode::FORBIDDEN,
            ApiError::NotFound(_) => StatusCode::NOT_FOUND,
            ApiError::Conflict(_) => StatusCode::CONFLICT,
            ApiError::PreconditionFailed(_) => StatusCode::PRECONDITION_FAILED,
            ApiError::Unprocessable(_) | ApiError::ValidationFailed(_) => {
                StatusCode::UNPROCESSABLE_ENTITY
            }
            ApiError::UnsupportedMediaType(_) => StatusCode::UNSUPPORTED_MEDIA_TYPE,
            ApiError::NotAcceptable(_) => StatusCode::NOT_ACCEPTABLE,
            ApiError::NotImplemented => StatusCode::NOT_IMPLEMENTED,
            ApiError::ServiceUnavailable(_) => StatusCode::SERVICE_UNAVAILABLE,
            ApiError::Internal(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

/// A header field that is not legal on the wire.
#[derive(Debug, thiserror::Error)]
pub enum HeaderError {
    /// The field name is not a legal header name.
    #[error("{name} is not a legal header name")]
    Name {
        /// The rejected name.
        name: String,
        /// What `http` reported.
        #[source]
        source: http::header::InvalidHeaderName,
    },
    /// The field value is not a legal header value.
    #[error("the value for the {name} header is not a legal header value")]
    Value {
        /// The header being set.
        name: String,
        /// What `http` reported.
        #[source]
        source: http::header::InvalidHeaderValue,
    },
}

/// The header name and value `name: value`, checked for the wire.
fn header_field(name: &str, value: &str) -> Result<(HeaderName, HeaderValue), HeaderError> {
    let header = HeaderName::from_bytes(name.as_bytes()).map_err(|source| HeaderError::Name {
        name: name.to_owned(),
        source,
    })?;
    let value = HeaderValue::from_str(value).map_err(|source| HeaderError::Value {
        name: name.to_owned(),
        source,
    })?;
    Ok((header, value))
}

/// The response headers of one documented answer, as header fields.
///
/// Every headers struct the OAS declares for an answer (`EhrCreateCreatedHeaders`,
/// `PersonUpdatePreconditionFailedHeaders`, …) implements it: a `Some` value
/// is one field, every item of a list header one field line, and `None` (or an
/// empty list) nothing.
pub trait ResponseHeaders {
    /// The header fields these values set.
    ///
    /// # Errors
    /// Returns a [`HeaderError`] when a value is not legal on the wire.
    fn into_header_map(self) -> Result<HeaderMap, HeaderError>;
}

/// Sets header `name` in `map` to `value`, when there is one.
pub(crate) fn set_header(
    map: &mut HeaderMap,
    name: &str,
    value: Option<String>,
) -> Result<(), HeaderError> {
    if let Some(value) = value {
        let (name, value) = header_field(name, &value)?;
        map.insert(name, value);
    }
    Ok(())
}

/// Appends one field line of header `name` to `map` per item of `values`.
pub(crate) fn append_headers(
    map: &mut HeaderMap,
    name: &str,
    values: Vec<String>,
) -> Result<(), HeaderError> {
    for value in values {
        let (name, value) = header_field(name, &value)?;
        map.append(name, value);
    }
    Ok(())
}

/// Merges `fields` into `headers`, each name replacing what `headers` carried
/// under it.
pub(crate) fn merge_headers(headers: &mut HeaderMap, fields: HeaderMap) {
    for name in fields.keys() {
        headers.remove(name);
    }
    let mut current: Option<HeaderName> = None;
    for (name, value) in fields {
        if let Some(name) = name {
            current = Some(name);
        }
        if let Some(name) = current.as_ref() {
            headers.append(name.clone(), value);
        }
    }
}

/// The refusal a generated server trait method answers with: an [`ApiError`]
/// and the response headers its answer carries.
///
/// Most refusals are an `ApiError` alone, and `?` converts one
/// (`From<ApiError>`). A refusal that names the resource it concerns carries
/// headers too: a `412` after a failed `If-Match` "SHOULD return also latest
/// `version_uid` in the `ETag` response headers"
/// (`ITS-REST/specifications/docs/overview/Requests_and_responses.md`
/// §If-Match and accidental overwrites), and the OAS declares `ETag` and
/// `Location` on the demographic `409` and `412` answers. Set them typed from
/// the operation's headers struct ([`Refusal::with_headers`]) or field by
/// field ([`Refusal::with_header`], [`Refusal::try_with_header`]).
#[derive(Debug)]
pub struct Refusal {
    error: ApiError,
    headers: HeaderMap,
}

impl Refusal {
    /// A refusal answering `error` with no headers.
    #[must_use]
    pub fn new(error: ApiError) -> Self {
        Self {
            error,
            headers: HeaderMap::new(),
        }
    }

    /// This refusal with one more field line of header `name`.
    #[must_use]
    pub fn with_header(mut self, name: HeaderName, value: HeaderValue) -> Self {
        self.headers.append(name, value);
        self
    }

    /// This refusal with one more field line `name: value`.
    ///
    /// # Errors
    /// Returns a [`HeaderError`] when the name or value is not legal on the
    /// wire.
    pub fn try_with_header(self, name: &str, value: &str) -> Result<Self, HeaderError> {
        let (name, value) = header_field(name, value)?;
        Ok(self.with_header(name, value))
    }

    /// This refusal with the headers the OAS declares for its answer, each
    /// replacing a same-named header set before.
    ///
    /// # Errors
    /// Returns a [`HeaderError`] when a value is not legal on the wire.
    pub fn with_headers(mut self, headers: impl ResponseHeaders) -> Result<Self, HeaderError> {
        merge_headers(&mut self.headers, headers.into_header_map()?);
        Ok(self)
    }

    /// The error the refusal answers with.
    #[must_use]
    pub fn error(&self) -> &ApiError {
        &self.error
    }

    /// The headers the answer carries.
    #[must_use]
    pub fn headers(&self) -> &HeaderMap {
        &self.headers
    }

    /// The HTTP status of the answer.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.error.status()
    }

    /// The error and the headers, taken apart.
    #[must_use]
    pub fn into_parts(self) -> (ApiError, HeaderMap) {
        (self.error, self.headers)
    }
}

impl From<ApiError> for Refusal {
    fn from(error: ApiError) -> Self {
        Self::new(error)
    }
}

/// A header an implementation answers with that is not legal on the wire is a
/// server fault: `500 Internal Server Error`, with a fixed message.
impl From<HeaderError> for Refusal {
    fn from(_error: HeaderError) -> Self {
        Self::new(ApiError::Internal(
            "the answer carries a header that is not legal on the wire".to_owned(),
        ))
    }
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.error, f)
    }
}

impl std::error::Error for Refusal {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        std::error::Error::source(&self.error)
    }
}

/// The ITS-REST `Error` body (`ITS-REST/specifications/schemas/others/Error.yaml`:
/// `{ message, validationErrors[] }`) as canonical JSON, under `status`.
#[cfg(feature = "rest-server")]
pub(crate) fn error_response(
    status: StatusCode,
    message: String,
    validation_errors: Vec<String>,
) -> axum::response::Response {
    use axum::response::IntoResponse as _;
    let body = super::generated::common::Error {
        message,
        validation_errors,
        additional_properties: std::collections::BTreeMap::new(),
    };
    match serde_json::to_vec(&body) {
        Ok(bytes) => (
            status,
            [(
                http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            )],
            bytes,
        )
            .into_response(),
        // A struct of strings always serializes; the status alone still
        // reaches the client should it ever not.
        Err(_unserializable) => status.into_response(),
    }
}

/// Renders the error as its status with the ITS-REST `Error` body.
///
/// `message` is the error's display text; `validationErrors` lists one
/// `<path>: <message>` item per violation of a
/// [`ApiError::ValidationFailed`] and is empty for every other error. The
/// schema types each item as a string without structure, so the item form is
/// our own design.
#[cfg(feature = "rest-server")]
impl axum::response::IntoResponse for ApiError {
    fn into_response(self) -> axum::response::Response {
        let validation_errors = match &self {
            ApiError::ValidationFailed(errors) => errors
                .iter()
                .map(|error| format!("{}: {}", error.path, error.message))
                .collect(),
            _ => Vec::new(),
        };
        error_response(self.status(), self.to_string(), validation_errors)
    }
}

/// Renders the refusal as its error's `Error` body plus its headers.
#[cfg(feature = "rest-server")]
impl axum::response::IntoResponse for Refusal {
    fn into_response(self) -> axum::response::Response {
        let mut response = self.error.into_response();
        merge_headers(response.headers_mut(), self.headers);
        response
    }
}
