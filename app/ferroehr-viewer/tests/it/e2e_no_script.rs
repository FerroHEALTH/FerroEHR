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
//! The viewer with its scripts refused: every page the navigation leads to
//! still draws its content, a `GET` form still answers, and a form post
//! still answers with its page (FerroUI `docs/consoles.md`, "Before you write
//! UI code", step 5: every important action is a link or a form).
//!
//! The browser is refused FerroUI's loader and every module, so no script
//! runs; the stylesheets still load. A refused module is the one browser
//! error a page may log here. Read-only towards the CDR.

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

/// Fails the journey if a script ran at `step`: the loader marks the root
/// once it has imported the page's modules.
async fn no_script_ran(h: &Harness, step: &str) {
    assert!(
        common::count_matching(h, "html[data-fui-script]").await == 0,
        "{step} ran a script with the scripts refused"
    );
}

#[tokio::test]
async fn every_page_and_a_post_work_with_the_scripts_refused() {
    let Some(h) = Harness::start_without_scripts("no-script").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/").await;
    h.wait_css("main h1").await;
    no_script_ran(&h, "the dashboard").await;
    for (words, path) in ENTRIES {
        common::follow_nav(&h, path).await;
        h.wait_url_contains(path).await;
        h.wait_css("main h1").await;
        no_script_ran(&h, words).await;
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
    no_script_ran(&h, "the theme post").await;
    h.wait_css("button[popovertarget='user-menu']")
        .await
        .click()
        .await;
    h.wait_css("#user-menu button[name='theme'][value='system']")
        .await
        .click()
        .await;
    h.wait_css("html[data-theme='system']").await;

    h.assert_console_clean(&["/scripts/", "Failed to load resource"])
        .await;
    h.finish().await;
}
