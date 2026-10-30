// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The viewer as the binary serves it: the family stack's health routes
//! answer the `healthcheck` subcommand, and the CDR and issuer clients trust
//! a private CA named by `[cdr.tls]` and `[auth.oidc.tls]` beside the
//! platform's roots, on every platform.
//!
//! `ssr`-gated like its siblings: the service exists on the server build only.

#![cfg(not(target_arch = "wasm32"))]
#![expect(
    clippy::expect_used,
    reason = "every step here is fixture construction: a certificate, a listener, a file; a \
              fixture that cannot be built is a broken harness, not a test outcome"
)]

use std::net::SocketAddr;
use std::path::{Path, PathBuf};

use ferrohealth_health::lifecycle::Lifecycle;
use ferrohealth_health::registry::Registry;
use ferrohealth_server::health::Health;
use ferrohealth_server::healthcheck::Outcome;
use ferrohealth_server::section::{ServerSection, TlsSection};
use ferrohealth_server::tls_client::TlsClientSection;
use rcgen::{BasicConstraints, CertificateParams, IsCa, Issuer, KeyPair};

/// A directory of its own for `name` under the test target's scratch.
fn scratch(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("serving-{name}"));
    std::fs::create_dir_all(&dir).expect("the scratch directory");
    dir
}

/// A test CA and a certificate for `localhost` it signed, written to `dir` as
/// `ca.pem`, `server.pem` and `server.key`.
fn private_pki(dir: &Path) {
    let ca_key = KeyPair::generate().expect("a CA key");
    let mut ca_params = CertificateParams::new(Vec::<String>::new()).expect("CA parameters");
    ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    let ca = ca_params.self_signed(&ca_key).expect("a CA certificate");
    std::fs::write(dir.join("ca.pem"), ca.pem()).expect("the CA file");
    let issuer = Issuer::new(ca_params, ca_key);
    let key = KeyPair::generate().expect("a server key");
    let certificate = CertificateParams::new(vec!["localhost".to_owned()])
        .expect("server parameters")
        .signed_by(&key, &issuer)
        .expect("a server certificate");
    std::fs::write(dir.join("server.pem"), certificate.pem()).expect("the certificate file");
    std::fs::write(dir.join("server.key"), key.serialize_pem()).expect("the key file");
}

/// Serves `app` on a loopback port as the binary does, over the TLS `tls`
/// names when set, with its lifecycle serving; the address.
async fn serve(app: axum::Router, tls: Option<TlsSection>, lifecycle: Lifecycle) -> SocketAddr {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("a loopback port");
    let address = listener.local_addr().expect("the bound address");
    let section = ServerSection {
        listen: address,
        tls,
        ..ServerSection::default()
    };
    tokio::spawn(async move {
        ferrohealth_server::serve::run(
            listener,
            app,
            &section,
            lifecycle,
            std::future::pending::<()>(),
        )
        .await
        .expect("the listener serves until the test ends");
    });
    address
}

/// The viewer's state over its default configuration, its CDR never reached.
fn app_state() -> ferroehr_viewer::state::AppState {
    let config = ferroehr_viewer::config::ViewerConfig::default();
    ferroehr_viewer::state::AppState {
        cdr: ferroehr_viewer::cdr::CdrClient::new(&config.cdr)
            .expect("the default CDR client should build"),
        config: std::sync::Arc::new(config),
        oidc: None,
        session_keys: ferroehr_viewer::session::SessionKeys::from_secret("")
            .expect("an empty secret always yields an ephemeral key"),
    }
}

