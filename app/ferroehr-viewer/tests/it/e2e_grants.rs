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
//! End-to-end journey over Roles and grants: the grants scripts/ui-e2e.sh
//! gives its SCIM clients and Basic users, listed for the administrator, and
//! the mark on a policy party's page.

use crate::common;

use common::{Harness, login_basic_as, main_text, wait_main_contains};

#[tokio::test]
async fn admin_lists_the_grants_and_opens_a_marked_policy_party() {
    let Some(h) = Harness::start("grants").await else {
        return;
    };
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    common::follow_nav(&h, "/grants").await;
    wait_main_contains(&h, "Roles holding a grant").await;
    let text = main_text(&h).await;
    for words in [
        "scim_provision",
        "hold_scopes",
        "System agents",
        "basic-users",
    ] {
        assert!(text.contains(words), "the grants say {words:?}: {text}");
    }
    h.shot(1, "grants").await;

    h.wait_xpath(
        "//main//table//a[contains(@href, '/parties/agent/')][normalize-space()='basic-users']",
    )
    .await
    .click()
    .await;
    h.wait_url_contains("/parties/agent/").await;
    wait_main_contains(&h, "Policy party").await;
    assert!(
        main_text(&h).await.contains("scim_provision"),
        "the agent's page shows the grant its role carries"
    );
    h.shot(2, "policy-party").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
