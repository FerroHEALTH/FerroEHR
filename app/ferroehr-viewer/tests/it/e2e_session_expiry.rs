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
//! End-to-end journeys over a session that ends mid-visit.
//!
//! The session is ended the way a revocation ends it: the sealed cookie is
//! replaced by one the viewer cannot open, which is observable in seconds,
//! while waiting out the configured idle window is not. Every page is drawn on
//! the server, so the next request is where the viewer notices: it answers
//! with the sign-in page at `/login?expired=1&next=…`, which says the session
//! ended and returns to `next` once signed in again.

use crate::common;

use common::{Harness, env, login_basic, wait_css_absent};

/// The sealed session cookie's name (`ferroehr_viewer::session::SESSION_COOKIE`,
/// which is server-only and so cannot be imported here).
const SESSION_COOKIE: &str = "ferroehr_viewer_session";

/// The signed-in chrome: the user menu's button and the shell's footer.
const AUTHED_CHROME: [&str; 2] = ["button[popovertarget='user-menu']", common::SHELL_FOOTER];

/// Replace the sealed session cookie with one no key opens: a revocation the
/// browser cannot predict, which still sends a session cookie.
async fn revoke_session(h: &Harness) {
    h.set_cookie(SESSION_COOKIE, "revoked").await;
}

/// Assert the signed-out landing: the sign-in card says the session ended,
/// and none of the signed-in chrome is on the page.
async fn assert_signed_out(h: &Harness) {
    h.wait_css("#sign-in-username").await;
    common::wait_main_contains(h, "session ended").await;
    for chrome in AUTHED_CHROME {
        wait_css_absent(h, chrome).await;
    }
}

/// Type the Basic credentials into the sign-in card on screen and submit.
async fn sign_in_on_this_page(h: &Harness) {
    let user = env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned());
    let pass = env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned());
    h.wait_css("#sign-in-username").await.send_keys(&user).await;
    h.wait_css("#sign-in-password").await.send_keys(&pass).await;
    h.wait_css("main button[type=submit]").await.click().await;
}

/// Assert the signed-in chrome is back and the sign-in card is gone.
async fn assert_signed_in_again(h: &Harness) {
    for chrome in AUTHED_CHROME {
        h.wait_css(chrome).await;
    }
    wait_css_absent(h, "#sign-in-username").await;
}

/// A session revoked mid-visit: the next page asked for is the sign-in page
/// that says the session ended, with nothing of the signed-in chrome left.
#[tokio::test]
async fn a_revoked_session_signs_the_ui_out_on_the_next_interaction() {
    let Some(h) = Harness::start("session-expiry-interactive").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/templates").await;
    h.shot(1, "signed-in").await;

    revoke_session(&h).await;
    common::follow_nav(&h, "/ehrs").await;

    h.wait_url_contains("expired=1").await;
    assert_signed_out(&h).await;
    h.shot(2, "signed-out").await;
    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// Signing in from the expired card returns to the screen the session ended
/// on (`next`), with the chrome rendered.
#[tokio::test]
async fn signing_in_from_the_expired_card_returns_to_the_screen_without_a_reload() {
    let Some(h) = Harness::start("session-expiry-sign-in-again").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/templates").await;
    revoke_session(&h).await;
    common::follow_nav(&h, "/ehrs").await;
    h.wait_url_contains("expired=1").await;
    assert!(
        h.current_url().await.contains("next=%2Fehrs"),
        "the sign-in page keeps the screen asked for as `next`"
    );
    assert_signed_out(&h).await;
    h.shot(1, "expired-card").await;
    sign_in_on_this_page(&h).await;
    h.wait_url_contains("/ehrs").await;
    assert_signed_in_again(&h).await;
    h.shot(2, "signed-in-again").await;
    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// Sign out from the user menu, then sign in again: the second sign-in renders
/// the chrome too.
#[tokio::test]
async fn signing_in_again_after_an_explicit_sign_out_renders_the_chrome() {
    let Some(h) = Harness::start("session-expiry-sign-out-sign-in").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/templates").await;
    h.wait_css("button[popovertarget='user-menu']").await.click().await;
    h.wait_css("#user-menu:popover-open").await;
    h.wait_css("#user-menu form[action='/logout'] button")
        .await
        .click()
        .await;
    h.wait_url_contains("/login").await;
    for chrome in AUTHED_CHROME {
        wait_css_absent(&h, chrome).await;
    }
    h.shot(1, "signed-out-explicitly").await;
    sign_in_on_this_page(&h).await;
    assert_signed_in_again(&h).await;
    h.shot(2, "signed-in-again").await;
    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// The expired card on the dashboard: `next=/`, where the shell and its index
/// page share the path, renders the dashboard again once signed in.
#[tokio::test]
async fn signing_in_from_the_expired_card_on_the_dashboard_renders_it_again() {
    let Some(h) = Harness::start("session-expiry-dashboard-sign-in-again").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/ehrs").await;
    revoke_session(&h).await;
    h.wait_css("header a.fui-lockup").await.click().await;
    h.wait_url_contains("expired=1").await;
    assert_signed_out(&h).await;
    h.shot(1, "expired-card").await;
    sign_in_on_this_page(&h).await;
    assert_signed_in_again(&h).await;
    assert!(
        !h.current_url().await.contains("/login"),
        "the address bar left /login"
    );
    h.wait_css("main a.fui-stat").await;
    h.shot(2, "signed-in-again").await;
    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// The OIDC return from the expired card: its OIDC link carries the
/// interrupted screen as `next`, and with Keycloak's own SSO session still
/// alive the browser comes straight back to it signed in. Skips without a
/// Keycloak.
#[tokio::test]
async fn returning_through_oidc_from_the_expired_card_renders_the_chrome() {
    let Some(h) = Harness::start("session-expiry-oidc-return").await else {
        return;
    };
    let (Some(user), Some(pass)) = (env("UI_E2E_OIDC_USER"), env("UI_E2E_OIDC_PASS")) else {
        println!("SKIP oidc return from the expired card: UI_E2E_OIDC_USER/PASS unset");
        h.finish().await;
        return;
    };
    h.goto("/login").await;
    h.wait_css("a[href='/auth/oidc/login']").await.click().await;
    h.wait_url_contains("/auth/realms/ferroehr").await;
    h.wait_css("#username").await.send_keys(&user).await;
    h.wait_css("#password").await.send_keys(&pass).await;
    h.wait_css("#kc-login").await.click().await;
    h.wait_css(common::SHELL_FOOTER).await;
    h.goto("/templates").await;
    revoke_session(&h).await;
    common::follow_nav(&h, "/ehrs").await;
    h.wait_url_contains("expired=1").await;
    assert_signed_out(&h).await;
    h.shot(1, "expired-card").await;
    h.wait_css("a[href^='/auth/oidc/login']")
        .await
        .click()
        .await;
    h.wait_url_contains("/ehrs").await;
    assert_signed_in_again(&h).await;
    h.shot(2, "signed-in-again").await;
    h.assert_console_clean(&[]).await;
    h.finish().await;
}
