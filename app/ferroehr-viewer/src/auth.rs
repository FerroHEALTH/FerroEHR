// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Viewer authentication: the login/logout/session server functions.
//!
//! Server functions are a public HTTP API — each one enforces auth itself
//! (Leptos book `server/25`); the CDR credential never leaves the server.

use leptos::server;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;

/// One deployment-configured link rendered under the sign-in card.
///
/// Doubles as the `[login.links]` configuration entry (ssr side), so the wire
/// and the configuration cannot drift; both fields are required there.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginLink {
    /// Visible link text.
    pub label: String,
    /// Absolute or same-origin URL the link opens.
    pub href: String,
}

/// What the login screen offers and says: the available modes plus the
/// deployment-configured notice and links.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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

/// What the UI may know about the current session (never the credential).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionInfo {
    /// Display identity.
    pub identity: String,
    /// `"basic"` or `"oidc"`.
    pub method: String,
    /// OIDC token scopes (empty for Basic).
    pub scopes: Vec<String>,
    /// Seconds left before this session's idle window closes, as of the cookie
    /// the request carried.
    ///
    /// The anchor slides on every authenticated response, so this is a lower
    /// bound on the real deadline; the browser schedules its own re-check on
    /// it, which is what catches an expiry nobody clicks through. A `u32`
    /// because WASM is 32-bit and this crosses the server-fn boundary.
    pub expires_in_secs: u32,
}

/// Basic login: validate the credentials against the CDR (an authenticated
/// ITS-REST call), then store them in the server-side session.
///
/// # Errors
/// [`ViewerError::Invalid`] on wrong credentials, [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] when the CDR misbehaves.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn login_basic(
    /// The CDR username the viewer authenticates as.
    username: String,
    /// The matching CDR password; it never leaves the server.
    password: String,
    /// Where to land after a successful login; the dashboard when absent.
    next: Option<String>,
) -> Result<(), ViewerError> {
    let state: crate::state::AppState = leptos::prelude::expect_context();
    if !state.config.auth.basic_enabled {
        return Err(ViewerError::Invalid("Basic login is disabled".to_owned()));
    }
    let credential = crate::session::Credential::Basic {
        username: username.clone(),
        password,
    };
    // An authenticated, cheap ITS-REST read: the ADL 1.4 template list
    // (Definition API). 401 = wrong credentials; with CDR auth disabled the
    // probe trivially passes, which is the dev posture.
    let url = state.cdr.rest_v1("definition/template/adl1.4");
    let probe = state.cdr.get(&credential, &url, "application/json").await?;
    if probe.is(http::StatusCode::UNAUTHORIZED) {
        return Err(ViewerError::Invalid(
            "wrong username or password".to_owned(),
        ));
    }
    if probe.status.is_server_error() {
        return Err(ViewerError::Cdr {
            status: probe.status.as_u16(),
            message: "CDR error while validating credentials".to_owned(),
        });
    }

    let mut session = crate::session::http_session().await?;
    session.insert(
        crate::session::SESSION_KEY,
        &crate::session::ViewerSession {
            identity: username,
            credential,
            scopes: Vec::new(),
        },
    )?;
    crate::session::commit(&session)?;

    leptos_axum::redirect(next.as_deref().unwrap_or("/"));
    Ok(())
}

/// Destroy the session and land on `/login`.
///
/// NOTE: deliberately not guarded by `require_session` — flushing the
/// caller's own cookie-scoped session is idempotent and leaks nothing; an
/// unauthenticated call is a harmless no-op redirect.
///
/// # Errors
/// [`ViewerError::Internal`] outside a request or on a sealing failure.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn logout() -> Result<(), ViewerError> {
    // The extraction is the in-request proof (it fails outside one); its
    // decoded content is irrelevant — sign-out overwrites the cookie.
    crate::session::http_session().await?;
    // Committing the EMPTY session sets the removal cookie.
    crate::session::commit(&crate::session::CookieSession::default())?;
    leptos_axum::redirect("/login");
    Ok(())
}

/// The current session, if any — drives the shell's user menu and the
/// scope panel. Deliberately NOT guarded: "no session" is a valid answer.
///
/// # Errors
/// [`ViewerError::Internal`] when called outside a request (a bug).
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn current_session() -> Result<Option<SessionInfo>, ViewerError> {
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let session = crate::session::http_session().await?;
    let expires_in_secs = session.expires_in_secs(state.config.session.idle_minutes);
    let admin = session.get::<crate::session::ViewerSession>(crate::session::SESSION_KEY);
    Ok(admin.map(|s| SessionInfo {
        identity: s.identity,
        method: match s.credential {
            crate::session::Credential::Basic { .. } => "basic".to_owned(),
            crate::session::Credential::Bearer { .. } => "oidc".to_owned(),
        },
        scopes: s.scopes,
        expires_in_secs,
    }))
}

/// Which login modes the login screen should offer, plus the configured
/// notice and links it renders.
///
/// # Errors
/// None in practice; the signature is fallible per the server-fn contract.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_login_screen() -> Result<LoginScreen, ViewerError> {
    let state: crate::state::AppState = leptos::prelude::expect_context();
    // The viewer offers only what BOTH sides support: its own configured
    // modes intersected with the schemes the CDR advertises in its
    // `WWW-Authenticate` challenge (a Basic form against a bearer-only CDR
    // can never succeed). An unreachable CDR falls back to the viewer's
    // config alone so the login page still renders — the login attempt
    // itself then surfaces the outage.
    let (cdr_basic, cdr_bearer) = state.cdr.advertised_schemes().await.unwrap_or((true, true));
    Ok(LoginScreen {
        basic: state.config.auth.basic_enabled && cdr_basic,
        oidc: state.config.auth.oidc.enabled && cdr_bearer,
        notice: state.config.login.notice.clone(),
        links: state.config.login.links.clone(),
    })
}

/// The CDR status document (`{rest root}/status`, a public endpoint), raw
/// JSON: the shell's health pill and the system panel both read it.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a session (the shell only
/// polls when logged in); CDR transport errors pass through.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_status() -> Result<String, ViewerError> {
    crate::session::require_session().await?;
    let state: crate::state::AppState = leptos::prelude::expect_context();
    let url = state.cdr.rest_root_url("status");
    let response = state.cdr.get_public(&url, "application/json").await?;
    Ok(crate::cdr::CdrClient::expect_success(response)?.body)
}
