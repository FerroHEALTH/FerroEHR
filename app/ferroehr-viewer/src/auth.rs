// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Viewer authentication: sign-in, sign-out and the session's state.
//!
//! Every route is a public HTTP endpoint, so each one enforces the session
//! itself; the CDR credential never leaves the server.

use ferrohealth_secret::secret::Secret;
use ferrohealth_session::return_path::ReturnPath;
use serde::Deserialize;

use crate::error::ViewerError;

/// One deployment-configured link rendered under the sign-in card.
///
/// Doubles as the `[login.links]` configuration entry, so the page
/// and the configuration cannot drift; both fields are required there.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginLink {
    /// Visible link text.
    pub label: String,
    /// Absolute or same-origin URL the link opens.
    pub href: String,
}

/// What the login screen offers and says: the available modes plus the
/// deployment-configured notice and links.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoginScreen {
    /// Offer the Basic (username/password) form.
    pub basic: bool,
    /// Offer the OIDC login button.
    pub oidc: bool,
    /// Informational text on the sign-in card (empty = none).
    pub notice: String,
    /// Links rendered under the sign-in card.
    pub links: Vec<LoginLink>,
}

/// The shell root, and the destination every refused `next` falls back to.
const DEFAULT_NEXT: &str = "/";

/// Returns the post-login destination a `next` value is allowed to name.
///
/// Only a page of this viewer is honoured, as
/// `ferrohealth_session::return_path::ReturnPath` reads one: a single leading
/// `/` (never `//` or a `\`, which a browser follows to another site),
/// visible ASCII only, and neither a sign-in route nor the sign-in page.
/// Everything else falls back to the shell root, because an open redirect on
/// a sign-in route is a phishing primitive. Both sign-in paths (Basic and
/// OIDC) are held to the same rule.
#[must_use]
pub(crate) fn same_origin_next(raw: Option<&str>) -> String {
    raw.and_then(|raw| ReturnPath::parse(raw, crate::session::SIGN_IN_PAGE))
        .map_or_else(|| DEFAULT_NEXT.to_owned(), |path| path.as_str().to_owned())
}

/// What a Basic sign-in came to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignedIn {
    /// The CDR accepted the credentials: the session is kept, and the reader
    /// goes on to the page named, filtered through `same_origin_next`.
    Accepted(String),
    /// The CDR refused the username or password (`401`).
    Refused,
}

/// Basic sign-in: checks the credentials against the CDR with an
/// authenticated ITS-REST read, then begins the session that carries them.
///
/// Credentials the CDR refuses are not an error: the answer says so, and the
/// sign-in page draws the refusal with the username kept. The session cookie
/// is appended to the response of the page being drawn.
///
/// # Errors
/// [`ViewerError::Invalid`] when Basic sign-in is off,
/// [`ViewerError::Cdr`] / [`ViewerError::CdrUnreachable`] when the CDR
/// misbehaves, [`ViewerError::Failed`] when the store refuses the session,
/// [`ViewerError::Internal`] outside a request.
pub async fn sign_in_basic(
    username: String,
    password: String,
    next: Option<String>,
) -> Result<SignedIn, ViewerError> {
    let state = crate::scope::state()?;
    if !state.config.auth.basic_enabled {
        return Err(ViewerError::Invalid("Basic login is disabled".to_owned()));
    }
    let password = Secret::new(password);
    let credential = ferrohealth_session::record::Credential::Basic {
        username: username.clone(),
        password: password.clone(),
    };
    // An authenticated, cheap ITS-REST read: the ADL 1.4 template list
    // (Definition API). 401 = wrong credentials; with CDR auth disabled the
    // probe trivially passes, which is the dev posture.
    let url = state.cdr.rest_v1("definition/template/adl1.4");
    let probe = state
        .cdr
        .call(http::Method::GET, &url, &credential)
        .send()
        .await?;
    if probe.is(http::StatusCode::UNAUTHORIZED) {
        return Ok(SignedIn::Refused);
    }
    if probe.status.is_server_error() {
        return Err(ViewerError::Cdr {
            status: probe.status.as_u16(),
            message: "CDR error while validating credentials".to_owned(),
        });
    }

    crate::session::begin(ferrohealth_session::record::SignedIn::basic(
        username, password,
    ))?;
    Ok(SignedIn::Accepted(same_origin_next(next.as_deref())))
}

