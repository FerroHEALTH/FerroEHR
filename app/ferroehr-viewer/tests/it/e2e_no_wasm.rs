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
//! The viewer with its WebAssembly withheld: every page the navigation leads
//! to still draws its content, and a form post still answers with its page
//! (FerroUI `docs/consoles.md`, "How a console builds": every page works
//! without the WebAssembly as the fallback).
//!
//! The browser is refused the bundle's WebAssembly, its glue and its chunks,
//! so nothing hydrates; the stylesheets still load. A refused bundle file is
//! the one browser error a page may log here. Read-only towards the CDR.

use crate::common;

use common::{Harness, login_basic, wait_main_contains};

/// Navigation entries every composed stack draws: their words and the page
/// each leads to.
const ENTRIES: [(&str, &str); 9] = [
    ("EHRs", "/ehrs"),
    ("Parties", "/parties"),
    ("Templates", "/templates"),
    ("Terminology", "/terminology"),
    ("Stored queries", "/queries"),
    ("Query", "/query"),
    ("Server", "/server"),
    ("Configuration", "/configuration"),
    ("About", "/about"),
];

/// Fails the journey if the page hydrated at `step`.
async fn not_hydrated(h: &Harness, step: &str) {
    assert!(
        common::count_matching(h, "html[data-ferroui-hydrated]").await == 0,
        "{step} hydrated with its WebAssembly withheld"
    );
}

#[tokio::test]
async fn every_page_and_a_post_work_with_the_webassembly_withheld() {
    let Some(h) = Harness::start_without_webassembly("no-webassembly").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    h.wait_css("main h1").await;
    not_hydrated(&h, "the dashboard").await;
    for (words, path) in ENTRIES {
        common::follow_nav(&h, path).await;
        h.wait_url_contains(path).await;
        h.wait_css("main h1").await;
        not_hydrated(&h, words).await;
    }

    // A GET form submits as the browser does.
    common::follow_nav(&h, "/server").await;
    h.wait_url_contains("/server").await;
    h.wait_xpath("//a[normalize-space(.) = 'API']")
        .await
        .click()
        .await;
    h.wait_url_contains("/server/api").await;
    h.wait_css("#api-f").await.select_by_value("query").await;
    common::submit(&h, "form[action='/server/api'] button[type=submit]").await;
    h.wait_url_contains("family=query").await;
    wait_main_contains(&h, "/query/aql").await;

    // A post answers with its page: the theme the user menu posts.
    h.wait_css("button[popovertarget='user-menu']")
        .await
        .click()
        .await;
    h.wait_css("#user-menu button[name='theme'][value='dark']")
        .await
        .click()
        .await;
    h.wait_css("html[data-theme='dark']").await;
    not_hydrated(&h, "the theme post").await;
    h.wait_css("button[popovertarget='user-menu']")
        .await
        .click()
        .await;
    h.wait_css("#user-menu button[name='theme'][value='system']")
        .await
        .click()
        .await;
    h.wait_css("html[data-theme='system']").await;

    h.assert_console_clean(&["/pkg/", "Failed to load resource"])
        .await;
    h.finish().await;
}
