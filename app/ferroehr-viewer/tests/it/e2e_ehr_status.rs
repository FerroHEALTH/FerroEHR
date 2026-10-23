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
// e2e journeys are assertive by design; skip-with-reason prints; the shared
// harness module is per-test-binary (the corpus.rs test-file precedent)
//! End-to-end journeys over the `EHR_STATUS` write path (`PUT
//! /ehr/{ehr_id}/ehr_status` with `If-Match`) and the `VERSIONED_EHR_STATUS`
//! history it produces:
//!
//! - the edit round trip: unticking `is_queryable` on Edit status commits a new
//!   version, the toast says so, the badge goes, and version 2 reads by its
//!   number;
//! - the mid-air collision: another client commits a version while the editor
//!   still holds version 1 as its `If-Match`; the save is refused with the
//!   CDR's `412`, nothing is overwritten, and version 1 still reads.
//!
//! Isolation: each journey creates its OWN EHR over ITS-REST, so neither touches
//! the seeded fixtures the other journeys and the documentation-screenshot pass
//! depend on.

use crate::common;

use common::{Harness, env, login_basic, wait_main_contains, wait_text_contains};

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

/// The Basic credentials the composed stack seeds (the same defaults the shared
/// harness login uses).
fn basic_credentials() -> (String, String) {
    (
        env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned()),
        env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// Create a fresh EHR over ITS-REST and return its `ehr_id`.
///
/// # Panics
/// When the CDR refuses the create (a broken stack, not a skip).
async fn create_ehr(http: &reqwest::Client, v1: &str) -> String {
    let (user, pass) = basic_credentials();
    let body: serde_json::Value = http
        .post(format!("{v1}/ehr"))
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

/// Read the EHR's current `EHR_STATUS` over ITS-REST
/// (`GET /ehr/{ehr_id}/ehr_status`).
///
/// # Panics
/// When the read fails or does not answer JSON.
async fn current_status(http: &reqwest::Client, v1: &str, ehr_id: &str) -> serde_json::Value {
    let (user, pass) = basic_credentials();
    http.get(format!("{v1}/ehr/{ehr_id}/ehr_status"))
        .basic_auth(user, Some(pass))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("read the current EHR_STATUS")
        .json()
        .await
        .expect("EHR_STATUS body")
}

/// Commit a new `EHR_STATUS` version over ITS-REST — the "another client" half
/// of the mid-air-collision journey. `if_match` is the current version's
/// `OBJECT_VERSION_ID`, sent quoted exactly as the spec requires.
///
/// # Panics
/// When the CDR does not accept the update (`200` or `204`).
async fn put_status(
    http: &reqwest::Client,
    v1: &str,
    ehr_id: &str,
    if_match: &str,
    body: &serde_json::Value,
) {
    let (user, pass) = basic_credentials();
    let response = http
        .put(format!("{v1}/ehr/{ehr_id}/ehr_status"))
        .basic_auth(user, Some(pass))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("If-Match", format!("\"{if_match}\""))
        .body(serde_json::to_string(body).expect("serialize the EHR_STATUS"))
        .send()
        .await
        .expect("commit an out-of-band EHR_STATUS version");
    let status = response.status();
    assert!(
        status == http::StatusCode::OK || status == http::StatusCode::NO_CONTENT,
        "the out-of-band EHR_STATUS update must be accepted (got {status})"
    );
}

/// Open Edit status, set `is_queryable` to `queryable`, and save.
async fn save_queryable(h: &Harness, ehr_id: &str, queryable: bool) {
    h.goto(&format!("/ehrs/{ehr_id}/status/edit")).await;
    let field = h.wait_css("#is_queryable").await;
    if field.selected().await != queryable {
        field.click().await;
    }
    h.shot(1, "status-edit").await;
    common::submit(h, "#editor-dock button[value='save']").await;
}

/// The `EHR_STATUS` edit round trip: unticking `is_queryable` commits
/// version 2, the badge goes, and the Versions table opens version 2.
#[tokio::test]
async fn ehr_status_edit_commits_a_version_and_flips_the_badge() {
    let Some(h) = Harness::start("ehr-status-edit").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let ehr_id = create_ehr(&http, &v1).await;

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}/status")).await;
    wait_text_contains(&h, ".fui-subject", "Queryable").await;
    h.wait_css(&format!("main a[href='/ehrs/{ehr_id}/status/edit']"))
        .await
        .click()
        .await;
    assert!(
        h.wait_css("#is_queryable").await.selected().await,
        "the edit form opens with the loaded is_queryable"
    );
    save_queryable(&h, &ehr_id, false).await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}/status")).await;
    h.wait_css(".fui-toast-text").await;
    wait_main_contains(&h, "Not queryable").await;
    let header = h.wait_css(".fui-subject").await.text().await;
    assert!(
        !header.contains("Queryable"),
        "the header drops the Queryable badge: {header}"
    );
    h.shot(2, "status-saved").await;

    h.goto(&format!("/ehrs/{ehr_id}/status?version=2")).await;
    wait_main_contains(&h, "EHR_STATUS, version 2").await;
    h.goto(&format!("/ehrs/{ehr_id}/status?version=1")).await;
    wait_main_contains(&h, "EHR_STATUS, version 1").await;
    h.shot(3, "status-history").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The mid-air collision: a save whose `If-Match` names a superseded version
/// is refused with the CDR's `412`, the editor offers to reload the newer
/// version, and the earlier version still reads by its own number.
#[tokio::test]
async fn a_stale_if_match_is_refused_and_the_old_version_still_reads() {
    let Some(h) = Harness::start("ehr-status-conflict").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let ehr_id = create_ehr(&http, &v1).await;

    login_basic(&h).await;
    // The editor opens on version 1 ...
    h.goto(&format!("/ehrs/{ehr_id}/status/edit")).await;
    h.wait_css("#is_queryable").await;
    // ... and version 2 is committed out of band before it saves.
    let mut status = current_status(&http, &v1, &ehr_id).await;
    let version_one = status["uid"]["value"]
        .as_str()
        .expect("the current EHR_STATUS uid")
        .to_owned();
    status["is_modifiable"] = serde_json::Value::Bool(false);
    put_status(&http, &v1, &ehr_id, &version_one, &status).await;

    h.wait_css("#is_queryable").await.click().await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    wait_main_contains(&h, "412").await;
    wait_main_contains(&h, "Reload version 2").await;
    h.shot(1, "status-conflict").await;

    h.goto(&format!("/ehrs/{ehr_id}/status?version=1")).await;
    wait_main_contains(&h, "EHR_STATUS, version 1").await;
    h.shot(2, "status-version-one").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