/// The served viewer answers `healthcheck` healthy, and the drain withdraws
/// readiness so the same probe turns unhealthy.
#[tokio::test]
async fn the_healthcheck_asks_the_readiness_of_the_served_viewer() {
    drop(leptos_axum::generate_route_list(|| ()));
    let options = leptos::prelude::LeptosOptions::builder()
        .output_name(env!("CARGO_PKG_NAME"))
        .build();
    let lifecycle = Lifecycle::default();
    let health = Health::new(Registry::default(), lifecycle.clone());
    let app = ferroehr_viewer::server::app(
        app_state(),
        options,
        health,
        ferrohealth_telemetry::request_log::RequestLog::new(&[]),
    );
    let address = serve(app, None, lifecycle.clone()).await;
    let section = ServerSection {
        listen: address,
        ..ServerSection::default()
    };
    let mut outcome = ferroehr_viewer::cli::healthcheck(&section).await;
    for _ in 0..50 {
        if outcome == Outcome::Healthy {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        outcome = ferroehr_viewer::cli::healthcheck(&section).await;
    }
    assert_eq!(Outcome::Healthy, outcome);
    lifecycle.drain();
    assert_eq!(
        Outcome::Unhealthy(503),
        ferroehr_viewer::cli::healthcheck(&section).await
    );
}

/// A document request without a session for a path no route matches meets
/// the session guard through the family stack, as a routed page does
/// (FerroHEALTH/FerroSYS#199), and health stays open.
#[tokio::test]
async fn an_unknown_address_without_a_session_meets_the_sign_in_through_the_stack() {
    use tower::util::ServiceExt as _;
    drop(leptos_axum::generate_route_list(|| ()));
    let options = leptos::prelude::LeptosOptions::builder()
        .output_name(env!("CARGO_PKG_NAME"))
        .build();
    let app = ferroehr_viewer::server::app(
        app_state(),
        options,
        Health::new(Registry::default(), Lifecycle::default()),
        ferrohealth_telemetry::request_log::RequestLog::new(&[]),
    );
    let get = |path: &str| {
        http::Request::get(path)
            .header(http::header::ACCEPT, "text/html")
            .header(http::header::HOST, "viewer.test")
            .body(axum::body::Body::empty())
            .expect("a request")
    };
    let answer = app
        .clone()
        .oneshot(get("/no-such-screen"))
        .await
        .expect("an answer");
    assert_eq!(http::StatusCode::FOUND, answer.status());
    assert_eq!(
        Some("/login?next=%2Fno-such-screen"),
        answer
            .headers()
            .get(http::header::LOCATION)
            .and_then(|value| value.to_str().ok())
    );
    let answer = app
        .oneshot(get("/health/liveness"))
        .await
        .expect("an answer");
    assert_eq!(http::StatusCode::OK, answer.status());
}

/// A CDR behind a private CA answers the client that names the CA in
/// `[cdr.tls] trust_roots_file`, and refuses the handshake of one that does
/// not.
#[tokio::test]
async fn the_cdr_client_trusts_the_ca_its_tls_table_names() {
    let dir = scratch("cdr");
    private_pki(&dir);
    let status = axum::Router::new().route("/status", axum::routing::get(|| async { "{}" }));
    let address = serve(
        status,
        Some(TlsSection::new(
            dir.join("server.pem"),
            dir.join("server.key"),
        )),
        Lifecycle::default(),
    )
    .await;
    let url = format!("https://localhost:{}/status", address.port());
    let config = |tls: TlsClientSection| ferroehr_viewer::config::CdrConfig {
        base_url: format!("https://localhost:{}", address.port()),
        tls,
        ..ferroehr_viewer::config::CdrConfig::default()
    };

    let trusting = ferroehr_viewer::cdr::CdrClient::new(&config(TlsClientSection {
        trust_roots_file: Some(dir.join("ca.pem")),
        ..TlsClientSection::default()
    }))
    .expect("the client builds over the CA file");
    let answered = trusting
        .get_public(&url, "application/json")
        .await
        .expect("the CA the table names is trusted");
    assert!(answered.is(http::StatusCode::OK));

    let platform_only = ferroehr_viewer::cdr::CdrClient::new(&config(TlsClientSection::default()))
        .expect("the client builds over the platform roots");
    assert!(
        platform_only
            .get_public(&url, "application/json")
            .await
            .is_err(),
        "a private CA is not among the platform's roots"
    );
}

/// The issuer client takes `[auth.oidc.tls]` the same way.
#[tokio::test]
async fn the_issuer_client_trusts_the_ca_its_tls_table_names() {
    let dir = scratch("issuer");
    private_pki(&dir);
    let discovery = axum::Router::new().route(
        "/.well-known/openid-configuration",
        axum::routing::get(|| async { "{}" }),
    );
    let address = serve(
        discovery,
        Some(TlsSection::new(
            dir.join("server.pem"),
            dir.join("server.key"),
        )),
        Lifecycle::default(),
    )
    .await;
    let url = format!(
        "https://localhost:{}/.well-known/openid-configuration",
        address.port()
    );
    let config = |tls: TlsClientSection| ferroehr_viewer::config::OidcConfig {
        tls,
        ..ferroehr_viewer::config::OidcConfig::default()
    };

    let trusting = ferroehr_viewer::oidc::issuer_client(&config(TlsClientSection {
        trust_roots_file: Some(dir.join("ca.pem")),
        ..TlsClientSection::default()
    }))
    .expect("the client builds over the CA file");
    let answered = trusting
        .get(&url)
        .send()
        .await
        .expect("the CA the table names is trusted");
    assert_eq!(http::StatusCode::OK, answered.status());

    let platform_only = ferroehr_viewer::oidc::issuer_client(&config(TlsClientSection::default()))
        .expect("the client builds over the platform roots");
    assert!(
        platform_only.get(&url).send().await.is_err(),
        "a private CA is not among the platform's roots"
    );
}

/// A trust roots file that does not read, or holds no certificate, refuses
/// the client, naming the key.
#[test]
fn an_unreadable_or_empty_roots_file_refuses_the_client() {
    let dir = scratch("refused");
    std::fs::write(dir.join("empty.pem"), "").expect("the empty file");
    for path in [dir.join("absent.pem"), dir.join("empty.pem")] {
        let refused = ferroehr_viewer::cdr::CdrClient::new(&ferroehr_viewer::config::CdrConfig {
            tls: TlsClientSection {
                trust_roots_file: Some(path.clone()),
                ..TlsClientSection::default()
            },
            ..ferroehr_viewer::config::CdrConfig::default()
        });
        let Err(error) = refused else {
            panic!("{} must refuse the CDR client", path.display());
        };
        let chain = ferroehr_viewer::error::error_chain(&error);
        assert!(chain.contains("cdr.tls.trust_roots_file"), "{chain}");

        let refused = ferroehr_viewer::oidc::issuer_client(&ferroehr_viewer::config::OidcConfig {
            tls: TlsClientSection {
                trust_roots_file: Some(path.clone()),
                ..TlsClientSection::default()
            },
            ..ferroehr_viewer::config::OidcConfig::default()
        });
        let Err(error) = refused else {
            panic!("{} must refuse the issuer client", path.display());
        };
        assert!(
            format!("{error:#}").contains("auth.oidc.tls.trust_roots_file"),
            "{error:#}"
        );
    }
}
