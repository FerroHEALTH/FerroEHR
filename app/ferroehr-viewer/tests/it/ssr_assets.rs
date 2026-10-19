// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hydration bundle under `/pkg/` through the REAL service: a file the
//! bundle holds is served whole with no session, and a file it lacks answers
//! `404` itself, never the rendered app and never the sign-in redirect (#3849).
//!
//! `ssr`-gated like its siblings: the service exists on the server build only.

#![cfg(not(target_arch = "wasm32"))]
#![expect(
    clippy::expect_used,
    reason = "every step here is fixture construction — a site directory, a static request, an \
              in-memory body; a fixture that cannot be built is a broken harness, not a test \
              outcome"
)]

use leptos::prelude::LeptosOptions;
use tower::util::ServiceExt;

/// The bytes of the bundle file the fixture site holds.
const WASM: &[u8] = b"\0asm\x01\0\0\0";

/// The name of the bundle file the fixture site holds.
const WASM_NAME: &str = "ferroehr-viewer.FixtureHash000000000000.wasm";

/// A site root holding one bundle file under `pkg/`, unique to `name`.
fn site(name: &str) -> String {
    let root = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("site-{name}"));
    let pkg = root.join("pkg");
    std::fs::create_dir_all(&pkg).expect("the fixture site directory");
    std::fs::write(pkg.join(WASM_NAME), WASM).expect("the fixture bundle file");
    root.to_string_lossy().into_owned()
}

/// The viewer state, its CDR never reached by these requests.
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

/// `GET path` with no session, as a browser's module loader sends it, against
/// a viewer serving the fixture site `name`; the parts and the body.
async fn get(name: &str, path: &str) -> (http::response::Parts, Vec<u8>) {
    // Initializes the reactive executor the way the viewer's own server does.
    drop(leptos_axum::generate_route_list(|| ()));
    let options = LeptosOptions::builder()
        .output_name(env!("CARGO_PKG_NAME"))
        .site_root(site(name))
        .build();
    let service = ferroehr_viewer::server::router(app_state(), options);
    let request = http::Request::builder()
        .uri(path)
        .header(http::header::ACCEPT, "*/*")
        .header(http::header::ACCEPT_ENCODING, "gzip, deflate, br, zstd")
        .body(axum::body::Body::empty())
        .expect("the request fixture should build from static parts");
    let (parts, body) = service
        .oneshot(request)
        .await
        .expect("the service is infallible")
        .into_parts();
    let body = axum::body::to_bytes(body, usize::MAX)
        .await
        .expect("an in-memory body")
        .to_vec();
    (parts, body)
}

#[tokio::test]
async fn a_bundle_file_is_served_whole_with_no_session() {
    let (parts, body) = get("served", &format!("/pkg/{WASM_NAME}")).await;
    assert_eq!(http::StatusCode::OK, parts.status);
    assert_eq!(WASM, body.as_slice(), "the file as it stands");
    assert_eq!(
        Some("application/wasm"),
        parts
            .headers
            .get(http::header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
    );
}

#[tokio::test]
async fn a_file_the_bundle_lacks_answers_404_and_never_the_app_or_the_sign_in() {
    let (parts, body) = get("missing", "/pkg/ferroehr-viewer.NoSuchHash0000000000.wasm").await;
    assert_eq!(http::StatusCode::NOT_FOUND, parts.status);
    assert!(
        parts.headers.get(http::header::LOCATION).is_none(),
        "a missing bundle file is no redirect"
    );
    let text = String::from_utf8_lossy(&body);
    assert!(
        !text.contains("<html") && !text.contains("<!DOCTYPE"),
        "a missing bundle file never renders the app: {text}"
    );
}
