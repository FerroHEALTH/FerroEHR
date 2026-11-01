// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer-wide error type, so a page can draw domain errors (CDR status +
//! diagnostic) instead of opaque strings.
//!
//! NOTE: the variants carry their cause as text because a page's model holds the
//! refusal it draws and is compared and cloned with it, which no `reqwest` error allows.

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
    #[error("CDR unreachable: {0}")]
    CdrUnreachable(String),
    /// Bad user input (login form, AQL text, template upload) with the
    /// message to render inline.
    #[error("{0}")]
    Invalid(String),
    /// Viewer-internal failure (config, session store, serialization).
    #[error("internal error: {0}")]
    Internal(String),
}

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
            | Self::Internal(_) => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::error_chain;

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
