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
//! End-to-end journey over Retention: the administrator declares a period,
//! finds it in the register, and reads what is due.

use crate::common;

use common::{Harness, fill, login_basic_as, main_text, submit, wait_main_contains};

#[tokio::test]
async fn admin_declares_a_period_and_reads_what_is_due() {
    let Some(h) = Harness::start("retention").await else {
        return;
    };
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    common::follow_nav(&h, "/retention").await;
    wait_main_contains(&h, "Declared periods").await;
    fill(&h, "#ret-jurisdiction", "CH").await;
    fill(&h, "#ret-period", "20 years").await;
    fill(&h, "#ret-source", "E2E journey period").await;
    h.shot(1, "declare").await;
    submit(&h, "button[name='intent'][value='declare']").await;
    wait_main_contains(&h, "E2E journey period").await;
    assert!(
        main_text(&h).await.contains("What is due"),
        "the page lists what is due"
    );
    h.shot(2, "declared").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
