// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FerroEHR's problem types and the RFC 9457 problem details every error on
//! the openEHR and extension routes answers with.
//!
//! ITS-REST makes the status codes normative and the error body a MAY with an
//! example shape (`Requests_and_responses.md` §HTTP status codes), so the body
//! is RFC 9457 problem details (<https://www.rfc-editor.org/rfc/rfc9457>),
//! written through `ferrohealth_server::problem`. Each [`Kind`] is defined
//! once, under the book page that documents it ([`BASE`], RFC 9457 §4); a
//! refusal with no kind is `about:blank` (§4.2.1).
//!
//! The extension members are the additional error details ITS-REST lets a
//! service return when the request carries `Prefer: return=representation`
//! (`Requests_and_responses.md` §HTTP status codes); without it the problem
//! carries the five members of RFC 9457 §3.1 alone. [`finish`] chooses, and
//! sets the server's request id as the `instance`. A route under the FHIR base
//! answers an `OperationOutcome` instead, as FHIR R4 `http.html` prescribes.

use std::sync::LazyLock;

use axum::extract::{Request, State};
use axum::middleware::Next;
use axum::response::{IntoResponse as _, Response};
use ferrohealth_server::problem::{Problem, ProblemError, ProblemType, ProblemTypes};
use ferrohealth_telemetry::request_log::RequestId;
use http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header};

/// The base URI every FerroEHR problem type is under: the book page that
/// documents each type at the anchor of its slug.
pub const BASE: &str = "https://ferroehr.eu/docs/latest/using-the-api/problems.html#";

/// The `Preference-Applied` header (RFC 7240 §3).
const PREFERENCE_APPLIED: HeaderName = HeaderName::from_static("preference-applied");

/// The preference that asks for the additional error details.
const REPRESENTATION: &str = "return=representation";

/// FerroEHR's problem types, one per refusal a client can act on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An AQL text that does not parse, or that the query engine refuses.
    AqlInvalid,
    /// A query that ran past the server's execution budget.
    QueryTimeout,
    /// A payload that fails template, RM or terminology validation.
    ValidationFailed,
    /// A payload that is well-formed but cannot be processed.
    ContentInvalid,
    /// A request that conflicts with the resource's current state.
    Conflict,
    /// An `If-Match` that names a version other than the latest.
    VersionConflict,
    /// A request without usable credentials.
    Unauthenticated,
    /// An authenticated caller the access policy refuses.
    Forbidden,
    /// A record whose processing is restricted (GDPR Art. 18).
    ProcessingRestricted,
    /// A server that is temporarily unable to answer.
    Unavailable,
    /// A caller past its request rate.
    RateLimited,
}

impl Kind {
    /// Every kind, in the order the book lists them.
    pub const ALL: [Self; 11] = [
        Self::AqlInvalid,
        Self::QueryTimeout,
        Self::ValidationFailed,
        Self::ContentInvalid,
        Self::Conflict,
        Self::VersionConflict,
        Self::Unauthenticated,
        Self::Forbidden,
        Self::ProcessingRestricted,
        Self::Unavailable,
        Self::RateLimited,
    ];

