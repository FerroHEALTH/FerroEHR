// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's sign-in and session, which `ferrohealth-session` keeps.
//!
//! The crate's `[oidc]` and `[session]` sections configure them, its store
//! keeps the session (in the process, or in one sealed cookie every
//! instance opens), its routes run the OpenID Connect sign-in, the sign-out
//! and the session watch, and its middleware hands every request the
//! [`Held`] it carries. This module is the viewer's side of that seam: the
//! cookie names, the pages a stopped sign-in and a sign-out land on, the
//! session a request holds, and the Basic sign-in the viewer checks against
//! the CDR itself. No openEHR spec governs the viewer session: our own
//! design.

use axum::response::{IntoResponse as _, Response};
use ferrohealth_session::cookies::CookieNames;
use ferrohealth_session::record::{Held, Session, SignedIn};
use ferrohealth_session::routes::{Pages, Stopped};
use http::request::Parts;

use crate::error::ViewerError;
use crate::locale::signin::SignInMsg;

/// The base name of the viewer's cookies (`ferroehr_viewer_session` and its
/// siblings, `__Host-` prefixed when they carry `Secure`).
pub const COOKIE_BASE: &str = "ferroehr_viewer";

/// The path of the viewer's own sign-in page, which is never a return path.
pub const SIGN_IN_PAGE: &str = "/sign-in";

/// Where a sign-out lands: at once when it ends no session at the provider,
/// and through the provider as `oidc.post_logout_redirect_uri` when it does.
pub const SIGNED_OUT_PAGE: &str = "/sign-in?signed-out=1";

/// The viewer's cookie names.
#[must_use]
pub fn cookie_names() -> CookieNames {
    CookieNames::for_console(COOKIE_BASE)
}

/// The pages the crate's sign-in routes land on.
#[derive(Debug, Clone, Copy, Default)]
pub struct ViewerPages;

impl Pages for ViewerPages {
    fn stopped(&self, _parts: Parts, why: Stopped) -> impl Future<Output = Response> + Send {
        let status = why.status();
        tracing::warn!(%status, ?why, "the OpenID Connect sign-in stopped");
        let answer = (status, crate::locale::tr(SignInMsg::RefusedProvider)).into_response();
        std::future::ready(answer)
    }

    fn sign_in_page(&self) -> &'static str {
        SIGN_IN_PAGE
    }

    fn signed_out(&self) -> &'static str {
        SIGNED_OUT_PAGE
    }

    fn home(&self) -> &'static str {
        "/"
    }
}

/// The error of a read that needs the request but runs outside one, a defect
/// of its caller.
#[must_use]
pub fn outside_request() -> ViewerError {
    ViewerError::Internal("no request in scope".to_owned())
}

/// What the request being drawn holds, as the crate's middleware found it.
///
/// # Errors
/// [`ViewerError::Internal`] outside a request (a bug).
pub fn held() -> Result<Held, ViewerError> {
    crate::scope::with(|scope| {
        scope
            .parts
            .extensions
            .get::<Held>()
            .cloned()
            .unwrap_or(Held::Absent)
    })
    .ok_or_else(outside_request)
}

/// The guard: the live session the request holds, or `Unauthenticated`.
///
/// Every read of the CDR or of the session state calls this first. No
/// cookie, a cookie that names no live session and a session whose access
/// token could not be renewed are the same answer:
/// [`ViewerError::Unauthenticated`].
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a live session;
/// [`ViewerError::Internal`] outside a request (a bug).
pub fn require_session() -> Result<Session, ViewerError> {
    let session = match held()? {
        Held::SignedIn(session) => Ok(session),
        Held::TokenExpired | Held::Ended | Held::Absent => Err(ViewerError::Unauthenticated),
    };
    if let Err(error) = &session {
        crate::session_client::note_if_over(error);
    }
    session
}

/// Begins the session of `signed_in`, a sign-in the viewer checked itself:
/// the session the request held is ended first, and the new session's
/// cookie is appended to the answer of the request being drawn.
///
/// # Errors
/// [`ViewerError::Failed`] when the store refuses the session;
/// [`ViewerError::Internal`] outside a request (a bug).
pub fn begin(signed_in: SignedIn) -> Result<(), ViewerError> {
    let state = crate::scope::state()?;
    let headers =
        crate::scope::with(|scope| scope.parts.headers.clone()).ok_or_else(outside_request)?;
    let cookie = state
        .sign_in
        .sessions()
        .establish(&headers, signed_in)
        .map_err(|error| ViewerError::failed(crate::locale::tr(SignInMsg::Title), error))?;
    crate::scope::with(|scope| scope.answer(|answer| answer.cookies.push(cookie)))
        .ok_or_else(outside_request)
}

/// The name the shell shows for `session`: the operator's name, else the
/// subject.
#[must_use]
pub fn identity(session: &Session) -> String {
    session
        .operator()
        .or_else(|| session.subject())
        .unwrap_or_default()
        .to_owned()
}
