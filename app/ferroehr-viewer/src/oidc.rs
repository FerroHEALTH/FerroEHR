// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer's OIDC authorization-code login (with PKCE): two plain axum
//! routes (`/auth/oidc/login`, `/auth/oidc/callback`) on the BFF.
//!
//! The bearer token is stored server-side in the session; the browser only
//! ever holds the viewer's session cookie.

use axum::response::{IntoResponse, Redirect, Response};
use http::StatusCode;
use openidconnect::{OAuth2TokenResponse, TokenResponse};
use serde::{Deserialize, Serialize};

use crate::config::OidcConfig;

/// Discovery output kept in [`crate::state::AppState`].
#[derive(Debug)]
pub struct OidcState {
    metadata: openidconnect::core::CoreProviderMetadata,
    http: openidconnect::reqwest::Client,
    config: OidcConfig,
}

/// The transient state parked in the session between login and callback.
#[derive(Debug, Serialize, Deserialize)]
struct FlowState {
    pkce_verifier: String,
    csrf: String,
    nonce: String,
    next: String,
}

const FLOW_KEY: &str = "oidc_flow";

/// The shell root, and the destination every refused `next` falls back to.
const DEFAULT_NEXT: &str = "/";

/// `/auth/oidc/login`'s query: where the completed round trip should land.
#[derive(Debug, Deserialize)]
pub struct LoginParams {
    next: Option<String>,
}

/// The post-login destination a `next` value is allowed to name.
///
/// Only a same-origin relative path is honoured. The value must begin with a
/// single `/` — never `//` or `/\`, which a browser reads as protocol-relative
/// and would follow to another site — and carry only visible ASCII, so nothing
/// can smuggle a control character into the `Location` header this ends up in.
/// Everything else (an absolute URL, a `javascript:` payload, an empty value)
/// falls back to [`DEFAULT_NEXT`], because an open redirect on the login route
/// is a phishing primitive: it lends this deployment's origin to someone
/// else's page at the exact moment the user has just proved who they are.
fn same_origin_next(raw: Option<&str>) -> String {
    let candidate = raw.unwrap_or_default();
    let mut bytes = candidate.bytes();
    if bytes.next() != Some(b'/') || matches!(bytes.next(), Some(b'/' | b'\\')) {
        return DEFAULT_NEXT.to_owned();
    }
    if candidate.bytes().all(|byte| byte.is_ascii_graphic()) {
        candidate.to_owned()
    } else {
        DEFAULT_NEXT.to_owned()
    }
}

/// Run OIDC discovery against the configured issuer.
///
/// # Errors
/// `anyhow::Error` on an unreachable/invalid issuer (boot-time failure —
/// the binary refuses to start with a broken OIDC config).
pub async fn discover(config: &OidcConfig) -> anyhow::Result<OidcState> {
    let mut builder = openidconnect::reqwest::ClientBuilder::new()
        .redirect(openidconnect::reqwest::redirect::Policy::none()); // SSRF hardening
    // Split-horizon issuer resolution (`host=ip:port`): the canonical
    // issuer hostname may only resolve inside a container network; the
    // override points this client at the mapped address while every URL
    // (and the token `iss`) keeps the canonical form.
    if !config.resolve.trim().is_empty() {
        let (host, addr) = config
            .resolve
            .split_once('=')
            .ok_or_else(|| anyhow::anyhow!("auth.oidc.resolve must be `host=ip:port`"))?;
        builder = builder.resolve(host.trim(), addr.trim().parse()?);
    }
    let http = builder.build()?;
    let issuer = openidconnect::IssuerUrl::new(config.issuer.clone())?;
    let metadata = openidconnect::core::CoreProviderMetadata::discover_async(issuer, &http).await?;
    Ok(OidcState {
        metadata,
        http,
        config: config.clone(),
    })
}

impl OidcState {
    fn client(
        &self,
    ) -> anyhow::Result<
        openidconnect::core::CoreClient<
            openidconnect::EndpointSet,
            openidconnect::EndpointNotSet,
            openidconnect::EndpointNotSet,
            openidconnect::EndpointNotSet,
            openidconnect::EndpointMaybeSet,
            openidconnect::EndpointMaybeSet,
        >,
    > {
        let redirect = openidconnect::RedirectUrl::new(format!(
            "{}/auth/oidc/callback",
            self.config.public_base_url.trim_end_matches('/')
        ))?;
        Ok(openidconnect::core::CoreClient::from_provider_metadata(
            self.metadata.clone(),
            openidconnect::ClientId::new(self.config.client_id.clone()),
            Some(openidconnect::ClientSecret::new(
                self.config.client_secret.clone(),
            )),
        )
        .set_redirect_uri(redirect))
    }
}

fn error_page(status: StatusCode, message: &str) -> Response {
    tracing::warn!(%status, message, "OIDC flow failed");
    (status, format!("OIDC login failed: {message}")).into_response()
}

