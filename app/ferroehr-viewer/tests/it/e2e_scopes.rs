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
//! End-to-end journey over the viewer's ACCESS DRAWER ("View scopes"): the
//! effective identity of the session and the SMART scope previewer.
//!
//! What it pins:
//! - the drawer opens from the user menu and states the capability-vs-
//!   authorization caveat (scopes narrow, the CDR enforces);
//! - it names the authenticated principal and the policy source — for the
//!   Basic session the harness logs in with, the existing full-access note;
//! - the free previewer renders a two-scope claim as two parsed grants with the
//!   right compartment/permission chips (the parse is the shared master08
//!   grammar `openehr_sdt::smart_scopes`, running in the browser);
//! - a resource-shaped scope the grammar rejects explains itself instead of
//!   silently reading as nothing.
//!
//! No fixtures: the drawer reads the session the harness already has, and the
//! previewer is a pure function of typed text.

use crate::common;

use common::{Harness, count_matching, find_all, login_basic, poll_until};

/// The previewer's input field.
const PREVIEW_INPUT: &str = "#scope-previewer-input";

/// Every grant card the previewer has rendered.
const PREVIEW_CARDS: &str = "#scope-preview-results [data-scope-grant]";

/// A claim with one patient-compartment composition scope and one user-
/// compartment template scope — the two master08 §Resource Scopes shapes.
const TWO_SCOPES: &str = "patient/composition-*.rs user/template-MyHospital::Template.v0.crud";

/// A resource-shaped scope with a permission letter master08 does not define.
const BAD_PERMISSION: &str = "patient/composition-*.rx";

/// Open the user menu and the access drawer behind it.
async fn open_access_drawer(h: &Harness) {
    h.goto("/").await;
    // The popover open is hydrated behaviour; a click landing before
    // hydration is silently lost (#2285's class).
    h.wait_hydrated().await;
    h.wait_css("#user-menu-trigger button").await.click().await;
    h.wait_css(".thaw-popover-surface").await;
    h.wait_xpath("//button[contains(., 'View scopes')]")
        .await
        .click()
        .await;
    h.wait_css("#access-drawer").await;
}

/// How many grant cards the previewer currently renders.
async fn card_count(h: &Harness) -> usize {
    count_matching(h, PREVIEW_CARDS).await
}

/// Explicit wait (never a sleep) on the SETTLED preview: the rendered grants
/// contain `needle`. Keying text in is character-by-character, and each prefix
/// is a legitimately different claim (`patient/composition-*.r` parses, `…*.rx`
/// does not), so the condition must be the final content — a card COUNT would
/// pass on an intermediate keystroke.
///
/// # Panics
/// When the preview never contains `needle` within 15 s.
async fn wait_cards_contain(h: &Harness, needle: &str) {
    if poll_until(async || cards_text(h).await.contains(needle)).await {
        return;
    }
    panic!(
        "preview never rendered `{needle}` (last: {})",
        cards_text(h).await
    );
}

/// The concatenated text of every rendered grant card.
async fn cards_text(h: &Harness) -> String {
    let cards = find_all(h, PREVIEW_CARDS).await;
    let mut text = String::new();
    for card in cards {
        // NOTE: probe tier — this read runs inside the poll loop that waits
        // for the preview to settle, and each keystroke re-renders the card
        // list, so a detached handle is "not yet" rather than a failure.
        text.push_str(&card.read_text().await.unwrap_or_default());
        text.push('\n');
    }
    text
}

/// The drawer states the identity, the policy source, this session's scopes,
/// and the capability caveat.
#[tokio::test]
async fn access_drawer_shows_effective_identity_and_policy_source() {
    let Some(h) = Harness::start("scopes-identity").await else {
        return;
    };
    login_basic(&h).await;
    open_access_drawer(&h).await;
    h.shot(1, "access-drawer").await;

    // Identity + policy source: the Basic session names the account it replays
    // and keeps the existing full-access note (it carries no SMART scopes).
    let scopes_panel = h.wait_css("#session-scopes").await.text().await;
    assert!(
        scopes_panel.contains("Basic authentication"),
        "the Basic session states its full-access note: {scopes_panel}"
    );

    let drawer = h.wait_css("#access-drawer").await.text().await;
    assert!(
        drawer.contains("ferroehr"),
        "the drawer names the authenticated principal: {drawer}"
    );
    assert!(
        drawer.contains("Basic authentication"),
        "the drawer names the policy source: {drawer}"
    );
    assert!(
        drawer.contains("never grant"),
        "the drawer states capability is not authorization: {drawer}"
    );

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The previewer parses a two-scope claim into two grants with the right
/// compartments and permissions, and explains a scope the grammar rejects.
#[tokio::test]
async fn scope_previewer_renders_grants_and_explains_an_invalid_scope() {
    let Some(h) = Harness::start("scopes-previewer").await else {
        return;
    };
    login_basic(&h).await;
    open_access_drawer(&h).await;

    let input = h.wait_css(PREVIEW_INPUT).await;
    assert_eq!(
        card_count(&h).await,
        0,
        "an empty previewer renders no grants"
    );

    input.send_keys(TWO_SCOPES).await;
    // The second scope, verbatim: only the fully typed claim carries it.
    wait_cards_contain(&h, "user/template-MyHospital::Template.v0.crud").await;
    h.shot(1, "previewer-two-scopes").await;
    assert_eq!(
        card_count(&h).await,
        2,
        "a two-scope claim renders two grants"
    );
    let rendered = cards_text(&h).await;
    for expected in [
        "patient",
        "Compositions",
        "read",
        "search",
        "user",
        "Operational templates",
        "MyHospital::Template.v0",
        "create",
        "delete",
    ] {
        assert!(
            rendered.contains(expected),
            "the parsed grants must show `{expected}`: {rendered}"
        );
    }

    // An invalid permission tail: one card, and it says what master08 expects.
    input.clear().await;
    input.send_keys(BAD_PERMISSION).await;
    // The full malformed scope, verbatim — the settled state.
    wait_cards_contain(&h, BAD_PERMISSION).await;
    h.wait_css("#scope-preview-results [data-scope-grant='unrecognized']")
        .await;
    h.shot(2, "previewer-invalid-scope").await;
    assert_eq!(card_count(&h).await, 1, "one scope in, one grant card out");
    let rejected = cards_text(&h).await;
    assert!(
        rejected.contains("permission"),
        "the rejection explains the permission tail: {rejected}"
    );

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
