// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer-wide error type, shared by both compilation targets so the UI
//! can render domain errors (CDR status + diagnostic) instead of opaque
//! strings.
//!
//! Implements `FromServerFnError` per the Leptos book (`server/25`).
//!
//! NOTE: the variants carry their cause as text because this type crosses the
//! server-fn boundary — `FromServerFnError` requires `Serialize`/`Deserialize`
//! (Leptos book `server/25`), which no underlying `reqwest` or parser error
//! implements, so an `#[source]` here would be unrepresentable rather than
//! merely inconvenient.

use serde::{Deserialize, Serialize};

/// Every failure a server function can hand the UI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
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

impl ViewerError {
    /// The CDR's HTTP status, typed, when this error carries one.
    ///
    /// [`ViewerError::Cdr`] transports the status as a `u16` because the enum
    /// crosses the server-fn boundary as JSON; every branch on it reads it back
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

impl leptos::server_fn::error::FromServerFnError for ViewerError {
    type Encoder = leptos::server_fn::codec::JsonEncoding;

    fn from_server_fn_error(value: leptos::server_fn::error::ServerFnErrorErr) -> Self {
        Self::Internal(value.to_string())
    }
}
