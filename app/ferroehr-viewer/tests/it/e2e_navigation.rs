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
//! Navigation in the page: once the viewer has hydrated, a followed link, a
//! `GET` form and a preference the user menu posts each draw their page in
//! the same document, and the browser's back and forward walk the pages it
//! drew.
//!
//! Each journey marks the window before it acts and asks for the mark after:
//! a document the browser loaded anew has lost it. Read-only towards the CDR.

use crate::common;

use common::{Harness, login_basic, wait_main_contains};

/// The navigation entry whose words are `words`.
fn entry(words: &str) -> String {
    format!("//nav[@aria-label = 'Viewer']//a[starts-with(normalize-space(.), '{words}')]")
}

/// Fails the journey unless the window kept its mark after `step`.
async fn kept(h: &Harness, step: &str) {
    assert!(h.document_kept().await, "{step} loaded a new document");
}

#[tokio::test]
async fn a_link_draws_its_page_in_the_same_document_and_back_and_forward_walk_it() {
    let Some(h) = Harness::start("navigation-links").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    h.mark_document().await;

    h.wait_xpath(&entry("EHRs")).await.click().await;
    h.wait_url_contains("/ehrs").await;
    h.wait_css("main h1").await;
    kept(&h, "following EHRs").await;

    h.wait_xpath(&entry("About")).await.click().await;
    h.wait_url_contains("/about").await;
    wait_main_contains(&h, "Manufacturer").await;
    assert!(h.title().await.starts_with("About"), "{}", h.title().await);
    kept(&h, "following About").await;

    h.back().await;
    h.wait_url_contains("/ehrs").await;
    h.wait_xpath(
        "//nav[@aria-label = 'Viewer']//a[starts-with(normalize-space(.), 'EHRs') and @aria-current = 'page']",
    )
    .await;
    kept(&h, "going back").await;

    h.forward().await;
    h.wait_url_contains("/about").await;
    wait_main_contains(&h, "Manufacturer").await;
    kept(&h, "going forward").await;
    h.assert_console_clean(&["Failed to load resource"]).await;

    h.goto("/ehrs").await;
    assert!(
        !h.document_kept().await,
        "a document loaded anew still carries the mark"
    );
    h.finish().await;
}

#[tokio::test]
async fn a_get_form_draws_its_answer_in_the_same_document() {
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
    kept(&h, "the family picker's GET form").await;
    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

#[tokio::test]
async fn a_theme_the_user_menu_posts_takes_effect_in_the_same_document() {
    let Some(h) = Harness::start("navigation-theme").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    h.mark_document().await;

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
    kept(&h, "the theme post").await;

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
