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
//! End-to-end journeys over Server's admin tabs: an activity report of the
//! seeded EHR service, and a verification of the stored data (#6656).

use crate::common;

use common::{Harness, login_basic_as, main_text, submit, wait_main_contains};

/// Activity counts the EHR service's history, its contribution ids listed;
/// Integrity verifies the whole store and reports what it checked.
#[tokio::test]
async fn server_counts_activity_and_verifies_integrity() {
    let Some(h) = Harness::start("server-admin").await else {
        return;
    };
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto("/server/activity").await;
    h.wait_css("main select[name='a_service']")
        .await
        .select_by_value("EHR")
        .await;
    submit(
        &h,
        "main form[action='/server/activity'] button[type=submit]",
    )
    .await;
    h.wait_url_contains("a_service=EHR").await;
    wait_main_contains(&h, "Composition versions").await;
    let report = main_text(&h).await;
    assert!(
        report.contains("Contribution ids"),
        "the seeded EHR service lists its contributions: {report}"
    );
    h.shot(1, "activity").await;

    h.goto("/server/integrity").await;
    submit(&h, "main button[name='intent'][value='verify']").await;
    wait_main_contains(&h, "Versions checked").await;
    h.shot(2, "integrity").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}