    /// The slug the type URI ends in, the anchor on the book page.
    #[must_use]
    pub const fn slug(self) -> &'static str {
        match self {
            Self::AqlInvalid => "aql-invalid",
            Self::QueryTimeout => "query-timeout",
            Self::ValidationFailed => "validation-failed",
            Self::ContentInvalid => "content-invalid",
            Self::Conflict => "conflict",
            Self::VersionConflict => "version-conflict",
            Self::Unauthenticated => "unauthenticated",
            Self::Forbidden => "forbidden",
            Self::ProcessingRestricted => "processing-restricted",
            Self::Unavailable => "unavailable",
            Self::RateLimited => "rate-limited",
        }
    }

    /// The type's title, the same for every occurrence (RFC 9457 §3.1.3).
    #[must_use]
    pub const fn title(self) -> &'static str {
        match self {
            Self::AqlInvalid => "The AQL query is not valid",
            Self::QueryTimeout => "The query ran longer than the server allows",
            Self::ValidationFailed => "The content fails validation",
            Self::ContentInvalid => "The content cannot be processed",
            Self::Conflict => "The request conflicts with the resource's current state",
            Self::VersionConflict => "The version named is not the latest",
            Self::Unauthenticated => "The request needs credentials",
            Self::Forbidden => "The caller may not perform this operation",
            Self::ProcessingRestricted => "Processing of this record is restricted",
            Self::Unavailable => "The server is temporarily unavailable",
            Self::RateLimited => "The caller sent more requests than its rate allows",
        }
    }

    /// The status the type answers with.
    #[must_use]
    pub const fn status(self) -> StatusCode {
        match self {
            Self::AqlInvalid => StatusCode::BAD_REQUEST,
            Self::QueryTimeout => StatusCode::REQUEST_TIMEOUT,
            Self::ValidationFailed | Self::ContentInvalid => StatusCode::UNPROCESSABLE_ENTITY,
            Self::Conflict => StatusCode::CONFLICT,
            Self::VersionConflict => StatusCode::PRECONDITION_FAILED,
            Self::Unauthenticated => StatusCode::UNAUTHORIZED,
            Self::Forbidden | Self::ProcessingRestricted => StatusCode::FORBIDDEN,
            Self::Unavailable => StatusCode::SERVICE_UNAVAILABLE,
            Self::RateLimited => StatusCode::TOO_MANY_REQUESTS,
        }
    }

    /// The registered type, or `about:blank` of the kind's status should the
    /// registry not have been built, which the tests rule out.
    fn problem_type(self) -> ProblemType {
        match &*TYPES {
            Ok(types) => types
                .get(self.slug())
                .cloned()
                .unwrap_or_else(|| ProblemType::about_blank(self.status())),
            Err(error) => {
                tracing::error!(%error, "the problem type registry did not build");
                ProblemType::about_blank(self.status())
            }
        }
    }
}

/// The registry of every [`Kind`], built once.
static TYPES: LazyLock<Result<ProblemTypes, ProblemError>> = LazyLock::new(|| {
    let mut types = ProblemTypes::new(BASE)?;
    for kind in Kind::ALL {
        types.define(kind.slug(), kind.title(), kind.status())?;
    }
    Ok(types)
});

/// FerroEHR's problem types, for the served `OpenAPI` document and the book.
///
/// # Errors
///
/// The [`ProblemError`] a definition met, which the tests rule out.
pub fn registry() -> Result<&'static ProblemTypes, &'static ProblemError> {
    TYPES.as_ref()
}

/// The problem details document every FerroEHR error answers with, as the
/// served `OpenAPI` document declares it (RFC 9457 §3.1, §3.2).
#[derive(Debug, serde::Serialize, utoipa::ToSchema)]
#[schema(as = Problem)]
pub struct ProblemDocument {
    /// The problem type: a FerroEHR type under [`BASE`], or `about:blank`.
    #[serde(rename = "type")]
    #[schema(rename = "type", example = "about:blank")]
    pub kind: String,
    /// The type's summary; for `about:blank` the status phrase.
    pub title: Option<String>,
    /// The HTTP status of this occurrence.
    pub status: u16,
    /// What helps the client correct this occurrence.
    pub detail: Option<String>,
    /// The server's request id, as a `urn:uuid:` URI.
    pub instance: Option<String>,
    /// `validation-failed`, with `Prefer: return=representation`: every
    /// violation.
    pub errors: Option<Vec<Violation>>,
    /// `aql-invalid`, with `Prefer: return=representation`: where in the AQL
    /// text the refusal points.
    pub position: Option<Position>,
    /// `version-conflict`, with `Prefer: return=representation`: the latest
    /// version of the resource.
    pub current_version: Option<String>,
}

/// The source byte range an AQL refusal points at.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct Position {
    /// The first byte of the range.
    pub start: usize,
    /// The byte after the range.
    pub end: usize,
}

