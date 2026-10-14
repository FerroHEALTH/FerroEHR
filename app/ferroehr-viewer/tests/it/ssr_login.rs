// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The Basic sign-in server function through the REAL service, against a stub
//! CDR that refuses every credential.
//!
//! A server function transports an error value as a `500`, so a refusal
//! carried as an error reads as a server failure on the wire. What this module
//! pins is that a refused sign-in is not one: posted as a plain form (no
//! JavaScript) it answers the `302` to the login screen's refusal URL, and
//! posted by the hydrated client it answers `200` with the server-function
//! redirect to the same URL — never `500`, and never a session cookie.
//!
//! `ssr`-gated like its siblings: the service and the server function exist on
//! the server build only.

#![cfg(feature = "ssr")]
#![expect(
    clippy::expect_used,
    reason = "every step here is fixture construction — a loopback listener, a static request, \
              an in-memory body; a fixture that cannot be built is a broken harness, not a test \
              outcome"
)]

use leptos::prelude::LeptosOptions;
use tower::util::ServiceExt;

/// Starts a CDR stand-in that answers every request `401` with a Basic
/// challenge, and returns its base URL.
async fn refusing_cdr() -> String {
    let app = axum::Router::new().fallback(|| async {
        (
            http::StatusCode::UNAUTHORIZED,
            [(http::header::WWW_AUTHENTICATE, "Basic realm=\"ferroehr\"")],
        )
    });
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind a loopback port");
    let addr = listener.local_addr().expect("the bound address");
    tokio::spawn(async move {
        axum::serve(listener, app)
            .await
            .expect("the stub CDR serves until the test ends");
    });
    format!("http://{addr}")
}

/// The viewer state, pointed at `cdr`.
fn app_state(cdr: &str) -> ferroehr_viewer::state::AppState {
    let config = ferroehr_viewer::config::ViewerConfig {
        cdr: ferroehr_viewer::config::CdrConfig {
            base_url: cdr.to_owned(),
            ..Default::default()
        },
        ..Default::default()
    };
    ferroehr_viewer::state::AppState {
        cdr: ferroehr_viewer::cdr::CdrClient::new(&config.cdr)
            .expect("the CDR client should build from a well-formed base URL"),
        config: std::sync::Arc::new(config),
        oidc: None,
        session_keys: ferroehr_viewer::session::SessionKeys::from_secret("")
            .expect("an empty secret always yields an ephemeral key"),
    }
}

/// Posts a wrong password to the Basic sign-in server function with `accept`,
/// and returns the response parts.
async fn post_wrong_password(accept: &str) -> http::response::Parts {
    // Initializes the reactive executor the way the viewer's own server does.
    drop(leptos_axum::generate_route_list(|| ()));
    let cdr = refusing_cdr().await;
    let options = LeptosOptions::builder()
        .output_name(env!("CARGO_PKG_NAME"))
        .build();
    let service = ferroehr_viewer::server::router(app_state(&cdr), options);
    let request = http::Request::builder()
        .method(http::Method::POST)
        .uri("/api/login_basic")
        .header(
            http::header::CONTENT_TYPE,
            "application/x-www-form-urlencoded",
        )
        .header(http::header::ACCEPT, accept)
        .header(
            http::header::REFERER,
            "http://viewer.test/login?next=%2Fehrs",
        )
        .body(axum::body::Body::from(
            "username=ferroehr&password=definitely-wrong&next=%2Fehrs",
        ))
        .expect("the request fixture should build from static parts");
    let (parts, _body) = service
        .oneshot(request)
        .await
        .expect("the service is infallible")
        .into_parts();
    parts
}

/// The `Location` a response carries, as text.
fn location(parts: &http::response::Parts) -> Option<&str> {
    parts
        .headers
        .get(http::header::LOCATION)
        .and_then(|value| value.to_str().ok())
}

/// Whether a response sets the viewer's session cookie.
fn sets_session(parts: &http::response::Parts) -> bool {
    parts
        .headers
        .get_all(http::header::SET_COOKIE)
        .iter()
        .any(|value| {
            value
                .to_str()
                .is_ok_and(|text| text.starts_with(ferroehr_viewer::session::SESSION_COOKIE))
        })
}

/// A wrong password posted without JavaScript answers the `302` to the
/// refusal screen, keeping the destination.
#[tokio::test]
async fn a_wrong_password_posted_as_a_form_redirects_to_the_refusal() {
    let parts = post_wrong_password("text/html,application/xhtml+xml").await;
    assert_eq!(parts.status, http::StatusCode::FOUND);
    assert_eq!(location(&parts), Some("/login?refused=1&next=%2Fehrs"));
    assert!(!sets_session(&parts), "{parts:?}");
}

/// A wrong password posted by the hydrated client answers `200` with the
/// server-function redirect, never the `500` an error value travels as.
#[tokio::test]
async fn a_wrong_password_posted_by_the_client_is_not_a_server_error() {
    let parts = post_wrong_password("application/json").await;
    assert_eq!(parts.status, http::StatusCode::OK);
    assert_eq!(location(&parts), Some("/login?refused=1&next=%2Fehrs"));
    assert!(!sets_session(&parts), "{parts:?}");
}
