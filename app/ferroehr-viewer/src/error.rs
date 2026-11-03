// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer-wide error type, so a page can draw domain errors (CDR status +
//! diagnostic) instead of opaque strings.
//!
//! A variant caused by another error carries it as a [`Cause`], walked by
//! [`std::error::Error::source`]: a page's model holds the refusal it draws
//! and is compared and cloned with it, so the cause is shared and compared by
//! its rendered chain.

/// Every failure a read of the CDR can hand a page.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ViewerError {
    /// No viewer session — the UI redirects to `/login`.
    #[error("not authenticated")]
    Unauthenticated,
    /// The CDR answered `401`: the credential this session carries is no
    /// longer valid for the CDR, whatever the viewer's own session says.
    #[error("unauthorized: {0}")]
    CdrUnauthorized(String),
    /// The CDR answered `403`: the credential is valid and the CDR refuses to
    /// authorize it — the wrong-role / insufficient-scope surface.
    #[error("forbidden: {0}")]
    Forbidden(String),
    /// A non-2xx CDR answer, normalized: status + the diagnostic the CDR
    /// returned in its error body.
    #[error("CDR answered {status}: {message}")]
    Cdr {
        /// The CDR's HTTP status code.
        status: u16,
        /// The diagnostic extracted from the CDR error body (or the raw body).
        message: String,
    },
    /// The CDR was unreachable (connect/timeout/transport).
    #[error("CDR unreachable")]
    CdrUnreachable(#[source] Cause),
    /// Bad user input (login form, AQL text, template upload) with the
    /// message to render inline.
    #[error("{0}")]
    Invalid(String),
    /// Viewer-internal failure (config, session store, serialization).
    #[error("internal error: {0}")]
    Internal(String),
    /// A viewer-internal failure another error caused: `step` names what
    /// failed, in the reader's language.
    #[error("internal error: {step}")]
    Failed {
        /// What failed.
        step: String,
        /// The error that caused it.
        #[source]
        cause: Cause,
    },
    /// Bad user input another error found, such as a JSON parse: `step`
    /// names what was refused, in the reader's language.
    #[error("{step}")]
    Rejected {
        /// What was refused.
        step: String,
        /// The error that found it.
        #[source]
        cause: Cause,
    },
}

/// The error that caused a [`ViewerError`], shared so the error stays
/// [`Clone`]; two causes are equal when they render the same chain.
#[derive(Debug, Clone)]
pub struct Cause(std::sync::Arc<dyn std::error::Error + Send + Sync>);

impl Cause {
    /// Wrap `error` as a cause.
    #[must_use]
    pub fn new(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self(std::sync::Arc::new(error))
    }
}

impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

impl std::error::Error for Cause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.0.source()
    }
}

impl PartialEq for Cause {
    fn eq(&self, other: &Self) -> bool {
        error_chain(self) == error_chain(other)
    }
}

impl Eq for Cause {}

/// Renders an error and every cause under it on one line, `: `-separated.
///
/// The text a [`ViewerError`] carries is a rendered cause, so it is rendered
/// from the whole chain: an error whose message names only its own step keeps
/// the diagnostic in its [`std::error::Error::source`]
/// ([RFC 0201](https://rust-lang.github.io/rfcs/0201-error-chaining.html)).
#[must_use]
pub fn error_chain(error: &(dyn std::error::Error + 'static)) -> String {
    let mut text = error.to_string();
    let mut next = error.source();
    while let Some(cause) = next {
        text.push_str(": ");
        text.push_str(&cause.to_string());
        next = cause.source();
    }
    text
}

impl ViewerError {
    /// A viewer-internal failure of `step`, caused by `error`.
    #[must_use]
    pub fn failed(
        step: impl Into<String>,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Failed {
            step: step.into(),
            cause: Cause::new(error),
        }
    }

    /// Bad user input `step` names, found by `error`.
    #[must_use]
    pub fn rejected(
        step: impl Into<String>,
        error: impl std::error::Error + Send + Sync + 'static,
    ) -> Self {
        Self::Rejected {
            step: step.into(),
            cause: Cause::new(error),
        }
    }

    /// The CDR did not answer: `error` is the transport failure.
    #[must_use]
    pub fn unreachable(error: impl std::error::Error + Send + Sync + 'static) -> Self {
        Self::CdrUnreachable(Cause::new(error))
    }

    /// The text a refusal shows under its code: the CDR's diagnostic or the
    /// message, and for a caused error its step with the whole cause chain;
    /// `None` when the session simply ended.
    #[must_use]
    pub fn detail(&self) -> Option<String> {
        match self {
            Self::Cdr { message, .. }
            | Self::CdrUnauthorized(message)
            | Self::Forbidden(message)
            | Self::Invalid(message)
            | Self::Internal(message) => Some(message.clone()),
            Self::CdrUnreachable(cause) => Some(error_chain(cause)),
            Self::Failed { step, cause } | Self::Rejected { step, cause } => {
                Some(format!("{step}: {}", error_chain(cause)))
            }
            Self::Unauthenticated => None,
        }
    }

    /// The CDR's HTTP status, typed, when this error carries one.
    ///
    /// [`ViewerError::Cdr`] carries the status as a `u16`; every branch on it
    /// reads it back
    /// through here, so a status comparison names an [`http::StatusCode`]
    /// constant instead of a bare literal. The two refusals answer their own
    /// status, so a caller can tell "sign in again" from "sign in as someone
    /// else"; [`ViewerError::Unauthenticated`] answers `None` because no CDR
    /// request was ever made.
    #[must_use]
    pub fn status_code(&self) -> Option<http::StatusCode> {
        match self {
            Self::Cdr { status, .. } => http::StatusCode::from_u16(*status).ok(),
            Self::CdrUnauthorized(_) => Some(http::StatusCode::UNAUTHORIZED),
            Self::Forbidden(_) => Some(http::StatusCode::FORBIDDEN),
            Self::Unauthenticated
            | Self::CdrUnreachable(_)
            | Self::Invalid(_)
            | Self::Internal(_)
            | Self::Failed { .. }
            | Self::Rejected { .. } => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ViewerError, error_chain};

    /// A failure another error caused keeps that error as its source, walked
    /// once by the chain, and its clone compares equal to it.
    #[test]
    fn a_caused_failure_keeps_its_source() {
        let parse = serde_json::from_str::<u32>("{").expect_err("an open object");
        let detail = parse.to_string();
        let err = ViewerError::failed("readiness JSON", parse);
        let source = std::error::Error::source(&err).expect("the parse error is the source");
        assert_eq!(source.to_string(), detail);
        assert_eq!(
            error_chain(&err),
            format!("internal error: readiness JSON: {detail}")
        );
        assert_eq!(err.clone(), err);
        assert_ne!(
            err,
            ViewerError::failed("readiness JSON", std::io::Error::other("other"))
        );
        let unreachable = ViewerError::unreachable(std::io::Error::other("connection refused"));
        assert_eq!(
            error_chain(&unreachable),
            "CDR unreachable: connection refused"
        );
    }

    /// A parse error names its pass and keeps the lexer diagnostic as its
    /// source; the rendered text carries both, once each.
    #[test]
    fn a_lex_failure_renders_with_its_diagnostic() {
        let err = openehr_query::parser::parse_str("SELECT # FROM EHR e")
            .expect_err("`#` lexes as nothing");
        let rendered = error_chain(&err);
        assert!(
            rendered.starts_with("the query could not be tokenised: "),
            "{rendered}"
        );
        assert_eq!(rendered.matches("\"#\"").count(), 1, "{rendered}");
    }
}
