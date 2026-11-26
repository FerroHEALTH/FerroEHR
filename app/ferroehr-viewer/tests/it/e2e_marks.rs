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
//! End-to-end journey over an EHR's Legal marks tab: the administrator
//! records the person's objection to research processing behind its
//! confirmation, reads it back, and withdraws it.

use crate::common;

use common::{Harness, env, login_basic_as, main_text, submit, wait_main_contains};

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`).
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

#[tokio::test]
async fn admin_records_and_withdraws_a_research_objection() {
    let Some(h) = Harness::start("legal-marks").await else {
        return;
    };
    let Some(ehr) = env("UI_E2E_SEEDED_EHR_ID") else {
        println!("SKIP legal-marks: UI_E2E_SEEDED_EHR_ID is unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto(&format!("/ehrs/{ehr}/marks")).await;
    wait_main_contains(&h, "Objection to research processing").await;
    h.shot(1, "marks").await;

    h.wait_css("button[commandfor='mark-object-confirm']")
        .await
        .click()
        .await;
    common::wait_visible(&h, "#mark-object-confirm").await;
    submit(&h, "#mark-object-confirm button[value='object']").await;
    wait_main_contains(&h, "The objection stands").await;
    h.shot(2, "objected").await;

    h.wait_css("button[commandfor='mark-withdraw-confirm']")
        .await
        .click()
        .await;
    common::wait_visible(&h, "#mark-withdraw-confirm").await;
    submit(&h, "#mark-withdraw-confirm button[value='withdraw']").await;
    wait_main_contains(&h, "No objection").await;
    assert!(
        !main_text(&h).await.contains("The objection stands"),
        "the withdrawal ends the objection"
    );

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
