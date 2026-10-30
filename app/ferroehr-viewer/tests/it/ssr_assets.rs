// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! FerroUI's assets under `ferrohealth_ui::serve::PREFIX` through the REAL
//! service: a script module the library vendors is served whole with no
//! session, and a file it lacks answers `404` itself, never a page and never
//! the sign-in redirect (#3849).

#![expect(
    clippy::expect_used,
    reason = "every step here is fixture construction — a site directory, a static request, an \
              in-memory body; a fixture that cannot be built is a broken harness, not a test \
              outcome"
)]

use tower::util::ServiceExt;

/// The path of one of FerroUI's script modules, the loader.
fn module_path() -> String {
    format!(
        "{}/{}/scripts/fui.js",
        ferrohealth_ui::serve::PREFIX,
        ferrohealth_ui::serve::VERSION
    )
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

/// `GET path` with no session, as a browser's module loader sends it; the
/// parts and the body.
async fn get(path: &str) -> (http::response::Parts, Vec<u8>) {
    let service = ferroehr_viewer::server::router(app_state());
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
async fn a_script_module_is_served_whole_with_no_session() {
    let (parts, body) = get(&module_path()).await;
    assert_eq!(http::StatusCode::OK, parts.status);
    let loader = ferrohealth_ui::script::LOADER.text;
    assert_eq!(loader.as_bytes(), body.as_slice(), "the module as vendored");
    let kind = parts
        .headers
        .get(http::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(kind.starts_with("text/javascript"), "{kind}");
}

#[tokio::test]
async fn a_file_the_library_lacks_answers_404_and_never_a_page_or_the_sign_in() {
    let (parts, body) = get(&format!(
        "{}/{}/scripts/no-such-module.js",
        ferrohealth_ui::serve::PREFIX,
        ferrohealth_ui::serve::VERSION
    ))
    .await;
    assert_eq!(http::StatusCode::NOT_FOUND, parts.status);
    assert!(
        parts.headers.get(http::header::LOCATION).is_none(),
        "a missing asset is no redirect"
    );
    let text = String::from_utf8_lossy(&body);
    assert!(
        !text.contains("<html") && !text.contains("<!doctype"),
        "a missing asset never renders a page: {text}"
    );
}
