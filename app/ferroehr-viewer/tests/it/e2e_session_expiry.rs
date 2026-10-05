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
//! End-to-end journeys over the END of a viewer session.
//!
//! The session is ended the way a revocation ends it — the sealed cookie is
//! deleted out from under the browser — because that is observable in seconds,
//! while waiting out the configured idle window is not. What the two journeys
//! pin is the behaviour on either side of the trigger:
//!
//! - with interaction, the first server function that answers "no session"
//!   drives the whole UI to the signed-out state (the transport backstop);
//! - with NO interaction at all, the same state is reached on the viewer's own
//!   schedule — which is what proves there is no interactive window where the
//!   authenticated chrome accepts input against a dead session.
//!
//! Both end on `/login?expired=1`, with the session-ended notice rendered and
//! no authenticated chrome left on the page.

use crate::common;

use std::time::Duration;

use common::{Harness, dwell, env, login_basic, wait_css_absent};

/// The sealed session cookie's name (`ferroehr_viewer::session::SESSION_COOKIE`,
/// which is server-only and so cannot be imported here).
const SESSION_COOKIE: &str = "ferroehr_viewer_session";

/// How long a journey waits for a transition the browser makes on its own
/// schedule: the shell re-checks the session on the health poll's tick, so the
/// budget is that interval plus room for the round trip.
const UNATTENDED_WAIT: Duration = Duration::from_mins(1);

/// The authenticated chrome, by the selectors the other journeys drive it with.
const AUTHED_CHROME: [&str; 2] = ["#user-menu-trigger", "footer"];

/// Deliberate console noise: every server function called with the cookie gone
/// answers an error status, which the browser logs as a failed load. Nothing
/// else is allowed through the gate.
const EXPECTED_CONSOLE: [&str; 1] = ["Failed to load resource"];

/// Delete the sealed session cookie: a revocation the browser cannot predict.
async fn revoke_session(h: &Harness) {
    h.delete_cookie(SESSION_COOKIE).await;
}

/// Assert the signed-out landing: the expiry notice is on screen and none of
/// the authenticated chrome survived the transition.
async fn assert_signed_out(h: &Harness) {
    h.wait_css("#session-expired").await;
    for chrome in AUTHED_CHROME {
        wait_css_absent(h, chrome).await;
    }
}

/// A session revoked mid-visit: the next interaction with the CDR lands the
/// whole UI on the signed-out state, with no reload and nothing left to click.
#[tokio::test]
async fn a_revoked_session_signs_the_ui_out_on_the_next_interaction() {
    let Some(h) = Harness::start("session-expiry-interactive").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/templates").await;
    h.shot(1, "signed-in").await;

    revoke_session(&h).await;
    // A sidebar navigation — the ordinary click a user makes without knowing
    // anything has changed. The screen it opens reads the CDR, that read is
    // refused, and the refusal is what the transition rides.
    h.wait_css("a[href='/ehrs']").await.click().await;

    h.wait_url_contains("expired=1").await;
    assert_signed_out(&h).await;
    h.shot(2, "signed-out").await;
    h.assert_console_clean(&EXPECTED_CONSOLE).await;
    h.finish().await;
}

/// A session revoked while the user does NOTHING: the viewer notices on its own
/// and signs out, so no interactive window over a dead session ever exists.
#[tokio::test]
async fn a_revoked_session_signs_the_ui_out_with_no_interaction_at_all() {
    let Some(h) = Harness::start("session-expiry-unattended").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    h.shot(1, "signed-in").await;

    revoke_session(&h).await;
    // Nothing is clicked, typed or navigated from here on.
    h.wait_url_contains_for("expired=1", UNATTENDED_WAIT).await;
    assert_signed_out(&h).await;
    h.shot(2, "signed-out").await;
    h.assert_console_clean(&EXPECTED_CONSOLE).await;
    h.finish().await;
}

// ── signing in AGAIN inside the same WASM runtime (#3066) ────────────────────

/// Type the Basic credentials into the login card ALREADY on screen and
/// submit. Deliberately no `goto`: a reload would start a fresh runtime and
/// mask any state the first session left behind, which is the defect under
/// test.
async fn sign_in_on_this_page(h: &Harness) {
    let user = env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned());
    let pass = env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned());
    h.wait_css("#login-username").await.send_keys(&user).await;
    h.wait_css("#login-password").await.send_keys(&pass).await;
    h.wait_css("button[type=submit]").await.click().await;
}

