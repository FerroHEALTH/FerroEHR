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
//! End-to-end journeys over the operator's two queues: Audit log › Delivery,
//! where the administrator reads the sink's cursors and dismisses a record
//! the audit repository refused, and Server › Identity feed, reached through
//! the server's tabs.
//!
//! The journeys' posture sends the ATNA records to a repository that refuses
//! every one (`docker/viewer/e2e-env.yml`), so the sink parks each record the
//! seed's requests wrote.

use crate::common;

use common::{Harness, login_basic_as, main_text, read_attr, submit, wait_main_contains};

#[tokio::test]
async fn admin_reads_the_delivery_and_dismisses_a_parked_record() {
    let Some(h) = Harness::start("audit-delivery").await else {
        return;
    };
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    common::follow_nav(&h, "/audit").await;
    h.wait_css("main a[href='/audit/delivery']")
        .await
        .click()
        .await;
    wait_main_contains(&h, "Parked records").await;
    let text = main_text(&h).await;
    for words in ["fhir_feed", "Configured", "refused"] {
        assert!(text.contains(words), "the delivery says {words:?}: {text}");
    }
    h.shot(1, "delivery").await;

    let dismiss = "main a[href*='confirm=dismiss']";
    let href = read_attr(&h, dismiss, "href")
        .await
        .expect("a parked record offers its dismissal");
    h.wait_css(dismiss).await.click().await;
    h.wait_url_contains("confirm=dismiss").await;
    h.shot(2, "dismiss-confirm").await;
    submit(
        &h,
        "main form[action='/audit/delivery'] button.fui-button-danger",
    )
    .await;
    h.wait_url_not_contains("confirm=").await;
    wait_main_contains(&h, "Parked records").await;
    assert!(
        !common::is_present(&h, &format!("main a[href='{href}']")).await,
        "the dismissed record leaves the list: {href}"
    );
    h.shot(3, "dismissed").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

#[tokio::test]
async fn admin_opens_the_identity_feed_from_the_server_tabs() {
    let Some(h) = Harness::start("identity-feed").await else {
        return;
    };
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    common::follow_nav(&h, "/server").await;
    common::open_tab(&h, "identity").await;
    h.wait_url_contains("/server/identity").await;
    wait_main_contains(&h, "Refused messages").await;
    let text = main_text(&h).await;
    assert!(
        text.contains("The identity feed has no refused message."),
        "a stack beside no patient identity registry has nothing to retry: {text}"
    );
    h.shot(1, "identity-feed").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
