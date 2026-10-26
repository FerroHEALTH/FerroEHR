// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! openEHR **ITS-REST 1.1.0** server surface (`axum`) — the protocol adapter
//! over the SM native API (`ferroehr-sm`).
//!
//! The crate is organised **per ITS-REST specification**, one folder per spec area:
//!
//! - [`overview`] — the cross-cutting Overview protocol (content negotiation,
//!   committal headers, resource identification, common params, the HTTP
//!   status/error table); [`overview::error::RestError`] renders the openEHR error body.
//! - [`api`] — the resource APIs, one module per group (`ehr`, `query`,
//!   `definition`, `demographic`, `admin`) implementing the generated
//!   `openehr_its::rest` contract, plus the hand-written `system` API
//!   (`OPTIONS /` conformance manifest). `api::api_router` is the hub over the
//!   generated `ROUTES` tables.
//! - [`formats`] — the Simplified Formats wire (FLAT / STRUCTURED media types).
//! - [`smart`] — SMART App Launch (service discovery + scope enforcement),
//!   config-gated, off by default.
//! - [`system_log`] — the SM System Log component at the wire (the IHE ATNA
//!   audit middleware + operation classification).
//! - [`extensions`] — everything the specs do **not** govern, quarantined and
//!   flagged: authentication + authorization ([`extensions::access`]),
//!   management/observability, `OpenAPI` serving, terminology, eventing, the
//!   ITI-81 audit retrieval, and each config-gated so a stock server exposes
//!   only the standardised ITS-REST surface.
//!
//! [`router::router`] assembles these under the configured base path with the
//! `tower-http` middleware stack. The adapter is generic over the platform
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
mod overload;
pub mod overview;
pub mod rate_limit;
pub mod router;
pub mod smart;
pub mod state;
pub mod system_log;
pub mod tls;

// `access` and `management` are part of the crate's public surface, reached at
// their defining modules because the crate has no re-exports. The two shared
// protocol helpers live under `overview` and are imported here for the
// dispatcher glue below.
use std::sync::Arc;

