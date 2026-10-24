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
//! End-to-end keyboard journeys (#2793): the viewer driven with a keyboard
//! alone.
//!
//! What a component does with the keyboard (focus rings, roles, roving focus
//! in a tree or a menu) is FerroUI's and is tested there; these journeys cover
//! what the viewer composes from the components: where the focus lands after a
//! route change, the query builder traversed and run by keyboard with Save
//! beside Run, and the user menu's keyboard pattern as the shell mounts it,
//! Your access among its items.

use crate::common;

use common::{Harness, login_basic, poll_until, wait_main_contains};
use thirtyfour::prelude::*;

/// How many Tab presses a journey allows before it calls a control
/// unreachable.
const TAB_BUDGET: usize = 80;

/// After a route change the page starts from the top: the first Tab reaches
/// the skip link, and following it puts the focus in the main area, past the
/// top bar and the sidebar.
#[tokio::test]
async fn a_route_change_starts_the_focus_at_the_skip_link() {
    let Some(h) = Harness::start("keyboard-route-change").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    assert!(
        h.tab_to("main a[href='/ehrs']", TAB_BUDGET).await,
        "the dashboard's EHRs card is reachable by Tab"
    );
    h.press(Key::Enter).await;
    h.wait_url_contains("/ehrs").await;
    h.wait_css("main table").await;

    h.press(Key::Tab).await;
    assert!(
        h.focus_matches("a.fui-skip").await,
        "the first Tab on a new page reaches the skip link"
    );
    h.press(Key::Enter).await;
    h.wait_url_contains("#main").await;
    h.press(Key::Tab).await;
    assert!(
        h.focus_matches("main *").await,
        "after the skip link the next Tab lands in the main area"
    );
    h.shot(1, "focus-in-main").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The query builder by keyboard alone: Tab reaches Add a condition, Enter
/// adds one, the condition's fields take arrow keys, and Run runs the query.
#[tokio::test]
async fn the_query_builder_is_traversed_and_run_by_keyboard() {
    let Some(h) = Harness::start("keyboard-query-builder").await else {
        return;
    };
    login_basic(&h).await;
    crate::e2e_browse::ensure_template_present(&h).await;
    h.goto(&format!(
        "/query?template={}",
        crate::e2e_browse::TEMPLATE_ID
    ))
    .await;

    assert!(
        h.tab_to("button[name='add-condition'][value='g']", TAB_BUDGET)
            .await,
        "Add a condition is reachable by Tab"
    );
    common::submit_by_key(&h, Key::Enter).await;
    h.wait_css("select[name='g.c0-path']").await;
    assert!(
        h.tab_to("select[name='g.c0-op']", TAB_BUDGET).await,
        "the condition's operator is reachable by Tab"
    );
    h.shot(1, "condition-added").await;

    assert!(
        h.tab_to("input[name='g.c0-value']", TAB_BUDGET).await,
        "the condition's value is reachable by Tab"
    );
    h.press("2020-01-01T00:00:00Z").await;

    // Run is in the page header, so Shift+Tab walks back to it.
    let mut reached = false;
    for _ in 0..TAB_BUDGET {
        if h.focus_matches("header.fui-page-header button[form='query-form']")
            .await
        {
            reached = true;
            break;
        }
        h.press(Key::Shift + Key::Tab).await;
    }
    assert!(reached, "Run is reachable by Shift+Tab");
    h.press(Key::Tab).await;
    assert!(
        h.focus_matches("header.fui-page-header button[commandfor='save-query']")
            .await,
        "Save to the CDR follows Run in the header's action group"
    );
    let mut back = false;
    for _ in 0..TAB_BUDGET {
        h.press(Key::Shift + Key::Tab).await;
        if h.focus_matches("header.fui-page-header button[form='query-form']")
            .await
        {
            back = true;
            break;
        }
    }
    assert!(back, "Shift+Tab from Save returns to Run");
    common::submit_by_key(&h, Key::Enter).await;
    wait_main_contains(&h, "2020-01-01T00:00:00Z").await;
    h.wait_css("main table tbody tr").await;
    h.shot(2, "run-by-keyboard").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The user menu as the shell mounts it: Enter opens it with the focus on its
/// first item, Your preferences, Down moves to Your access, Escape closes it
/// and returns the focus to its button, and Enter on Your access opens it.
#[tokio::test]
async fn the_user_menu_answers_the_keyboard() {
    let Some(h) = Harness::start("keyboard-user-menu").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    let button = "button[popovertarget='user-menu']";
    assert!(
        h.tab_to(button, TAB_BUDGET).await,
        "the user menu's button is reachable by Tab"
    );
    h.press(Key::Enter).await;
    h.wait_css("#user-menu:popover-open").await;
    h.wait_css(&format!("{button}[aria-expanded='true']")).await;
    assert!(
        h.focus_matches("#user-menu [role^='menuitem'][href='/preferences']")
            .await,
        "opening the menu puts the focus on its first item, Your preferences"
    );
    h.press(Key::Down).await;
    assert!(
        h.focus_matches("#user-menu [role^='menuitem'][href='/access']")
            .await,
        "Down moves to Your access"
    );
    h.shot(1, "user-menu-by-keyboard").await;
    h.press(Key::Escape).await;
    common::wait_css_absent(&h, "#user-menu:popover-open").await;
    assert!(
        h.focus_matches(button).await,
        "Escape returns the focus to the menu's button"
    );
    h.press(Key::Enter).await;
    h.wait_css("#user-menu:popover-open").await;
    let first = poll_until(async || {
        h.focus_matches("#user-menu [role^='menuitem'][href='/preferences']")
            .await
    })
    .await;
    assert!(first, "the menu opens again on Your preferences");
    h.press(Key::Down).await;
    let access = poll_until(async || {
        h.focus_matches("#user-menu [role^='menuitem'][href='/access']")
            .await
    })
    .await;
    assert!(access, "Down moves to Your access again");
    h.press(Key::Enter).await;
    h.wait_url_contains("/access").await;
    h.wait_css("#scope").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
