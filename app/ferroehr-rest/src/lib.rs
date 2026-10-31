// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR **ITS-REST 1.1.0** server surface (`axum`) — the protocol adapter
//! over the service layer (`ferroehr::service`).
//!
//! The crate is organised **per ITS-REST specification**, one folder per spec area:
//!
//! - [`overview`] — the cross-cutting Overview protocol (content negotiation,
//!   committal headers, resource identification, common params, the HTTP
//!   status/error table); [`overview::error::RestError`] renders RFC 9457 problem details.
//! - [`api`] — the resource APIs, one module per group (`ehr`, `query`,
//!   `definition`, `demographic`, `admin`) implementing the generated
//!   `openehr_its::rest` contract, plus the hand-written `system` API
//!   (`OPTIONS /` conformance manifest). `api::api_router` is the hub over the
//!   generated `ROUTES` tables.
//! - [`formats`] — the Simplified Formats wire (FLAT / STRUCTURED media types).
//! - [`smart`] — SMART App Launch (service discovery + scope enforcement),
//!   config-gated, off by default.
//! - [`system_log`] — the audit log at the wire (the IHE ATNA
//!   audit middleware + operation classification).
//! - [`extensions`] — everything the specs do **not** govern, quarantined and
//!   flagged: authentication + authorization ([`extensions::access`]),
//!   management/observability, `OpenAPI` serving, terminology, eventing, the
//!   ITI-81 audit retrieval, and each config-gated so a stock server exposes
//!   only the standardised ITS-REST surface.
//!
//! [`router::router`] assembles these under the configured base path inside
//! the family stack of `ferrohealth-server`, and [`serve_full`] serves them
//! until the process is asked to stop, then drains. The adapter is generic over the platform
//! concrete `FerroEhrService` (no trait seam, no stub backend) — the
//! `ferroehr` crate monomorphizes it over its DB-backed `FerroEhrService` via
//! [`AppState::with_backend`], and the tests over a mock.
//!
//! Authentication is HTTP Basic plus OAuth2/OIDC bearer, applied as one
//! middleware over the API router, with the coarse RBAC gate and the ABAC PEP
//! composing on top when wired ([`extensions::access`]). Auth is out of band per
//! `overview/Requests_and_responses.md` §Authentication.

// Doctests are copy-paste templates: they must use `?`, never unwrap
// (C-QUESTION-MARK, https://rust-lang.github.io/api-guidelines/documentation.html#c-question-mark).
#![doc(test(attr(deny(warnings))))]
pub mod api;
pub mod config;
pub mod extensions;
pub mod formats;
mod limits;
pub mod overview;
pub mod router;
pub mod smart;
pub mod state;
pub mod system_log;

// `access` and `management` are part of the crate's public surface, reached at
// their defining modules because the crate has no re-exports. The two shared
// protocol helpers live under `overview` and are imported here for the
// dispatcher glue below.
use std::future::Future;
use std::sync::Arc;

use ferroehr::service::FerroEhrService;
use ferrohealth_server::serve::{Drained, Listener};
use tokio::net::TcpListener;

use crate::config::AppConfig;
use crate::extensions::access::authn::Authenticator;
use crate::extensions::access::authz::AuthzHandle;
use crate::extensions::management::Observability;
use crate::router::{management_router, router};
use crate::state::AppState;
use overview::{negotiate, params};