/// Which login modes the login screen should offer, plus the configured
/// notice and links it renders.
///
/// # Errors
/// [`ViewerError::Internal`] outside a request.
pub async fn fetch_login_screen() -> Result<LoginScreen, ViewerError> {
    let state = crate::scope::state()?;
    // The viewer offers only what BOTH sides support: its own configured
    // modes intersected with the schemes the CDR advertises in its
    // `WWW-Authenticate` challenge (a Basic form against a bearer-only CDR
    // can never succeed). An unreachable CDR falls back to the viewer's
    // config alone so the login page still renders — the login attempt
    // itself then surfaces the outage.
    let (cdr_basic, cdr_bearer) = state.cdr.advertised_schemes().await.unwrap_or((true, true));
    Ok(LoginScreen {
        basic: state.config.auth.basic_enabled && cdr_basic,
        oidc: state.config.auth.oidc_enabled && cdr_bearer,
        notice: state.config.login.notice.clone(),
        links: state.config.login.links.clone(),
    })
}

/// The CDR status document (`{rest root}/status`, a public endpoint), raw
/// JSON: the shell's health pill and the system panel both read it.
///
/// # Errors
/// The errors of a CDR call ([`cdr`](crate::bff::cdr));
/// [`ViewerError::Unauthenticated`] without a session (the shell only polls
/// when logged in).
pub async fn fetch_status() -> Result<String, ViewerError> {
    let cdr = crate::bff::cdr()?;
    let url = cdr.rest_root_url("status");
    Ok(cdr.public(http::Method::GET, &url).ok().await?.body)
}

#[cfg(test)]
mod tests {
    use super::same_origin_next;

    /// The sign-in routes' `next` is attacker-supplied by construction — it
    /// arrives in a link anyone can craft — so what it accepts is the whole
    /// security property, and it is pinned from both sides.
    #[test]
    fn only_a_same_origin_relative_path_survives_as_the_post_login_destination() {
        assert_eq!(same_origin_next(Some("/ehrs?x=1")), "/ehrs?x=1");
        assert_eq!(same_origin_next(Some("/")), "/");
        assert_eq!(
            same_origin_next(Some("/ehrs/01a06375?tab=status#top")),
            "/ehrs/01a06375?tab=status#top"
        );

        // Protocol-relative: a browser reads both as "another origin".
        assert_eq!(same_origin_next(Some("//evil.example")), "/");
        assert_eq!(same_origin_next(Some("/\\evil.example")), "/");
        // Absolute and scheme-bearing values never begin with `/` at all.
        assert_eq!(same_origin_next(Some("https://evil.example")), "/");
        assert_eq!(same_origin_next(Some("javascript:alert(1)")), "/");
        assert_eq!(same_origin_next(Some("ehrs")), "/");
        // Nothing that no `Location` header could carry.
        assert_eq!(same_origin_next(Some("/ehrs\r\nSet-Cookie: x=1")), "/");
        assert_eq!(same_origin_next(Some("/ehrs with space")), "/");
        // Absent or empty.
        assert_eq!(same_origin_next(None), "/");
        assert_eq!(same_origin_next(Some("")), "/");
        // The sign-in page and the sign-in routes are never a destination.
        assert_eq!(same_origin_next(Some("/sign-in?expired=1")), "/");
        assert_eq!(same_origin_next(Some("/login")), "/");
        assert_eq!(same_origin_next(Some("/auth/callback?code=x")), "/");
    }
}
