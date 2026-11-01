// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Mapping the contract's [`ApiError`] onto an HTTP response with RFC 9457
//! problem details.
//!
//! The generated server traits return [`openehr_its::rest::runtime::ApiError`],
//! which carries the ITS-REST status code but renders as a bare string. The
//! REST layer wraps it in [`RestError`] so handlers can use `?` and every error
//! leaves the server as `application/problem+json`
//! ([`crate::overview::problem`]): the status ITS-REST's table requires, a
//! FerroEHR problem type for a refusal a client acts on, and the detail, with
//! the violations, the AQL position and the latest version as the additional
//! details a `Prefer: return=representation` request receives.
//!
//! NOTE: `Requests_and_responses.md` §HTTP status codes makes the error body a
//! MAY with an example shape, so the problem details body is our own design.

#![allow(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction); the carriers here are \
              cfg(test)-only, so #[expect] would be unfulfilled in the non-test build"
)]

use axum::response::{IntoResponse, Response};
use http::{HeaderValue, StatusCode, header};

use ferroehr::aql::error::AqlError;
use ferroehr::service::error::{ErrorChain, ServiceError};
use ferroehr::service::status::{QUERY_TIMEOUT_TAG, ServiceStatus, StatusError};
use ferroehr::versioning::object_version_id::VersionIdError;
use openehr_its::rest::runtime::ApiError;
use openehr_query::lexer::LexError;
use openehr_query::parser::ParseError;

use crate::overview::problem::{Detail, Kind, Position, Refused, Violation};

/// A response-rendering wrapper over the contract's [`ApiError`], with the
/// problem type and the additional details its refusal carries
/// ([`crate::overview::problem`]).
#[derive(Debug)]
pub struct RestError {
    error: ApiError,
    kind: Option<Kind>,
    details: Vec<Detail>,
}

impl RestError {
    /// The contract error the refusal answers.
    #[must_use]
    pub fn error(&self) -> &ApiError {
        &self.error
    }

    /// The status the refusal answers with.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.error.status()
    }

    /// This refusal as a problem of `kind`.
    #[must_use]
    pub fn of_kind(mut self, kind: Kind) -> Self {
        self.kind = Some(kind);
        self
    }

    /// This refusal with an additional detail, the extension member a
    /// `Prefer: return=representation` request receives.
    #[must_use]
    pub fn with_detail(mut self, detail: Detail) -> Self {
        self.details.push(detail);
        self
    }

    /// The AQL kind and the source position of an AQL refusal in `error`'s
    /// cause chain, when `status` is the `400` an AQL refusal answers.
    fn with_aql_cause(self, error: &(dyn std::error::Error + 'static)) -> Self {
        if self.status() != StatusCode::BAD_REQUEST {
            return self;
        }
        let mut aql = false;
        let mut position = None;
        let mut current = Some(error);
        while let Some(cause) = current {
            if let Some(parse) = cause.downcast_ref::<ParseError>() {
                aql = true;
                position = position.or_else(|| parse_position(parse));
            } else if let Some(lex) = cause.downcast_ref::<LexError>() {
                aql = true;
                position = position.or_else(|| Some(lex.span.clone()));
            } else if cause.downcast_ref::<AqlError>().is_some() {
                aql = true;
            }
            current = cause.source();
        }
        if !aql {
            return self;
        }
        let refused = self.of_kind(Kind::AqlInvalid);
        match position {
            Some(range) => refused.with_detail(Detail::Position(Position {
                start: range.start,
                end: range.end,
            })),
            None => refused,
        }
    }
}

/// The byte range of the source an AQL parse refusal points at.
fn parse_position(error: &ParseError) -> Option<std::ops::Range<usize>> {
    match error {
        ParseError::Lex(lex) => Some(lex.span.clone()),
        ParseError::Syntax { faults } => faults.iter().find_map(|fault| fault.bytes.clone()),
        ParseError::TooDeep { at, .. } => at.bytes.clone(),
    }
}