/// One validation violation: the RM path and what failed there.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, utoipa::ToSchema)]
pub struct Violation {
    /// The RM path of the offending node.
    pub path: String,
    /// What the node violates.
    pub detail: String,
}

/// An additional error detail, one extension member of a problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detail {
    /// `position`: where in the AQL text the refusal points.
    Position(Position),
    /// `errors`: every violation of a validation refusal.
    Errors(Vec<Violation>),
    /// `current_version`: the latest version of the resource a `412` names.
    CurrentVersion(String),
}

impl Detail {
    /// The extension member's name.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Position(_) => "position",
            Self::Errors(_) => "errors",
            Self::CurrentVersion(_) => "current_version",
        }
    }

    /// `problem` with this member.
    fn extend(&self, problem: Problem) -> Result<Problem, ProblemError> {
        match self {
            Self::Position(position) => problem.with_extension(self.name(), position),
            Self::Errors(errors) => problem.with_extension(self.name(), errors),
            Self::CurrentVersion(version) => problem.with_extension(self.name(), version),
        }
    }
}

/// One refusal as problem details: the members every request receives, and
/// the extension members a `Prefer: return=representation` request receives
/// beside them.
#[derive(Debug, Clone)]
pub struct Refused {
    problem: Problem,
    details: Vec<Detail>,
}

impl Refused {
    /// A refusal of `kind` with `detail`.
    #[must_use]
    pub fn of(kind: Kind, detail: impl Into<String>) -> Self {
        Self {
            problem: Problem::of(&kind.problem_type()).with_detail(detail),
            details: Vec::new(),
        }
    }

    /// An `about:blank` refusal of `status` with `detail`.
    #[must_use]
    pub fn blank(status: StatusCode, detail: impl Into<String>) -> Self {
        Self {
            problem: Problem::new(status).with_detail(detail),
            details: Vec::new(),
        }
    }

    /// This refusal with an additional detail, the extension member a
    /// `Prefer: return=representation` request receives.
    #[must_use]
    pub fn with_detail(mut self, detail: Detail) -> Self {
        self.details.retain(|kept| kept.name() != detail.name());
        self.details.push(detail);
        self
    }

    /// The status the refusal answers with.
    #[must_use]
    pub fn status(&self) -> StatusCode {
        self.problem.status()
    }

    /// The problem as `instance` and the preference choose it.
    pub(crate) fn render(&self, request_id: Option<&str>, representation: bool) -> Problem {
        let mut problem = self.problem.clone();
        if let Some(id) = request_id {
            problem = problem.with_request_id(id);
        }
        if representation {
            for detail in &self.details {
                problem = match detail.extend(problem.clone()) {
                    Ok(extended) => extended,
                    Err(error) => {
                        tracing::error!(%error, member = detail.name(), "a problem detail was dropped");
                        problem
                    }
                };
            }
        }
        problem
    }

    /// The response: the problem without its additional details, marked so
    /// [`finish`] can write it for the request.
    #[must_use]
    pub fn into_response(self) -> Response {
        let mut response = self.render(None, false).into_response();
        response.extensions_mut().insert(self);
        response
    }
}

/// Whether `headers` carry the `return=representation` preference (RFC 7240
/// §4.2): one `Prefer` field or several, each a comma-separated list whose
/// items may carry parameters after `;`.
#[must_use]
pub fn prefers_representation(headers: &HeaderMap) -> bool {
    headers
        .get_all(HeaderName::from_static("prefer"))
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|value| value.split(','))
        .filter_map(|item| item.split(';').next())
        .any(|item| {
            item.split('=')
                .map(str::trim)
                .collect::<Vec<_>>()
                .join("=")
                .eq_ignore_ascii_case(REPRESENTATION)
        })
}

/// The version an `ETag` names, without the weak marker and the quotes.
fn version_of(etag: &HeaderValue) -> Option<String> {
    let text = etag.to_str().ok()?.trim();
    let text = text.strip_prefix("W/").unwrap_or(text);
    let text = text
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .unwrap_or(text);
    (!text.is_empty()).then(|| text.to_owned())
}

