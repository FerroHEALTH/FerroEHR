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
//! Navigation as one document a page (FerroUI #510): a followed link and a
//! `GET` form each load their page whole, the browser's back and forward
//! restore the pages, a preference the user menu posts serves the page it
//! returns to in that preference, a tree's level folds in place with its
//! folds kept across a reload, and every page reflows at 320 px with the
//! scripts loaded and refused.
//!
//! A journey marks the window before it acts and asks for the mark after: a
//! document the browser loaded anew has lost it, and a change a module made
//! in place keeps it. Read-only towards the CDR.

use crate::common;
use common::{Harness, login_basic, wait_main_contains};

/// Fails the journey unless `step` loaded a new document.
async fn loaded(h: &Harness, step: &str) {
    assert!(!h.document_kept().await, "{step} drew in the old document");
}

#[tokio::test]
async fn a_link_loads_its_page_whole_and_back_and_forward_restore_the_pages() {
    let Some(h) = Harness::start("navigation-links").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    h.mark_document().await;
    common::follow_nav(&h, "/ehrs").await;
    h.wait_url_contains("/ehrs").await;
    h.wait_css("main h1").await;
    loaded(&h, "following EHRs").await;
    h.wait_scripts().await;
    h.mark_document().await;
    common::follow_nav(&h, "/about").await;
    h.wait_url_contains("/about").await;
    wait_main_contains(&h, "Manufacturer").await;
    assert!(h.title().await.starts_with("About"), "{}", h.title().await);
    loaded(&h, "following About").await;
    h.back().await;
    h.wait_url_contains("/ehrs").await;
    h.wait_xpath(
        "//nav[@aria-label = 'Viewer']//a[starts-with(normalize-space(.), 'EHRs') and @aria-current = 'page']",
    )
    .await;
    h.forward().await;
    h.wait_url_contains("/about").await;
    wait_main_contains(&h, "Manufacturer").await;
    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

#[tokio::test]
async fn a_get_form_loads_its_answer_whole() {
    let Some(h) = Harness::start("navigation-get-form").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/server/api").await;
    wait_main_contains(&h, "/ehr/{ehr_id}/composition").await;
    h.mark_document().await;
    h.wait_css("#api-f").await.select_by_value("query").await;
    common::submit(&h, "form[action='/server/api'] button[type=submit]").await;
    h.wait_url_contains("family=query").await;
    wait_main_contains(&h, "/query/aql").await;
    loaded(&h, "the family picker's GET form").await;
    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

#[tokio::test]
async fn a_theme_the_user_menu_posts_serves_the_page_it_returns_to() {
    let Some(h) = Harness::start("navigation-theme").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    h.wait_css("button[popovertarget='user-menu']")
        .await
        .click()
        .await;
    h.wait_css("#user-menu:popover-open").await;
    h.wait_css("#user-menu button[name='theme'][value='dark']")
        .await
        .click()
        .await;
    h.wait_css("html[data-theme='dark']").await;
    // The cookie it kept serves the next document in the same theme.
    h.goto("/about").await;
    h.wait_css("html[data-theme='dark']").await;
    h.wait_css("button[popovertarget='user-menu']")
        .await
        .click()
        .await;
    h.wait_css("#user-menu button[name='theme'][value='system']")
        .await
        .click()
        .await;
    h.wait_css("html[data-theme='system']").await;
    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

/// The path tree of the seeded template's Paths tab.
const PATHS_TREE: &str = "template-paths";

/// The root item of [`PATHS_TREE`], the template's composition.
const ROOT: &str = "#template-paths li[role='treeitem'][data-key='n']";

/// Whether the root item of [`PATHS_TREE`] is open.
async fn root_open(h: &Harness) -> bool {
    h.wait_css(ROOT).await.attr("aria-expanded").await == "true"
}

#[tokio::test]
async fn a_tree_level_folds_in_place_and_its_folds_survive_a_reload() {
    let Some(h) = Harness::start("navigation-tree-folds").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/templates/minimal_evaluation.en.v1?tab=paths")
        .await;
    h.wait_css(&format!("#{PATHS_TREE}")).await;
    assert!(root_open(&h).await, "the root arrives open");
    h.mark_document().await;
    h.wait_css(&format!("{ROOT} > .fui-tree-row > .fui-tree-toggle"))
        .await
        .click()
        .await;
    assert!(
        common::poll_until(async || !root_open(&h).await).await,
        "the root closes"
    );
    assert!(h.document_kept().await, "the fold loaded no document");
    h.goto("/templates/minimal_evaluation.en.v1?tab=paths")
        .await;
    assert!(
        common::poll_until(async || !root_open(&h).await).await,
        "the reload keeps the root closed"
    );
    h.mark_document().await;
    h.wait_css(&format!("{ROOT} > .fui-tree-row > .fui-tree-toggle"))
        .await
        .click()
        .await;
    assert!(
        common::poll_until(async || root_open(&h).await).await,
        "the root opens in place"
    );
    assert!(h.document_kept().await, "the open loaded no document");
    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

/// The pages the reflow journey lays out at 320 px: every navigation entry a
/// composed stack draws, the dashboard, a template, a query and the reader's
/// own pages.
const REFLOWED: [&str; 14] = [
    "/",
    "/ehrs",
    "/parties",
    "/templates",
    "/templates/minimal_evaluation.en.v1",
    "/terminology",
    "/queries",
    "/query",
    "/server",
    "/configuration",
    "/about",
    "/access",
    "/preferences",
    "/find?q=minimal",
];

/// Lays every page of [`REFLOWED`] out at 320 CSS pixels and fails the
/// journey on one that scrolls sideways (WCAG 2.2 SC 1.4.10 Reflow).
async fn every_page_reflows(h: &Harness) {
    h.emulate(320, false).await;
    for path in REFLOWED {
        h.goto(path).await;
        h.wait_css("main h1").await;
        let overflow = h.sideways_overflow().await;
        assert!(
            overflow <= 0.5,
            "{path} scrolls sideways at 320 px by {overflow} px"
        );
    }
}

#[tokio::test]
async fn every_page_reflows_at_320_pixels_with_the_scripts_loaded() {
    let Some(h) = Harness::start("navigation-reflow").await else {
        return;
    };
    login_basic(&h).await;
    every_page_reflows(&h).await;
    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

#[tokio::test]
async fn every_page_reflows_at_320_pixels_with_the_scripts_refused() {
    let Some(h) = Harness::start_without_scripts("navigation-reflow-no-script").await else {
        return;
    };
    login_basic(&h).await;
    every_page_reflows(&h).await;
    h.assert_console_clean(&["/scripts/", "Failed to load resource"])
        .await;
    h.finish().await;
}