use ferroehr::service::FerroEhrService;

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
    /// Binding the listener or serving failed.
    #[error("binding or serving the listener failed")]
    Io(#[from] std::io::Error),
    /// The `[server.tls]` material could not be turned into a TLS listener.
    #[error("the TLS listener could not be configured")]
    Tls(#[from] tls::TlsError),
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
    let authenticator = build_authenticator(&config).await?;
    let state = AppState::with_backend(config, backend);
    state.observability().lifecycle.booted();
    Ok(router(state, authenticator))
}

/// Builds and serves the application backed by a concrete service, with graceful
/// shutdown on `SIGINT`/`SIGTERM` and client peer addresses from `ConnectInfo`.
///
/// ATNA auditing is emitted through the platform service's SM `SystemLog`
/// component, whose sender the binary boots, injects and drains on shutdown.
///
/// # Errors
/// [`ServeError::Auth`] on bad auth config; [`ServeError::Tls`] on unusable
/// `[server.tls]` material; [`ServeError::Io`] on bind/serve failure.
pub async fn serve_with(
    config: AppConfig,
    backend: Arc<FerroEhrService>,
) -> Result<(), ServeError> {
    let bind = config.server.bind.clone();
    let tls = config.server.tls.clone();
    let connection = config.server.connection;
    let app = build_with(config, backend).await?;
    run_server(app, &bind, &tls, connection).await
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
    let authenticator = build_authenticator(&config).await?;
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
async fn build_authenticator(config: &AppConfig) -> Result<Arc<Authenticator>, ServeError> {
    let authenticator = Authenticator::new(config).await.map_err(ServeError::Auth)?;
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

/// Builds and serves the application with full observability.
///
/// Mounts the API + audit + telemetry surface, and — when `management.port` is
/// set — the management surface on its own internal listener (otherwise merged
/// into the main app). Graceful shutdown on `SIGINT`/`SIGTERM` covers both
/// listeners. ATNA auditing lives in the backend's SM `SystemLog` component.
///
/// # Errors
/// [`ServeError::Auth`] on bad auth config; [`ServeError::Tls`] on unusable
/// `[server.tls]` material; [`ServeError::Io`] on bind/serve failure.
pub async fn serve_full(
    config: AppConfig,
    backend: Arc<FerroEhrService>,
    authz: Option<Arc<AuthzHandle>>,
    observability: Observability,
) -> Result<(), ServeError> {
    let authenticator = build_authenticator(&config).await?;
    let bind = config.server.bind.clone();
    let tls = config.server.tls.clone();
    let connection = config.server.connection;
    let management_enabled = observability.management.enabled;
    let management_port = observability.management.port;
    let state = AppState::with_parts(config, backend, authz, observability);
    let main_app = router(state.clone(), Arc::clone(&authenticator));

    // The separate-port management listener stays plain HTTP on every interface
    // even with `[server.tls]` on; keeping it off a routed network is the
    // deployment's, and `production` refuses it unless the `plaintext_listener`
    // gap is accepted. The health probes stay on the main listener.
    let management_task = if management_enabled && let Some(port) = management_port {
        let management_app = management_router(&state, authenticator);
        let management_bind = format!("0.0.0.0:{port}");
        tracing::info!(bind = %management_bind, "ferroehr-rest management listening (separate port)");
        Some(tokio::spawn(async move {
            let plain = ferroehr::config::server::TlsConfig::default();
            if let Err(e) = run_server(management_app, &management_bind, &plain, connection).await {
                tracing::error!("management listener stopped: {e}");
            }
        }))
    } else {
        None
    };

    // Readiness withdraws the moment the shutdown signal arrives, before the
    // listener stops accepting.
    let lifecycle = state.observability().lifecycle.clone();
    let drain = tokio::spawn({
        let lifecycle = lifecycle.clone();
        async move {
            shutdown_signal().await;
            lifecycle.drain();
        }
    });
    lifecycle.booted();
    let result = run_server(main_app, &bind, &tls, connection).await;
    drain.abort();
    if let Some(task) = management_task {
        task.abort();
    }
    result
}

/// Serves one router: wraps it in the path-normalization layer, binds, and
/// serves with graceful shutdown and per-connection peer info.
///
/// Plain HTTP, or native TLS with optional client-certificate verification when
/// `[server.tls]` is enabled — the IHE ATNA ITI-19 node-authentication posture,
/// with a TLS 1.3 floor unless `min_version = "1.2"` admits 1.2 alongside.
/// Under TLS, the certificate and key are re-read every
/// `reload_interval_secs` while the listener runs ([`tls::CertificateReloader`]).
async fn run_server(
    app: axum::Router,
    bind: &str,
    tls: &ferroehr::config::server::TlsConfig,
    connection: ferroehr::config::server::ConnectionConfig,
) -> Result<(), ServeError> {
    use std::net::SocketAddr;

    use tower::Layer;
    use tower_http::normalize_path::NormalizePathLayer;

    let app = NormalizePathLayer::trim_trailing_slash().layer(app);
    let make = axum::ServiceExt::<axum::extract::Request>::into_make_service_with_connect_info::<
        SocketAddr,
    >(app);

    if tls.enabled {
        let server_tls = tls::server_tls(tls)?;
        let addr: SocketAddr = bind.parse().map_err(|e| {
            ServeError::Io(std::io::Error::new(std::io::ErrorKind::InvalidInput, e))
        })?;
        let handle = axum_server::Handle::new();
        // Translate the process signals into axum-server's graceful shutdown.
        let signal_handle = handle.clone();
        tokio::spawn(async move {
            shutdown_signal().await;
            signal_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
        });
        tracing::info!(%bind, client_auth = ?tls.client_auth, "ferroehr-rest listening (TLS)");
        let listener_config = axum_server::tls_rustls::RustlsConfig::from_config(server_tls.config);
        let watcher = tls
            .reload_interval()
            .map(|interval| tokio::spawn(Arc::clone(&server_tls.certificate).watch(interval)));
        // A renewed client CA needs a new configuration; the listener swaps it
        // in for every new handshake.
        let ca_watcher = tls.reload_interval().and_then(|interval| {
            server_tls.client_ca.as_ref().map(|reloader| {
                tokio::spawn(Arc::clone(reloader).watch(interval, listener_config.clone()))
            })
        });
        let mut server = axum_server::bind_rustls(addr, listener_config).handle(handle);
        // The connection-level bound, applied where hyper actually enforces it.
        // Everything else in this stack engages after a request head is parsed,
        // so this is the only place a connection that never finishes writing one
        // can be reclaimed.
        bound_connections(server.http_builder(), connection);
        let outcome = server.serve(make).await;
        for task in [watcher, ca_watcher].into_iter().flatten() {
            task.abort();
        }
        outcome?;
        return Ok(());
    }

    // Both listeners run through `axum_server`: it is the only one of the two that
    // exposes hyper's connection builder, and the bound has to apply to the
    // plaintext listener most of all — the one the quickstart publishes.
    let listener = tokio::net::TcpListener::bind(bind).await?;
    let listener = listener.into_std()?;
    tracing::info!(%bind, "ferroehr-rest listening");
    let handle = axum_server::Handle::new();
    let signal_handle = handle.clone();
    tokio::spawn(async move {
        shutdown_signal().await;
        signal_handle.graceful_shutdown(Some(std::time::Duration::from_secs(10)));
    });
    let mut server = axum_server::from_tcp(listener)
        .map_err(ServeError::Io)?
        .acceptor(NoDelayAcceptor)
        .handle(handle);
    bound_connections(server.http_builder(), connection);
    server.serve(make).await?;
    Ok(())
}

/// Applies the connection-level bounds to hyper's builder, for both listeners.
///
/// The builder is `hyper_util`'s AUTO builder, so this server speaks HTTP/1 and
/// HTTP/2 — negotiated by ALPN on the TLS listener, and by prior knowledge or an
/// upgrade on the plaintext one. Configuring the HTTP/1 side does not disable
/// HTTP/2; each protocol needs its own bounds because the exposure differs:
///
/// - **HTTP/1**: a request head arrives as a byte stream, so a peer can trickle
///   it forever. `header_read_timeout` is the bound
///   (<https://docs.rs/hyper/latest/hyper/server/conn/http1/struct.Builder.html>).
/// - **HTTP/2**: the head arrives in HEADERS frames on a multiplexed connection,
///   so there is nothing to trickle — the exposure is stream CONCURRENCY. A peer
///   that opens streams and cancels them immediately makes the server do request
///   setup at almost no cost to itself (CVE-2023-44487, "Rapid Reset"), which
///   `max_concurrent_streams` bounds. The keep-alive PING pair additionally
///   reclaims a connection whose peer vanished without a FIN.
fn bound_connections(
    builder: &mut hyper_util::server::conn::auto::Builder<hyper_util::rt::TokioExecutor>,
    connection: ferroehr::config::server::ConnectionConfig,
) {
    // NOTE: a timeout knob without a timer makes hyper PANIC per connection
    // ("timeout `header_read_timeout` set, but no timer set" —
    // <https://docs.rs/hyper/latest/hyper/server/conn/http1/struct.Builder.html#method.timer>).
    builder
        .http1()
        .timer(hyper_util::rt::TokioTimer::new())
        .header_read_timeout(connection.header_read_timeout());
    builder
        .http2()
        .timer(hyper_util::rt::TokioTimer::new())
        .max_concurrent_streams(connection.stream_cap())
        .keep_alive_interval(connection.http2_keep_alive_interval())
        .keep_alive_timeout(connection.http2_keep_alive_timeout());
}

/// An acceptor that sets `TCP_NODELAY` on every accepted socket and otherwise
/// passes it through.
///
/// The `204` and minimal write acknowledgements this API is full of must not sit
/// in Nagle's buffer waiting for an ACK, and the option is per-connection, so it
/// cannot be set once on the listener. A failed `setsockopt` is logged and the
/// connection served anyway: a latency optimisation must never refuse a request.
#[derive(Debug, Clone, Copy, Default)]
struct NoDelayAcceptor;

impl<S> axum_server::accept::Accept<tokio::net::TcpStream, S> for NoDelayAcceptor {
    type Stream = tokio::net::TcpStream;
    type Service = S;
    type Future = std::future::Ready<std::io::Result<(Self::Stream, Self::Service)>>;

    fn accept(&self, stream: tokio::net::TcpStream, service: S) -> Self::Future {
        if let Err(err) = stream.set_nodelay(true) {
            tracing::debug!(%err, "TCP_NODELAY could not be set on an accepted socket");
        }
        std::future::ready(Ok((stream, service)))
    }
}

/// Resolve when the process receives `SIGINT` (Ctrl-C) or, on Unix, `SIGTERM`.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(err) = tokio::signal::ctrl_c().await {
            tracing::warn!(%err, "SIGINT handler unavailable; relying on SIGTERM");
        }
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => tracing::warn!("failed to install SIGTERM handler: {e}"),
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        () = ctrl_c => {},
        () = terminate => {},
    }
    tracing::info!("shutdown signal received");
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
        let authenticator = build_authenticator(&config).await.expect("authenticator");
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