/// The FHIR base under the REST root, whose routes answer an
/// `OperationOutcome`; `None` when the FHIR routes are not built.
#[derive(Debug, Clone, Default)]
pub struct FhirBase(Option<std::sync::Arc<str>>);

impl FhirBase {
    /// The FHIR base `{rest_root}/fhir/r4`, when the FHIR routes are built.
    #[must_use]
    pub fn of(rest_root: &str) -> Self {
        if cfg!(feature = "fhir") {
            Self(Some(std::sync::Arc::from(format!(
                "{rest_root}{}",
                crate::api::FHIR_PREFIX
            ))))
        } else {
            Self(None)
        }
    }

    /// Whether `path` is the FHIR base or under it.
    fn covers(&self, path: &str) -> bool {
        self.0.as_deref().is_some_and(|base| {
            path.strip_prefix(base)
                .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
        })
    }
}

/// The refusal on a FHIR route: an `OperationOutcome` (FHIR R4 `http.html`).
#[cfg(feature = "fhir")]
fn outcome(status: StatusCode, detail: &str) -> Response {
    crate::extensions::fhir::refusal_outcome(status, detail)
}

/// Without the FHIR routes no path is under the FHIR base.
#[cfg(not(feature = "fhir"))]
fn outcome(status: StatusCode, detail: &str) -> Response {
    Refused::blank(status, detail)
        .render(None, false)
        .into_response()
}

/// A refusal or panic the family stack writes, for `uri`: an
/// `OperationOutcome` under the FHIR base, else a problem with the stack's
/// reason as the detail and the request id as the instance.
#[must_use]
pub fn stack_answer(
    fhir: &FhirBase,
    uri: &http::Uri,
    request_id: &str,
    status: StatusCode,
    reason: &str,
) -> Response {
    if fhir.covers(uri.path()) {
        return outcome(status, reason);
    }
    let refused = match status {
        StatusCode::SERVICE_UNAVAILABLE => Refused::of(Kind::Unavailable, reason),
        StatusCode::TOO_MANY_REQUESTS => Refused::of(Kind::RateLimited, reason),
        _ => Refused::blank(status, reason),
    };
    refused.render(Some(request_id), false).into_response()
}

/// The middleware over every FerroEHR route that writes each [`Refused`]
/// response for its request.
///
/// The server's request id becomes the `instance`, and a request that prefers
/// the representation receives the additional details.
///
/// A `412` that carries the latest version in its `ETag` (ITS-REST
/// `Requests_and_responses.md` §If-Match and accidental overwrites) names it
/// as `current_version` too. On a route under the FHIR base the refusal is
/// an `OperationOutcome`.
pub async fn finish(State(fhir): State<FhirBase>, request: Request, next: Next) -> Response {
    let representation = prefers_representation(request.headers());
    let request_id = request
        .extensions()
        .get::<RequestId>()
        .map(|id| id.id().to_owned());
    let fhir = fhir.covers(request.uri().path());
    let mut response = next.run(request).await;
    let Some(refused) = response.extensions_mut().remove::<Refused>() else {
        return response;
    };
    let mut refused = refused;
    if response.status() == StatusCode::PRECONDITION_FAILED
        && let Some(version) = response.headers().get(header::ETAG).and_then(version_of)
    {
        refused = refused.with_detail(Detail::CurrentVersion(version));
    }
    let (mut parts, _body) = response.into_parts();
    let rendered = if fhir {
        outcome(parts.status, refused.problem.detail().unwrap_or_default())
    } else {
        refused
            .render(request_id.as_deref(), representation)
            .into_response()
    };
    let (rendered_parts, body) = rendered.into_parts();
    for name in [header::CONTENT_TYPE, header::CONTENT_LENGTH] {
        parts.headers.remove(&name);
        if let Some(value) = rendered_parts.headers.get(&name) {
            parts.headers.insert(name, value.clone());
        }
    }
    if representation && !fhir {
        parts
            .headers
            .insert(PREFERENCE_APPLIED, HeaderValue::from_static(REPRESENTATION));
    }
    Response::from_parts(parts, body)
}

