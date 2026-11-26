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
//! End-to-end journey over Archive and Restore: an EHR the journey creates
//! itself, so no seeded record leaves query results, archived from its page
//! and restored (#6655).

use crate::common;

use common::{Harness, env, login_basic_as};

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`).
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// Opens the EHR page's More menu and confirms the dialog `dialog`.
async fn confirm_from_more(h: &Harness, dialog: &str) {
    h.wait_toasts_cleared().await;
    h.wait_css("summary[aria-controls='ehr-more']")
        .await
        .click()
        .await;
    h.wait_css(&format!("button[commandfor='{dialog}']"))
        .await
        .click()
        .await;
    common::wait_dialog_settled(h, &format!("#{dialog} button[type=submit]")).await;
    common::submit(h, &format!("#{dialog} button[type=submit]")).await;
}

/// An EHR is archived from its page, stays readable by id, and is restored.
#[tokio::test]
async fn an_ehr_is_archived_and_restored_from_its_page() {
    let Some(h) = Harness::start("archive-restore").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto("/ehrs/new").await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_contains("/ehrs/").await;
    let url = h.current_url().await;
    let ehr_id = url
        .split("/ehrs/")
        .nth(1)
        .map(|rest| rest.split(['?', '/']).next().unwrap_or(rest).to_owned())
        .filter(|id| id.len() > 8)
        .expect("the create landed on /ehrs/{ehr_id}");

    confirm_from_more(&h, "archive-record").await;
    common::wait_text_contains(&h, ".fui-toast-text", "Archived").await;
    h.wait_url_contains(&ehr_id).await;
    h.shot(1, "ehr-archived").await;

    h.goto(&format!("/ehrs/{ehr_id}")).await;
    confirm_from_more(&h, "restore-record").await;
    common::wait_text_contains(&h, ".fui-toast-text", "Restored from the archive").await;
    h.shot(2, "ehr-restored").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}
