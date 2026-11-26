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
//! End-to-end journey over Access policy: the policy the demo seed
//! commits twice, read by section, its versions, an older version opened
//! read-only, and a save over a version that is no longer the newest, which
//! the CDR refuses with `412`.

use crate::common;

use common::{Harness, env, login_basic_as, main_text, wait_main_contains};

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`), which
/// scripts/ui-e2e.sh grants `write_access_policy`.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

#[tokio::test]
async fn admin_reads_the_policy_its_versions_and_a_stale_save_is_refused() {
    let Some(h) = Harness::start("access-policy").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    common::follow_nav(&h, "/access-policy").await;
    wait_main_contains(&h, "Classifications").await;
    let text = main_text(&h).await;
    for words in [
        "FerroEHR Demo Hospital access policy",
        "Mental health notes",
        "ACCESS_POLICY, version 2",
    ] {
        assert!(text.contains(words), "the policy says {words:?}: {text}");
    }
    h.shot(1, "policy").await;

    h.wait_css("a[href='/access-policy/versions']")
        .await
        .click()
        .await;
    h.wait_url_contains("/access-policy/versions").await;
    h.wait_css("main a[href='/access-policy?version=1']")
        .await
        .click()
        .await;
    h.wait_url_contains("version=1").await;
    wait_main_contains(&h, "ACCESS_POLICY, version 1").await;
    assert!(
        !main_text(&h).await.contains("Edit the policy"),
        "an older version opens read-only"
    );
    h.shot(2, "older-version").await;

    h.goto("/access-policy/edit").await;
    let version = h
        .wait_css("input[name='version']")
        .await
        .attr("value")
        .await;
    assert!(
        version.ends_with("::2"),
        "the editor opens the newest: {version}"
    );
    let stale = version.replace("::2", "::1");
    h.run_script(&format!(
        "document.querySelector(\"input[name='version']\").value = {stale:?};"
    ))
    .await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    wait_main_contains(&h, "412 Precondition Failed").await;
    h.shot(3, "stale-save").await;

    h.assert_console_clean(&["412", "Failed to load resource"])
        .await;
    h.finish().await;
}
