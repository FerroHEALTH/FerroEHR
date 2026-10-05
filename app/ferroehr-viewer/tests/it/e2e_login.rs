// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

#![allow(
    clippy::panic,
    clippy::expect_used,
    reason = "a browser journey asserts by panicking, and the shared harness panics when a configured stack cannot be driven"
)]
#![allow(
    clippy::print_stdout,
    reason = "the skip-with-reason and progress lines ARE this suite's report"
)]
#![allow(
    unreachable_pub,
    dead_code,
    reason = "the shared `common` harness is compiled into every journey binary; each one drives a different subset of it"
)]
// e2e journeys are assertive by design; skip-with-reason prints; the shared
// harness module is per-test-binary (the corpus.rs test-file precedent)
//! End-to-end login and access-control journeys, driven by
//! `scripts/ui-e2e.sh`; each test skips with a printed reason when the
//! harness environment is absent.

use crate::common;

use common::{Harness, env, login_basic};

/// The Basic form logs in and lands on the dashboard with the
/// authenticated chrome; wrong credentials render an inline error and do
/// NOT create a session.
#[tokio::test]
async fn login_basic_authenticates_and_rejects_bad_credentials() {
    let Some(h) = Harness::start("login-basic").await else {
        return;
    };
    // Wrong password first: error surface, still on /login.
    h.goto("/login").await;
    h.shot(1, "login-form").await;
    h.wait_css("#login-username")
        .await
        .send_keys("ferroehr")
        .await;
    h.wait_css("#login-password")
        .await
        .send_keys("definitely-wrong")
        .await;
    h.wait_css("button[type=submit]").await.click().await;
    h.wait_css(".thaw-message-bar").await;
    h.shot(2, "login-error").await;
    assert!(
        h.current_url().await.contains("/login"),
        "wrong credentials must stay on /login"
    );

    // Right credentials: dashboard + chrome.
    login_basic(&h).await;
    h.shot(3, "dashboard-after-login").await;
    let footer = h.wait_css("footer").await.text().await;
    assert!(!footer.is_empty(), "authenticated footer renders");
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The OIDC button starts the Keycloak redirect flow; after authenticating
/// at Keycloak the browser lands back on the viewer with a session.
#[tokio::test]
async fn login_oidc_round_trips_through_keycloak() {
    let Some(h) = Harness::start("login-oidc").await else {
        return;
    };
    let (Some(user), Some(pass)) = (env("UI_E2E_OIDC_USER"), env("UI_E2E_OIDC_PASS")) else {
        println!("SKIP login_oidc: UI_E2E_OIDC_USER/PASS unset");
        h.finish().await;
        return;
    };
    h.goto("/login").await;
    h.wait_css("a[href='/auth/oidc/login']").await.click().await;
    // Keycloak's login form.
    h.wait_url_contains("/auth/realms/ferroehr").await;
    h.shot(1, "keycloak-form").await;
    h.wait_css("#username").await.send_keys(&user).await;
    h.wait_css("#password").await.send_keys(&pass).await;
    h.wait_css("#kc-login").await.click().await;
    // Back on the viewer, authenticated.
    h.wait_css("footer").await;
    h.shot(2, "viewer-after-oidc").await;
    let url = h.current_url().await;
    assert!(
        !url.contains("/login"),
        "OIDC flow must land on the viewer, not back at /login (got {url})"
    );
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Hydration proof: after first paint an interaction actually mutates the
/// DOM (WASM attached), and the browser console log carries no hydration error.
#[tokio::test]
async fn hydration_attaches_interactivity() {
    let Some(h) = Harness::start("hydration").await else {
        return;
    };
    login_basic(&h).await;
    h.shot(1, "shell-before-toggle").await;
    // The user-menu popover opens on click — DOM state only WASM can flip.
    let user_button = h.wait_css("#user-menu-trigger button").await;
    user_button.click().await;
    h.wait_css(".thaw-popover-surface").await;
    h.shot(2, "user-menu-open").await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// An unauthenticated deep link redirects to `/login`; the login page
/// renders (the streaming-SSR client redirect completes).
#[tokio::test]
async fn unauthenticated_deep_link_redirects_to_login() {
    let Some(h) = Harness::start("auth-redirect").await else {
        return;
    };
    h.goto("/system").await;
    h.wait_url_contains("/login").await;
    h.wait_css("#login-username").await;
    h.shot(1, "redirected-to-login").await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
