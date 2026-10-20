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
/// # Errors
/// [`ServeError::Auth`] if the OIDC key material/algorithms are invalid.
pub fn build_with(
    config: AppConfig,
    backend: Arc<FerroEhrService>,
) -> Result<axum::Router, ServeError> {
    let authenticator = build_authenticator(&config)?;
    let state = AppState::with_backend(config, backend);
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
    let app = build_with(config, backend)?;
    run_server(app, &bind, &tls, connection).await
}

/// Builds the application router with a concrete backend and a full
/// [`Observability`] bundle.
///
/// The management surface is merged into the returned router when it is enabled
/// and not bound to a separate port.
///
/// # Errors
/// [`ServeError::Auth`] if the OIDC key material/algorithms are invalid.
pub fn build_full(
    config: AppConfig,
    backend: Arc<FerroEhrService>,
    authz: Option<Arc<AuthzHandle>>,
    observability: Observability,
) -> Result<axum::Router, ServeError> {
    let authenticator = build_authenticator(&config)?;
    let state = AppState::with_parts(config, backend, authz, observability);
    Ok(router(state, authenticator))
}

/// Builds the [`Authenticator`] with the configured role-claim paths.
///
/// The paths are `[authz.rbac] role_claims` as lifted into
/// [`AppConfig::role_claims`], whether or not an authorization gate is wired,
/// so Bearer role extraction never falls back to the defaults behind the
/// operator's back.
///
/// # Errors
/// [`ServeError::Auth`] if the OIDC key material/algorithms are invalid.
fn build_authenticator(config: &AppConfig) -> Result<Arc<Authenticator>, ServeError> {
    Authenticator::with_role_claims(config.auth.clone(), config.role_claims.clone())
        .map_err(ServeError::Auth)
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
    let authenticator = build_authenticator(&config)?;
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

    let result = run_server(main_app, &bind, &tls, connection).await;
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
mod tests {
    use super::*;
    use ferroehr::config::auth::{AuthConfig, OidcConfig};
    use jsonwebtoken::{Algorithm, EncodingKey, Header, encode};

    const SECRET: &str = "role-claims-secret";

    /// The claim set the test token carries, roles under a custom path.
    #[derive(serde::Serialize)]
    struct Claims {
        sub: &'static str,
        iss: &'static str,
        aud: &'static str,
        exp: i64,
        app_roles: [&'static str; 1],
    }

    /// The configured `[authz.rbac] role_claims` reach the authenticator with
    /// no authorization handle wired (RBAC, ABAC and the per-EHR default all
    /// off), so a role under a custom claim path is extracted.
    #[tokio::test]
    async fn configured_role_claims_apply_without_an_authz_handle() {
        let config = AppConfig {
            auth: AuthConfig {
                enabled: true,
                basic: None,
                oidc: Some(OidcConfig {
                    issuer: "https://issuer.example".to_owned(),
                    audiences: vec!["ferroehr".to_owned()],
                    algorithms: vec!["HS256".to_owned()],
                    hmac_secret: Some(ferroehr::config::secret::Secret::new(SECRET.to_owned())),
                    jwks_json: None,
                    ..OidcConfig::default()
                }),
                ..AuthConfig::default()
            },
            role_claims: vec!["app_roles".to_owned()],
            ..AppConfig::default()
        };
        let authenticator = build_authenticator(&config).expect("authenticator");
        let exp = jiff::Timestamp::now().as_second() + 3600;
        let token = encode(
            &Header::new(Algorithm::HS256),
            &Claims {
                sub: "alice",
                iss: "https://issuer.example",
                aud: "ferroehr",
                exp,
                app_roles: ["CLINICIAN"],
            },
            &EncodingKey::from_secret(SECRET.as_bytes()),
        )
        .expect("encode");
        let mut headers = http::HeaderMap::new();
        headers.insert(
            http::header::AUTHORIZATION,
            http::HeaderValue::from_str(&format!("Bearer {token}")).expect("header"),
        );
        let authenticated = authenticator.authenticate(&headers).await.expect("valid");
        assert_eq!(authenticated.principal.roles, vec!["CLINICIAN".to_owned()]);
    }
}
