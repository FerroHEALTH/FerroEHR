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
//! End-to-end journey over an EHR's Access settings tab: the administrator
//! reads the settings, commits them again on the person's instruction, and
//! finds the new version in the list.

use crate::common;

use common::{Harness, env, login_basic_as, main_text, wait_main_contains};

#[tokio::test]
async fn admin_reads_and_commits_an_ehrs_access_settings() {
    let Some(h) = Harness::start("ehr-access").await else {
        return;
    };
    let Some(ehr) = env("UI_E2E_SEEDED_EHR_ID") else {
        println!("SKIP ehr-access: UI_E2E_SEEDED_EHR_ID is unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    };
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto(&format!("/ehrs/{ehr}/access")).await;
    wait_main_contains(&h, "Access settings").await;
    let before = common::count_matching(&h, "main a[href*='/access?version=']").await;
    h.shot(1, "access").await;

    h.goto(&format!("/ehrs/{ehr}/access/edit")).await;
    h.wait_css("#document").await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_contains(&format!("/ehrs/{ehr}/access")).await;
    wait_main_contains(&h, "Versions").await;
    let after = common::count_matching(&h, "main a[href*='/access?version=']").await;
    assert!(
        after > before,
        "the commit adds a version: {before} then {after}: {}",
        main_text(&h).await
    );
    h.shot(2, "committed").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