#[cfg(test)]
#[expect(
    clippy::disallowed_types,
    reason = "wire assertions are raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]
mod tests {
    use http::{HeaderMap, HeaderValue, StatusCode};

    use super::{
        BASE, Detail, Kind, Refused, Violation, prefers_representation, registry, version_of,
    };

    #[test]
    fn every_kind_is_registered_under_the_book_page() {
        let types = registry().expect("the registry builds");
        for kind in Kind::ALL {
            let defined = types.get(kind.slug()).expect("the kind is defined");
            assert_eq!(format!("{BASE}{}", kind.slug()), defined.uri());
            assert_eq!(Some(kind.title()), defined.title());
            assert_eq!(kind.status(), defined.status());
        }
        assert_eq!(Kind::ALL.len(), types.iter().count());
    }

    /// The book page every type URI resolves to documents each type, under
    /// the anchor of its slug, with its status and title (RFC 9457 §4).
    #[test]
    fn the_book_documents_every_kind() {
        let page = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../website/book/src/using-the-api/problems.md"
        ))
        .expect("the problem types page");
        for kind in Kind::ALL {
            let heading = format!("## `{}`", kind.slug());
            let section = page
                .split("\n## ")
                .find(|section| format!("## {section}").starts_with(&heading))
                .unwrap_or_else(|| panic!("no section {heading}"));
            let status = kind.status();
            assert!(
                section.contains(&format!(
                    "`{} {}`",
                    status.as_u16(),
                    status.canonical_reason().unwrap_or_default()
                )) || section.contains(&format!("`{} ", status.as_u16())),
                "{heading} names its status"
            );
            assert!(
                section.replace('\n', " ").contains(kind.title()),
                "{heading} names its title"
            );
        }
    }

    #[test]
    fn the_preference_is_read_from_every_prefer_field() {
        let mut headers = HeaderMap::new();
        assert!(!prefers_representation(&headers));
        headers.append("prefer", HeaderValue::from_static("respond-async, wait=10"));
        assert!(!prefers_representation(&headers));
        headers.append(
            "prefer",
            HeaderValue::from_static("return = representation; x=1"),
        );
        assert!(prefers_representation(&headers));
        let mut minimal = HeaderMap::new();
        minimal.insert("prefer", HeaderValue::from_static("return=minimal"));
        assert!(!prefers_representation(&minimal));
    }

    #[test]
    fn an_etag_names_its_version() {
        let uid = "8849182c-82ad-4088-a07f-48ead4180515::openEHRSys.example.com::2";
        for etag in [format!("W/\"{uid}\""), format!("\"{uid}\""), uid.to_owned()] {
            let value = HeaderValue::from_str(&etag).expect("a header value");
            assert_eq!(
                Some(uid.to_owned()),
                version_of(&value),
                "each ETag form names its version"
            );
        }
    }

    #[test]
    fn the_additional_details_need_the_preference() {
        let refused = Refused::of(Kind::ValidationFailed, "the content fails validation")
            .with_detail(Detail::Errors(vec![Violation {
                path: "/a".to_owned(),
                detail: "b".to_owned(),
            }]));
        let id = "67e55044-10b1-426f-9247-bb680e5fe0c8";
        let minimal = refused.render(Some(id), false).to_json();
        assert_eq!(
            serde_json::json!({
                "type": format!("{BASE}validation-failed"),
                "title": "The content fails validation",
                "status": 422,
                "detail": "the content fails validation",
                "instance": format!("urn:uuid:{id}"),
            }),
            minimal
        );
        let full = refused.render(Some(id), true).to_json();
        assert_eq!(
            serde_json::json!([{"path": "/a", "detail": "b"}]),
            full["errors"]
        );
        assert_eq!(StatusCode::UNPROCESSABLE_ENTITY, refused.status());
    }
}