/// `GET /auth/oidc/login` — park PKCE/CSRF/nonce plus the post-login
/// destination in the sealed session cookie and redirect to the authorization
/// endpoint.
///
/// `?next=` names where the completed round trip lands, filtered through
/// `same_origin_next`; it rides the sealed cookie rather than the OAuth2
/// `state` parameter, so the provider never sees it and it cannot be edited
/// between the two legs.
pub async fn login(
    axum::Extension(state): axum::Extension<crate::state::AppState>,
    axum::extract::Query(params): axum::extract::Query<LoginParams>,
    headers: http::HeaderMap,
) -> Response {
    let Some(oidc) = state.oidc.as_ref() else {
        return error_page(StatusCode::NOT_FOUND, "OIDC is not enabled");
    };
    let client = match oidc.client() {
        Ok(client) => client,
        Err(e) => return error_page(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let (pkce_challenge, pkce_verifier) = openidconnect::PkceCodeChallenge::new_random_sha256();
    let mut request = client
        .authorize_url(
            openidconnect::core::CoreAuthenticationFlow::AuthorizationCode,
            openidconnect::CsrfToken::new_random,
            openidconnect::Nonce::new_random,
        )
        .set_pkce_challenge(pkce_challenge);
    for scope in &oidc.config.scopes {
        request = request.add_scope(openidconnect::Scope::new(scope.clone()));
    }
    let (auth_url, csrf, nonce) = request.url();

    let flow = FlowState {
        pkce_verifier: pkce_verifier.secret().clone(),
        csrf: csrf.secret().clone(),
        nonce: nonce.secret().clone(),
        next: same_origin_next(params.next.as_deref()),
    };
    let mut session = crate::session::unseal(
        &state.session_keys,
        &headers,
        state.config.session.idle_minutes,
    );
    if let Err(e) = session.insert(FLOW_KEY, &flow) {
        return error_page(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    with_session_cookie(&state, &session, Redirect::to(auth_url.as_str()))
}

/// Attach the sealed session cookie to a handler's response.
fn with_session_cookie(
    state: &crate::state::AppState,
    session: &crate::session::CookieSession,
    response: impl IntoResponse,
) -> Response {
    let sealed = match crate::session::set_cookie(
        &state.session_keys,
        session,
        state.config.session.cookie_secure,
        state.config.session.idle_minutes,
    ) {
        Ok(sealed) => sealed,
        Err(e) => return error_page(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let mut response = response.into_response();
    response
        .headers_mut()
        .append(http::header::SET_COOKIE, sealed);
    response
}

/// The callback's query parameters (only the success pair; a provider
/// error arrives as `error`/`error_description`).
#[derive(Debug, Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
    error_description: Option<String>,
}

/// `GET /auth/oidc/callback` — exchange the code (PKCE), validate the ID
/// token nonce, store the session, and land on the destination the login leg
/// parked (the shell root when there was none).
pub async fn callback(
    axum::Extension(state): axum::Extension<crate::state::AppState>,
    headers: http::HeaderMap,
    axum::extract::Query(params): axum::extract::Query<CallbackParams>,
) -> Response {
    let Some(oidc) = state.oidc.as_ref() else {
        return error_page(StatusCode::NOT_FOUND, "OIDC is not enabled");
    };
    if let Some(error) = params.error {
        let detail = params.error_description.unwrap_or_default();
        return error_page(StatusCode::BAD_REQUEST, &format!("{error} {detail}"));
    }
    let (Some(code), Some(state_param)) = (params.code, params.state) else {
        return error_page(StatusCode::BAD_REQUEST, "missing code/state");
    };
    let mut session = crate::session::unseal(
        &state.session_keys,
        &headers,
        state.config.session.idle_minutes,
    );
    let Some(flow): Option<FlowState> = session.remove(FLOW_KEY) else {
        return error_page(StatusCode::BAD_REQUEST, "no login in progress");
    };
    if flow.csrf != state_param {
        return error_page(StatusCode::BAD_REQUEST, "state mismatch");
    }

    let client = match oidc.client() {
        Ok(client) => client,
        Err(e) => return error_page(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let exchange = match client.exchange_code(openidconnect::AuthorizationCode::new(code)) {
        Ok(exchange) => {
            exchange.set_pkce_verifier(openidconnect::PkceCodeVerifier::new(flow.pkce_verifier))
        }
        Err(e) => return error_page(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string()),
    };
    let tokens = match exchange.request_async(&oidc.http).await {
        Ok(tokens) => tokens,
        Err(e) => return error_page(StatusCode::BAD_GATEWAY, &e.to_string()),
    };

    let Some(id_token) = tokens.id_token() else {
        return error_page(StatusCode::BAD_GATEWAY, "no ID token in response");
    };
    let nonce = openidconnect::Nonce::new(flow.nonce);
    let claims = match id_token.claims(&client.id_token_verifier(), &nonce) {
        Ok(claims) => claims,
        Err(e) => return error_page(StatusCode::BAD_REQUEST, &e.to_string()),
    };
    let identity = claims.preferred_username().map_or_else(
        || claims.subject().as_str().to_owned(),
        |u| u.as_str().to_owned(),
    );

    let scopes = tokens
        .scopes()
        .map(|s| s.iter().map(|scope| scope.to_string()).collect())
        .unwrap_or_default();
    let admin = crate::session::ViewerSession {
        identity,
        credential: crate::session::Credential::Bearer {
            access_token: tokens.access_token().secret().clone(),
        },
        scopes,
    };
    if let Err(e) = session.insert(crate::session::SESSION_KEY, &admin) {
        return error_page(StatusCode::INTERNAL_SERVER_ERROR, &e.to_string());
    }
    // Re-validated at the point of use: the destination rode a sealed,
    // authenticated cookie, so this cannot fail — and that is exactly why it is
    // cheap to prove rather than assume, since `Redirect::to` panics on a value
    // no `Location` header can carry.
    let next = same_origin_next(Some(&flow.next));
    with_session_cookie(&state, &session, Redirect::to(&next))
}

#[cfg(test)]
mod tests {
    use super::same_origin_next;

    /// The login route's `next` is attacker-supplied by construction — it
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
    }
}