/// Assert the authenticated chrome is back on screen and the login card is
/// gone, without any reload having happened.
async fn assert_signed_in_again(h: &Harness) {
    for chrome in AUTHED_CHROME {
        h.wait_css(chrome).await;
    }
    wait_css_absent(h, "#login-username").await;
}

/// The signed-out transition lands on `/login?expired=1&next=…`; signing in
/// from THAT card returns to `next` with the chrome rendered, in the same
/// runtime, without a reload.
#[tokio::test]
async fn signing_in_from_the_expired_card_returns_to_the_screen_without_a_reload() {
    let Some(h) = Harness::start("session-expiry-sign-in-again").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/templates").await;
    revoke_session(&h).await;
    h.wait_css("a[href='/ehrs']").await.click().await;
    h.wait_url_contains("expired=1").await;
    assert_signed_out(&h).await;
    h.shot(1, "expired-card").await;
    sign_in_on_this_page(&h).await;
    // `next` is the screen the refusal interrupted.
    h.wait_url_contains("/ehrs").await;
    assert_signed_in_again(&h).await;
    h.shot(2, "signed-in-again").await;
    h.assert_console_clean(&EXPECTED_CONSOLE).await;
    h.finish().await;
}

/// The explicit Sign out → sign in path shares the machinery: the second
/// sign-in in the same runtime must render the chrome too.
#[tokio::test]
async fn signing_in_again_after_an_explicit_sign_out_renders_the_chrome() {
    let Some(h) = Harness::start("session-expiry-sign-out-sign-in").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/templates").await;
    h.wait_css("#user-menu-trigger button").await.click().await;
    h.wait_css(".thaw-popover-surface").await;
    h.wait_xpath("//button[normalize-space()='Sign out']")
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
    h.assert_console_clean(&EXPECTED_CONSOLE).await;
    h.finish().await;
}

/// The unattended expiry on the dashboard: the shell notices on its own poll,
/// lands on `/login?expired=1&next=%2F`, and signing in from that card, after
/// the page has sat there a moment, renders the dashboard chrome again — the
/// `next=/` case, where the parent route and its index child share the path.
#[tokio::test]
async fn signing_in_from_the_unattended_expired_card_renders_the_dashboard_again() {
    let Some(h) = Harness::start("session-expiry-unattended-sign-in-again").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    revoke_session(&h).await;
    h.wait_url_contains_for("expired=1", UNATTENDED_WAIT).await;
    assert_signed_out(&h).await;
    h.shot(1, "expired-card").await;
    // The card sits for a moment first: any timer the first shell left behind
    // gets its chance to fire before the second sign-in. An absence over a
    // window has no observable to poll for, so this is the harness's one
    // deliberate timed wait rather than a condition.
    dwell(
        Duration::from_secs(8),
        "let a leftover session timer fire before signing in again",
    )
    .await;
    sign_in_on_this_page(&h).await;
    assert_signed_in_again(&h).await;
    assert!(
        !h.current_url().await.contains("/login"),
        "the address bar left /login"
    );
    h.shot(2, "signed-in-again").await;
    h.assert_console_clean(&EXPECTED_CONSOLE).await;
    h.finish().await;
}

/// The OIDC return from the expired card. The card's OIDC anchor carries the
/// interrupted screen as `next` and is a top-level redirect through the BFF,
/// so the browser leaves the WASM runtime entirely; with Keycloak's own SSO
/// session still alive it comes straight back to `next` with the chrome
/// rendered. Recorded because it is the other half of #3066's question, not
/// because it shares the client-side machinery. Skips without a Keycloak.
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
    h.wait_css("footer").await;
    h.goto("/templates").await;
    revoke_session(&h).await;
    h.wait_css("a[href='/ehrs']").await.click().await;
    h.wait_url_contains("expired=1").await;
    assert_signed_out(&h).await;
    h.shot(1, "expired-card").await;
    // The anchor now carries `?next=%2Fehrs`; Keycloak still holds its SSO
    // session, so no second credential prompt is expected.
    h.wait_css("a[href^='/auth/oidc/login']")
        .await
        .click()
        .await;
    h.wait_url_contains("/ehrs").await;
    assert_signed_in_again(&h).await;
    h.shot(2, "signed-in-again").await;
    h.assert_console_clean(&EXPECTED_CONSOLE).await;
    h.finish().await;
}