/// The client-visible message of a server-side fault raised inside the protocol
/// adapter.
///
/// Deliberately opaque: a codec or serializer diagnostic names Rust types, RM
/// element names and parser offsets, which the client can neither act on nor be
/// trusted with, so the detail rides a structured `tracing` record instead
/// ([`internal_fault`]). The overview fixes the body shape but not the wording;
/// the opacity is our own design.
pub(crate) const INTERNAL_MESSAGE: &str = "the server encountered an internal error";

/// Records an adapter-side fault on the trace record and returns the curated
/// opaque `500` [`ApiError`] its body carries.
///
/// `context` names the step that failed; `detail` is the raw diagnostic, which
/// is written to `tracing` and never to the wire.
pub(crate) fn internal_fault(context: &'static str, detail: &dyn std::fmt::Display) -> ApiError {
    tracing::error!(context, error = %detail, "protocol adapter: internal fault → 500");
    ApiError::Internal(INTERNAL_MESSAGE.to_owned())
}

/// Records an adapter-side fault and its whole cause chain on the trace record,
/// and returns the curated opaque `500` [`ApiError`] its body carries.
///
/// The sibling of [`internal_fault`] for a fault carrying a
/// [`std::error::Error`] source: the `cause` field is the walked chain
/// ([`ErrorChain`]), the only place the underlying diagnosis is readable.
pub(crate) fn internal_fault_caused(
    context: &'static str,
    error: &(dyn std::error::Error + 'static),
) -> ApiError {
    if let Some(cause) = error.source() {
        tracing::error!(
            context,
            error = %error,
            cause = %ErrorChain::new(cause),
            "protocol adapter: internal fault → 500"
        );
    } else {
        tracing::error!(context, error = %error, "protocol adapter: internal fault → 500");
    }
    ApiError::Internal(INTERNAL_MESSAGE.to_owned())
}

impl From<ApiError> for RestError {
    fn from(error: ApiError) -> Self {
        Self {
            error,
            kind: None,
            details: Vec::new(),
        }
    }
}

impl From<VersionIdError> for RestError {
    /// A malformed `uid_based_id` or `version_uid` wire value is a `400`: the
    /// platform decoder ([`ferroehr::versioning::object_version_id`]) classifies
    /// why it was rejected and its own [`ApiError`] mapping fixes the status, so
    /// the adapter only lifts it into the response wrapper.
    fn from(e: VersionIdError) -> Self {
        Self::from(ApiError::from(e))
    }
}

/// Maps a [`StatusError`]'s [`ServiceStatus`] onto its ITS-REST 1.1.0 status code.
///
/// The wire oracle decides each row. A free function rather than
/// `impl From<StatusError> for ApiError`, because both types are foreign to this
/// crate and the orphan rule forbids that impl.
#[must_use]
pub(crate) fn status_api_error(e: StatusError) -> ApiError {
    use ServiceStatus as S;
    let message = e.message;
    match e.status {
        // Two different refusals, one status. AuthFailure is the access
        // layer's; ProcessingRestricted is the legal mark's, our own
        // extension — no ITS-REST operation defines restriction of
        // processing, and GDPR Art. 18(2) leaves storage as the only
        // processing it admits (`docs/law/eu/gdpr/text.html`). RFC 9110
        // §15.5.4 is the HTTP authority for both; the body says which.
        S::AuthFailure | S::ProcessingRestricted => ApiError::Forbidden(message),
        S::PreconditionViolation | S::InvalidIdPattern => ApiError::BadRequest(message),
        S::ObjectVersionDoesNotExist
        | S::VersionedObjectDoesNotExist
        | S::EhrIdDoesNotExist
        | S::PartyIdDoesNotExist
        | S::CompositionDoesNotExist
        | S::ContributionDoesNotExist
        | S::ArtefactDoesNotExist
        | S::TemplateDoesNotExist
        | S::VersionDoesNotExist
        | S::SubjectIdDoesNotExist
        | S::VersionedCompositionDoesNotExist => ApiError::NotFound(message),
        S::VersionMismatch => ApiError::PreconditionFailed(message),
        S::EhrCreateFailDuplicateId
        | S::CompositionAlreadyExists
        | S::EhrForSubjectAlreadyExists
        | S::Conflict => ApiError::Conflict(message),
        S::CompositionArchetypeInvalid
        | S::InvalidArchetype
        | S::InvalidTemplate
        | S::InvalidArtefact
        | S::InvalidQuery
        | S::DefinitionUnknown
        | S::ContentInvalid => ApiError::Unprocessable(message),
        S::NotImplemented => ApiError::NotImplemented,
        // NOTE: no openEHR spec governs backend resource exhaustion — our own
        // overload contract, on RFC 9110 §15.6.4 as the HTTP authority.
        S::ServiceOverloaded => ApiError::ServiceUnavailable(message),
        // `success` is not an error; mapping it is defensively a 500.
        S::Success | S::FileNotWritable | S::Exception => ApiError::Internal(message),
    }
}

impl From<StatusError> for RestError {
    fn from(e: StatusError) -> Self {
        let restricted = e.status == ServiceStatus::ProcessingRestricted;
        let cause = e.clone();
        let refused = Self::from(status_api_error(e)).with_aql_cause(&cause);
        if restricted {
            refused.of_kind(Kind::ProcessingRestricted)
        } else {
            refused
        }
    }
}

impl From<ServiceError> for RestError {
    /// Maps a service failure straight onto the wire error, preserving the
    /// per-path violations of [`ServiceError::ValidationFailed`] that the
    /// `StatusError` bridge collapses into a flat message.
    fn from(e: ServiceError) -> Self {
        let restricted = matches!(
            &e,
            ServiceError::NotFound(err) | ServiceError::BadRequest(err) | ServiceError::Conflict(err)
                if err.status == ServiceStatus::ProcessingRestricted
        );
        let aql = match &e {
            ServiceError::BadRequest(err) => Some(err.clone()),
            _ => None,
        };
        let refused = Self::from(ApiError::from(e));
        let refused = match aql {
            Some(cause) => refused.with_aql_cause(&cause),
            None => refused,
        };
        if restricted {
            refused.of_kind(Kind::ProcessingRestricted)
        } else {
            refused
        }
    }
}

/// Renders an arbitrary status as an `about:blank` problem with `message` as
/// its detail.
///
/// Used for the method-status responses no [`ApiError`] variant can represent,
/// and for the transport-layer statuses the stack writes ([`crate::router()`]).
pub(crate) fn status_error_response(status: StatusCode, message: &str) -> Response {
    Refused::blank(status, message).into_response()
}

// NOTE: overview §HTTP Methods asks for `501` on an unrecognized method, which
// axum exposes no seam for, so it is answered `405` — itself a predefined code
// in the spec's own status table.

/// Axum fallback for a request whose method is not served by the matched
/// resource: a `405` (overview §HTTP Methods) with problem details.
///
/// The mandatory `Allow` header (RFC 9110 §15.5.6) is supplied by axum, not
/// here: only the router knows the matched path's method set, and axum decorates
/// a method-fallback response with the `Allow` it accumulated unless the
/// response already carries one
/// (<https://docs.rs/axum/0.8/axum/struct.Router.html#method.method_not_allowed_fallback>),
/// so a hand-built value would replace the accurate set with a guess. A `405`
/// from a matched handler never reaches that decoration and must carry its own
/// — see [`method_not_allowed_response`].
pub(crate) async fn method_not_allowed_handler() -> Response {
    status_error_response(
        StatusCode::METHOD_NOT_ALLOWED,
        "the request method is not allowed on this resource",
    )
}

/// Renders a `405 Method Not Allowed` from a matched handler, with the openEHR
/// error body and an explicit `Allow` header.
///
/// RFC 9110 §15.5.6 makes `Allow` mandatory on a `405`, and a handler-produced
/// one bypasses axum's allow-header machinery, which runs only on the method
/// fallback, so the caller states the set itself. `allow` is the §10.2.1
/// `Allow = #method` field value: a comma-separated method list, or the empty
/// string where the resource "has been temporarily disabled by configuration".
///
/// # Panics
/// If `allow` is not a valid header field value — impossible for the method
/// tokens and the empty string this takes, all of which are compile-time
/// literals at the call sites.
pub(crate) fn method_not_allowed_response(allow: &'static str, message: &str) -> Response {
    let mut resp = status_error_response(StatusCode::METHOD_NOT_ALLOWED, message);
    resp.headers_mut()
        .insert(header::ALLOW, HeaderValue::from_static(allow));
    resp
}

/// What a validation refusal tells a request that did not prefer the
/// representation, which therefore was not sent the violations.
const VALIDATION_HINT: &str =
    "a request with `Prefer: return=representation` receives each path and its violation";

/// The detail of `error`: its message without the status word the
/// [`ApiError`] display leads with, which the problem's title already says.
fn detail_of(error: &ApiError) -> String {
    match error {
        ApiError::BadRequest(message)
        | ApiError::Unauthorized(message)
        | ApiError::Forbidden(message)
        | ApiError::NotFound(message)
        | ApiError::Conflict(message)
        | ApiError::PreconditionFailed(message)
        | ApiError::Unprocessable(message)
        | ApiError::UnsupportedMediaType(message)
        | ApiError::NotAcceptable(message)
        | ApiError::ServiceUnavailable(message)
        | ApiError::Internal(message) => message.clone(),
        ApiError::ValidationFailed(errors) => {
            format!("the content fails validation at {} path(s)", errors.len())
        }
        ApiError::NotImplemented => "this server does not provide the operation".to_owned(),
    }
}

/// The problem type of `error` when the refusal names none: the ones a
/// client acts on have a FerroEHR type, the rest are `about:blank`.
fn kind_of(error: &ApiError) -> Option<Kind> {
    match error {
        ApiError::Unauthorized(_) => Some(Kind::Unauthenticated),
        ApiError::Forbidden(_) => Some(Kind::Forbidden),
        ApiError::Conflict(_) => Some(Kind::Conflict),
        ApiError::PreconditionFailed(_) => Some(Kind::VersionConflict),
        ApiError::Unprocessable(_) => Some(Kind::ContentInvalid),
        ApiError::ValidationFailed(_) => Some(Kind::ValidationFailed),
        ApiError::ServiceUnavailable(_) => Some(Kind::Unavailable),
        ApiError::BadRequest(_)
        | ApiError::NotFound(_)
        | ApiError::UnsupportedMediaType(_)
        | ApiError::NotAcceptable(_)
        | ApiError::NotImplemented
        | ApiError::Internal(_) => None,
    }
}

impl IntoResponse for RestError {
    fn into_response(self) -> Response {
        // A query-execution timeout arrives as an `exception` `StatusError` whose
        // message carries [`QUERY_TIMEOUT_TAG`], and is rendered as the `408` of
        // `Requests_and_responses.md` §HTTP status codes with the sentinel
        // stripped.
        if let ApiError::Internal(raw) = &self.error
            && let Some(detail) = raw.strip_prefix(QUERY_TIMEOUT_TAG)
        {
            return Refused::of(Kind::QueryTimeout, detail).into_response();
        }
        let status = self.error.status();
        let detail = detail_of(&self.error);
        let mut refused = match self.kind.or_else(|| kind_of(&self.error)) {
            Some(kind) if kind.status() == status => Refused::of(kind, detail),
            _ => Refused::blank(status, detail),
        };
        if let ApiError::ValidationFailed(errors) = &self.error {
            refused = refused
                .with_hint(VALIDATION_HINT)
                .with_detail(Detail::Errors(
                    errors
                        .iter()
                        .map(|e| Violation {
                            path: e.path.clone(),
                            detail: e.message.clone(),
                        })
                        .collect(),
                ));
        }
        for detail in self.details {
            refused = refused.with_detail(detail);
        }
        let mut resp = refused.into_response();
        // A `503` condition is transient by definition (RFC 9110 §15.6.4), so
        // every one carries a `Retry-After` hint.
        if status == StatusCode::SERVICE_UNAVAILABLE {
            resp.headers_mut()
                .insert(header::RETRY_AFTER, HeaderValue::from_static("1"));
        }
        resp
    }
}

#[cfg(test)]
mod tests {
    use axum::response::IntoResponse;
    use http::StatusCode;
    use http_body_util::BodyExt;
    use openehr_its::rest::runtime::{ApiError, ValidationError};
    use serde_json::Value;

    use super::RestError;
    use crate::overview::problem::{BASE, Refused};

    async fn body_json(err: ApiError) -> (StatusCode, Value) {
        let resp = RestError::from(err).into_response();
        let status = resp.status();
        assert_eq!(
            resp.headers().get(http::header::CONTENT_TYPE).unwrap(),
            "application/problem+json"
        );
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    /// The problem a `Prefer: return=representation` request receives.
    fn full_problem(err: ApiError) -> Value {
        let resp = RestError::from(err).into_response();
        resp.extensions()
            .get::<Refused>()
            .expect("the refusal is marked for the middleware")
            .render(None, true)
            .to_json()
    }

    #[tokio::test]
    async fn validation_failure_renders_422_with_its_violations_on_request() {
        let violations = || {
            ApiError::ValidationFailed(vec![
                ValidationError {
                    path: "/content[0]/data".to_owned(),
                    message: "value out of range".to_owned(),
                },
                ValidationError {
                    path: "/category".to_owned(),
                    message: "code not in group".to_owned(),
                },
            ])
        };
        let (status, body) = body_json(violations()).await;
        assert_eq!(status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(body["type"], format!("{BASE}validation-failed"));
        assert_eq!(body["status"], 422);
        assert_eq!(
            body["detail"],
            "the content fails validation at 2 path(s); a request with \
             `Prefer: return=representation` receives each path and its violation",
            "a request without the preference is told how to receive the violations"
        );
        assert!(
            body.get("errors").is_none(),
            "the additional details wait for Prefer"
        );
        let full = full_problem(violations());
        assert_eq!(
            full["detail"], "the content fails validation at 2 path(s)",
            "a request that sent the preference is not told to send it"
        );
        let errors = full["errors"].as_array().expect("errors");
        assert_eq!(errors.len(), 2);
        assert_eq!(
            errors[0],
            serde_json::json!({"path": "/content[0]/data", "detail": "value out of range"})
        );
        assert_eq!(
            errors[1],
            serde_json::json!({"path": "/category", "detail": "code not in group"})
        );
    }

    #[tokio::test]
    async fn other_errors_carry_the_five_members_and_no_status_word_in_the_detail() {
        let (status, body) = body_json(ApiError::NotFound("EHR x".to_owned())).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(body["type"], "about:blank");
        assert_eq!(body["title"], "Not Found");
        assert_eq!(body["status"], 404);
        assert_eq!(body["detail"], "EHR x");
        assert!(body.get("errors").is_none());
    }

    async fn handler_body(resp: axum::response::Response) -> (StatusCode, Value) {
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn method_not_allowed_renders_405_problem() {
        let resp = super::method_not_allowed_handler().await;
        // The router fallback deliberately leaves `Allow` unset: axum fills it
        // in from the matched route's method set, and would NOT overwrite a
        // value the handler had already written (see the handler doc). The
        // header's presence on the wire is pinned end-to-end in
        // `tests/http.rs`.
        assert!(
            !resp.headers().contains_key(http::header::ALLOW),
            "the fallback must leave Allow to axum's per-route set"
        );
        let (status, body) = handler_body(resp).await;
        assert_eq!(status, StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(body["title"], "Method Not Allowed");
        assert!(body.get("detail").and_then(Value::as_str).is_some());
    }

    #[tokio::test]
    async fn handler_produced_405_carries_an_allow_header() {
        // RFC 9110 §15.5.6: "The origin server MUST generate an Allow header
        // field in a 405 response containing a list of the target resource's
        // currently supported methods." A handler-produced 405 gets no axum
        // decoration, so the helper states the set itself.
        let resp = super::method_not_allowed_response("GET,HEAD", "nope");
        assert_eq!(resp.status(), StatusCode::METHOD_NOT_ALLOWED);
        assert_eq!(
            resp.headers()
                .get(http::header::ALLOW)
                .and_then(|v| v.to_str().ok()),
            Some("GET,HEAD")
        );
        let (_status, body) = handler_body(resp).await;
        assert_eq!(body["title"], "Method Not Allowed");
        assert_eq!(body["detail"], "nope");
    }

    #[tokio::test]
    async fn handler_produced_405_can_advertise_the_empty_method_set() {
        // RFC 9110 §10.2.1: "An empty Allow field value indicates that the
        // resource allows no methods, which might occur in a 405 response if
        // the resource has been temporarily disabled by configuration" — the
        // config-gated admin group's case. The header is PRESENT and empty,
        // never absent.
        let resp = super::method_not_allowed_response("", "disabled");
        assert_eq!(
            resp.headers()
                .get(http::header::ALLOW)
                .and_then(|v| v.to_str().ok()),
            Some("")
        );
    }

    #[tokio::test]
    async fn query_timeout_tag_renders_408_with_clean_detail() {
        // Requests_and_responses.md §HTTP status codes, row 408: a query-execution
        // timeout is tagged on an `exception` message and rendered as 408 with the
        // sentinel stripped from the client-visible detail.
        let tagged = format!("{}query aborted after 5000ms", super::QUERY_TIMEOUT_TAG);
        let (status, body) = body_json(ApiError::Internal(tagged)).await;
        assert_eq!(status, StatusCode::REQUEST_TIMEOUT);
        assert_eq!(body["type"], format!("{BASE}query-timeout"));
        assert_eq!(body["detail"], "query aborted after 5000ms");
    }

    #[test]
    fn an_aql_parse_refusal_is_aql_invalid_with_its_position() {
        let err = openehr_query::parser::parse_str("SELECT e FROM EHR e WHERE").unwrap_err();
        let err =
            ferroehr::service::status::StatusError::precondition("invalid AQL").with_source(err);
        let full = full_problem_of(RestError::from(err));
        assert_eq!(full["type"], format!("{BASE}aql-invalid"));
        assert_eq!(full["status"], 400);
        assert!(full["position"]["start"].is_u64(), "{full}");
        assert!(full["position"]["end"].is_u64(), "{full}");
    }

    #[test]
    fn a_processing_restriction_is_its_own_type() {
        use ferroehr::service::status::{ServiceStatus, StatusError};
        let err = StatusError::new(ServiceStatus::ProcessingRestricted, "restricted");
        let full = full_problem_of(RestError::from(err));
        assert_eq!(full["type"], format!("{BASE}processing-restricted"));
        assert_eq!(full["status"], 403);
    }

    fn full_problem_of(err: RestError) -> Value {
        err.into_response()
            .extensions()
            .get::<Refused>()
            .expect("the refusal is marked")
            .render(None, true)
            .to_json()
    }

    #[tokio::test]
    async fn untagged_internal_stays_500() {
        // A genuine server fault (no timeout tag) still maps to 500.
        let (status, _body) = body_json(ApiError::Internal("boom".to_owned())).await;
        assert_eq!(status, StatusCode::INTERNAL_SERVER_ERROR);
    }

    #[tokio::test]
    async fn service_overloaded_maps_to_503_with_retry_after() {
        // A storage-classified pool exhaustion (`ServiceOverloaded`)
        // surfaces as 503 + Retry-After (our overload contract; RFC 9110
        // §15.6.4).
        use ferroehr::service::status::{ServiceStatus, StatusError};
        let err = StatusError::new(ServiceStatus::ServiceOverloaded, "overloaded");
        let resp = RestError::from(err).into_response();
        assert_eq!(resp.status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(
            resp.headers()
                .get(http::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some("1")
        );
    }

    #[tokio::test]
    async fn storage_conflict_maps_to_409() {
        // A storage-classified integrity/serialization conflict
        // (`Conflict`) surfaces as 409 (ITS-REST overview §HTTP status codes).
        use ferroehr::service::status::{ServiceStatus, StatusError};
        let err = StatusError::new(ServiceStatus::Conflict, "duplicate key");
        let resp = RestError::from(err).into_response();
        assert_eq!(resp.status(), StatusCode::CONFLICT);
    }
}
