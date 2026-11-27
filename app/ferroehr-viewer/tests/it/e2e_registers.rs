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
//! End-to-end journey over the admin registers: the access delay read with
//! its history back to version 0, saved as the next version with the same
//! document the register holds, and the new version listed.

use crate::common;

use common::{Harness, login_basic_as, main_text, wait_main_contains};

#[tokio::test]
async fn admin_reads_the_access_delay_its_history_and_saves_it() {
    let Some(h) = Harness::start("registers").await else {
        return;
    };
    let (user, pass) = common::admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    common::follow_nav(&h, "/access-policy").await;
    h.wait_css("a[href='/access-policy/registers/access-delay']")
        .await
        .click()
        .await;
    wait_main_contains(&h, "The default, which no one committed").await;
    let before = main_text(&h).await;
    assert!(
        before.contains("Enabled"),
        "the register is drawn by its members: {before}"
    );
    h.shot(1, "access-delay").await;

    h.goto("/access-policy/registers/access-delay/edit").await;
    let opened = h
        .wait_css("input[name='version']")
        .await
        .attr("value")
        .await;
    let next = opened.parse::<i64>().expect("a version number") + 1;
    common::submit(&h, "#editor-dock button[value='save']").await;
    wait_main_contains(&h, &format!("{next} (shown)")).await;
    h.shot(2, "saved").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