/// Errors raised while starting the server.
#[derive(Debug, thiserror::Error)]
pub enum ServeError {
    /// The authentication configuration was invalid.
    #[error("authentication configuration error: {0}")]
    Auth(String),
    /// Binding a listener failed.
    #[error("binding the listener failed")]
    Io(#[from] std::io::Error),
    /// Serving failed: the `[server.tls]` files could not be read, or a
    /// listener failed.
    #[error(transparent)]
    Serve(#[from] ferrohealth_server::serve::ServeError),
}

/// Build the application router backed by a concrete service (the `ferroehr`
/// application injects its DB-backed service here).
///
/// The router is ready to serve, so its lifecycle reads serving.
///
/// # Errors
/// [`ServeError::Auth`] if `[auth]` is refused or an issuer cannot be read.
pub async fn build_with(
    config: AppConfig,
    backend: Arc<FerroEhrService>,
) -> Result<axum::Router, ServeError> {
    let authenticator = build_authenticator(&config, None).await?;
    let state = AppState::with_backend(config, backend);
    state.observability().lifecycle.booted();
    Ok(router(state, authenticator))
}

/// Builds the application router with a concrete backend and a full
/// [`Observability`] bundle.
///
/// The management surface is merged into the returned router when it is enabled
/// and not bound to a separate port. The router is ready to serve, so the
/// bundle's lifecycle reads serving.
///
/// # Errors
/// [`ServeError::Auth`] if `[auth]` is refused or an issuer cannot be read.
pub async fn build_full(
    config: AppConfig,
    backend: Arc<FerroEhrService>,
    authz: Option<Arc<AuthzHandle>>,
    observability: Observability,
) -> Result<axum::Router, ServeError> {
    let authenticator = build_authenticator(&config, authz.as_deref()).await?;
    let state = AppState::with_parts(config, backend, authz, observability);
    state.observability().lifecycle.booted();
    Ok(router(state, authenticator))
}

/// Builds the [`Authenticator`] of `config`, reading each `[auth]` issuer's
/// metadata and key set.
///
/// The role-claim paths are `[auth] role_claims`, which `ferrohealth-smart`
/// reads whether or not an authorization gate is wired. A served SMART discovery document needs the issuer's
/// metadata to name a `token_endpoint`, which the document requires (SMART App
/// Launch 2.2.0, Conformance, §Metadata), so a `jwks_file` issuer refuses it
/// unless `[smart] issuer_metadata_file` supplies that metadata.
///
/// # Errors
/// [`ServeError::Auth`] if `[auth]` is refused or an issuer cannot be read.
async fn build_authenticator(
    config: &AppConfig,
    authz: Option<&AuthzHandle>,
) -> Result<Arc<Authenticator>, ServeError> {
    let claim = authz
        .and_then(AuthzHandle::abac)
        .and_then(|abac| abac.patient_claim.as_deref());
    let authenticator = Authenticator::new(config, claim)
        .await
        .map_err(ServeError::Auth)?;
    if config.smart.enabled
        && config.auth.enabled
        && authenticator
            .smart_configuration(&smart::discovery::options(config))
            .is_none_or(|document| document.get("token_endpoint").is_none())
    {
        return Err(ServeError::Auth(
            "smart.enabled = true needs an [auth] issuer whose metadata names a \
             token_endpoint (SMART App Launch 2.2.0, Conformance, §Metadata); an issuer \
             read from a jwks_file carries none unless smart.issuer_metadata_file supplies it"
                .to_owned(),
        ));
    }
    Ok(authenticator)
}

/// Builds and serves the application with full observability until the
/// process receives `SIGTERM` or `SIGINT`, then drains.
///
/// Binds `server.listen` and, when `management.port` is set, the management
/// surface on its own internal listener (otherwise merged into the main app),
/// and hands both to [`serve_on`]. ATNA auditing lives in the backend's
/// `SystemLog` component.
///
/// # Errors
/// [`ServeError::Auth`] on bad auth config; [`ServeError::Io`] when a listener
/// cannot bind; [`ServeError::Serve`] on unusable `[server.tls]` material or a
/// failed listener.
pub async fn serve_full(
    config: AppConfig,
    backend: Arc<FerroEhrService>,
    authz: Option<Arc<AuthzHandle>>,
    observability: Observability,
) -> Result<Drained, ServeError> {
    let listener = TcpListener::bind(config.server.listen).await?;
    let management = match observability.management.port {
        Some(port) if observability.management.enabled => {
            Some(TcpListener::bind(std::net::SocketAddr::from(([0, 0, 0, 0], port))).await?)
        }
        _ => None,
    };
    serve_on(
        listener,
        management,
        config,
        backend,
        authz,
        observability,
        ferrohealth_server::serve::shutdown_signal(),
    )
    .await
}

/// Serves the application on `listener` and, when one is given, the management
/// surface on `management`, until `signal` completes, then drains both.
///
/// Readiness turns `503` at once, the listeners keep accepting for
/// `server.drain_delay_ms`, then close, and the requests in flight get
/// `server.shutdown_timeout_ms` to finish; a connection still open then is
/// dropped and the result is [`Drained::Cut`]. The main listener speaks the TLS
/// `[server.tls]` configures; the management listener stays plain HTTP, and
/// keeping it off a routed network is the deployment's (`production` refuses it
/// unless the `plaintext_listener` gap is accepted).
///
/// # Errors
/// [`ServeError::Auth`] on bad auth config; [`ServeError::Serve`] on unusable
/// `[server.tls]` material or a failed listener.
pub async fn serve_on<F>(
    listener: TcpListener,
    management: Option<TcpListener>,
    config: AppConfig,
    backend: Arc<FerroEhrService>,
    authz: Option<Arc<AuthzHandle>>,
    observability: Observability,
    signal: F,
) -> Result<Drained, ServeError>
where
    F: Future<Output = ()> + Send + 'static,
{
    let authenticator = build_authenticator(&config, authz.as_deref()).await?;
    let section = config.server.clone();
    let state = AppState::with_parts(config, backend, authz, observability);
    let lifecycle = state.observability().lifecycle.clone();
    let main = Listener::new(listener, router(state.clone(), Arc::clone(&authenticator)));
    let main = match &section.tls {
        Some(tls) => main.with_tls(tls.clone()),
        None => main,
    };
    tracing::info!(
        listen = %section.listen,
        tls = section.tls.is_some(),
        "ferroehr-rest listening"
    );
    let mut listeners = vec![main];
    if let Some(management) = management {
        if let Ok(address) = management.local_addr() {
            tracing::info!(listen = %address, "ferroehr-rest management listening (separate port)");
        }
        listeners.push(Listener::new(
            management,
            management_router(&state, authenticator),
        ));
    }
    Ok(ferrohealth_server::serve::run_all(listeners, &section, lifecycle, signal).await?)
}

#[cfg(test)]
pub(crate) mod test_support {
    //! The committed CNF test issuer (`docs/conformance/party/smart/`) as the
    //! key the unit tests mint RS256 access tokens with.

    #![expect(
        clippy::disallowed_types,
        reason = "a test builds a token's claim set directly; the claim set is RFC 7519-open"
    )]

