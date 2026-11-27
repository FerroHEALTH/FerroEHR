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
#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]
//! End-to-end journeys over an EHR's Access log tab: a reviewer reads the
//! entries a fresh EHR's own reads wrote, filters them and opens one; a user
//! the CDR does not admit reads why.
//!
//! Isolation: each journey creates its OWN EHR over ITS-REST.

use crate::common;

use common::{Harness, env, login_basic, login_basic_as, wait_main_contains};

/// The CDR base URL the harness exports for REST-side test setup; `None` skips
/// with a reason.
fn cdr_url() -> Option<String> {
    if let Some(url) = env("UI_E2E_CDR_URL") {
        Some(url)
    } else {
        println!("SKIP: UI_E2E_CDR_URL unset (run scripts/ui-e2e.sh)");
        None
    }
}

/// The administrator the composed stack provisions, a reviewer of the log.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// Create a fresh EHR over ITS-REST as the administrator and return its
/// `ehr_id`.
///
/// # Panics
/// When the CDR refuses the create (a broken stack, not a skip).
async fn create_ehr(cdr: &str) -> String {
    let (user, pass) = admin_credentials();
    let body: serde_json::Value = reqwest::Client::new()
        .post(format!("{cdr}/ferroehr/rest/openehr/v1/ehr"))
        .basic_auth(user, Some(pass))
        .header("Prefer", "return=representation")
        .header("Accept", "application/json")
        .send()
        .await
        .expect("create an EHR")
        .json()
        .await
        .expect("EHR body");
    body.get("ehr_id")
        .and_then(|id| id.get("value"))
        .and_then(serde_json::Value::as_str)
        .expect("the created ehr_id")
        .to_owned()
}

/// A reviewer opens the Access log tab of an EHR whose page they just read,
/// filters it to granted reads and opens an entry.
#[tokio::test]
async fn a_reviewer_reads_filters_and_opens_an_access_log_entry() {
    let Some(h) = Harness::start("access-log-review").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let ehr = create_ehr(&cdr).await;
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    // Reading the EHR's page writes the entries the tab then lists.
    h.goto(&format!("/ehrs/{ehr}")).await;
    h.goto(&format!("/ehrs/{ehr}/access-log")).await;
    wait_main_contains(&h, "Entries of the access log").await;
    h.shot(1, "access-log").await;

    h.wait_css("#l-outcome")
        .await
        .select_by_value("openehr_access|granted")
        .await;
    common::submit(&h, "form.fui-list-toolbar button[type=submit]").await;
    wait_main_contains(&h, "Outcome: Granted").await;
    let url = h.current_url().await;
    assert!(
        url.contains("outcome=openehr_access%7Cgranted"),
        "the filter is in the address: {url}"
    );

    h.wait_css("main a.fui-row-link[href*='/access-log/']")
        .await
        .click()
        .await;
    wait_main_contains(&h, "What it rested on").await;
    wait_main_contains(&h, "As the CDR served it").await;
    h.shot(2, "access-log-entry").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// A user who is neither the person, their representative nor a reviewer
/// reads on the tab who may read the log, as the CDR refused it.
#[tokio::test]
async fn a_user_the_cdr_does_not_admit_reads_who_may() {
    let Some(h) = Harness::start("access-log-refused").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let ehr = create_ehr(&cdr).await;
    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr}/access-log")).await;
    wait_main_contains(&h, "Your account may not read this access log").await;
    h.assert_console_clean(&["401", "403", "Failed to load resource"])
        .await;
    h.finish().await;
}