    use std::path::PathBuf;
    use std::sync::OnceLock;

    use ferrohealth_smart::config::{AuthSection, IssuerKey};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

    /// The issuer the test tokens name.
    pub(crate) const ISSUER: &str = "https://issuer.example";
    /// The audience the test tokens name.
    pub(crate) const AUDIENCE: &str = "ferroehr";
    /// The `kid` of the committed test key.
    const KID: &str = "cnf-smart-test";

    /// The public key set, written once to a file the resource server reads.
    pub(crate) fn jwks_file() -> PathBuf {
        static FILE: OnceLock<PathBuf> = OnceLock::new();
        FILE.get_or_init(|| {
            let path = std::env::temp_dir().join(format!(
                "ferroehr-rest-unit-jwks-{}.json",
                std::process::id()
            ));
            std::fs::write(
                &path,
                include_str!("../../../docs/conformance/party/smart/jwks.json"),
            )
            .expect("write the test key set");
            path
        })
        .clone()
    }

    /// An `[auth]` section trusting the test issuer from its key-set file.
    pub(crate) fn bearer_auth() -> AuthSection {
        AuthSection {
            issuer: IssuerKey::Flat(ISSUER.to_owned()),
            audience: vec![AUDIENCE.to_owned()],
            jwks_file: jwks_file().display().to_string(),
            role_claims: keycloak_role_claims(),
            ..ferroehr::config::FerroEhrConfig::default().auth
        }
    }

    /// The role claims a Keycloak deployment configures: the RFC 9068
    /// §2.2.3.1 default and Keycloak's nested `realm_access.roles`.
    pub(crate) fn keycloak_role_claims() -> Vec<String> {
        let mut claims = ferrohealth_smart::roles::default_claims();
        claims.push("realm_access.roles".to_owned());
        claims
    }

    /// An RFC 9068 access token carrying `claims`, signed with the test key.
    pub(crate) fn token(claims: &serde_json::Value) -> String {
        let mut header = Header::new(Algorithm::RS256);
        header.typ = Some("at+jwt".to_owned());
        header.kid = Some(KID.to_owned());
        let key = EncodingKey::from_rsa_pem(include_bytes!(
            "../../../docs/conformance/party/smart/cnf-smart-test.key.pem"
        ))
        .expect("the committed test key");
        encode(&header, claims, &key).expect("encode")
    }

    /// The claims of a valid access token for `sub` granting `scope`.
    pub(crate) fn claims(sub: &str, scope: &str) -> serde_json::Value {
        let now = jiff::Timestamp::now().as_second();
        serde_json::json!({
            "iss": ISSUER,
            "aud": AUDIENCE,
            "sub": sub,
            "client_id": "unit-test-client",
            "scope": scope,
            "iat": now,
            "exp": now + 3600,
            "jti": format!("{sub}-{now}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The configured `[auth] role_claims` reach the authenticator with
    /// no authorization handle wired (RBAC, ABAC and the per-EHR default all
    /// off), so a role under a custom claim path is extracted.
    #[tokio::test]
    async fn configured_role_claims_apply_without_an_authz_handle() {
        let config = AppConfig {
            auth: ferrohealth_smart::config::AuthSection {
                role_claims: vec!["app_roles".to_owned()],
                ..test_support::bearer_auth()
            },
            ..AppConfig::default()
        };
        let authenticator = build_authenticator(&config, None)
            .await
            .expect("authenticator");
        let mut claims = test_support::claims("alice", "user/composition-*.r");
        claims["app_roles"] = serde_json::json!(["CLINICIAN"]);
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {}", test_support::token(&claims)))
                .expect("header"),
        );
        let authenticated = authenticator.authenticate(&headers).await.expect("valid");
        assert_eq!(authenticated.principal.roles, vec!["CLINICIAN".to_owned()]);
    }
}
