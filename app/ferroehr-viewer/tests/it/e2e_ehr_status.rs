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
//! End-to-end journeys over the viewer's **`EHR_STATUS` write path** — the
//! openEHR operation every client has (`PUT /ehr/{ehr_id}/ehr_status` with
//! `If-Match`), plus the `VERSIONED_EHR_STATUS` history it produces:
//!
//! - **the edit round trip**: unticking `is_queryable` on the Status tab commits
//!   a new version, the viewer reports the commit, the capability badge flips,
//!   and the Status-history tab lists a second version whose document opens by
//!   its own `OBJECT_VERSION_ID`;
//! - **the mid-air collision**: after that edit, another client commits a third
//!   version over REST while the screen still holds version 2 as its `If-Match`;
//!   saving again must be REFUSED with the CDR's `412` surfaced as actionable
//!   copy (nothing silently overwritten), and version 1 must still read by its
//!   own uid afterwards.
//!
//! Isolation: each journey creates its OWN EHR over ITS-REST, so neither touches
//! the seeded fixtures the other journeys and the documentation-screenshot pass
//! depend on.

use crate::common;

use std::time::Duration;

use common::{
    Element, Harness, click_until_css, env, is_present_by, login_basic, poll_until, poll_until_for,
    wait_enabled, wait_text_suffix,
};
use thirtyfour::prelude::*;

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

/// Drive the checkbox at `css` to `desired`, clicking only when it differs, and
/// report whether it ends up there.
///
/// A checkbox flips natively on click, so its state proves nothing about
/// hydration — which is why the caller reaches hydration first
/// ([`wait_checkbox`]) and then waits for the mutation's own outcome
/// ([`save_queryable`]).
async fn set_checkbox(h: &Harness, css: &str, desired: bool) -> bool {
    for _ in 0..10 {
        let field = h.wait_css(css).await;
        if checkbox_state(&field).await == desired {
            return true;
        }
        field.click().await;
        if poll_until_for(Duration::from_millis(200), async || {
            checkbox_state(&h.wait_css(css).await).await == desired
        })
        .await
        {
            return true;
        }
    }
    false
}

/// The live `checked` property of a checkbox element.
///
/// # Panics
/// When the property cannot be read (a dead session, not a state).
async fn checkbox_state(field: &Element) -> bool {
    field.prop("checked").await == "true"
}

/// Wait until the checkbox at `css` reports `expected`; returns whether it did.
///
/// This is also the journey's HYDRATION signal for the edit form: the
/// server-rendered checkbox carries no `checked` attribute (the viewer drives it
/// with `prop:checked`, which is applied when the WASM bundle hydrates), so the
/// box reporting the value the CDR served proves the form is live. Interacting
/// before that would flip the DOM checkbox natively while the viewer's own state
/// stayed untouched — and the save would then commit the value the operator
/// thought they had changed.
async fn wait_checkbox(h: &Harness, css: &str, expected: bool) -> bool {
    poll_until(async || checkbox_state(&h.wait_css(css).await).await == expected).await
}

/// Set `is_queryable` to `desired` and save, retrying until the viewer reports
/// the commit; returns whether it did.
///
/// The toggle is an `on:change` listener and the save an `on:click` one, so an
/// interaction landing before hydration is simply lost — the bounded retry is the
/// login-submit precedent. A retry after a save that DID land is harmless: the
/// form re-seeds from the freshly committed version, and the checkbox is only
/// clicked when it does not already hold the wanted value.
///
/// The success toast is the mutation's OWN outcome and is awaited here, before
/// the journey does anything else — a navigation would abort the in-flight
/// server-fn call (the hardening the admin-ops journeys established).
async fn save_queryable(h: &Harness, desired: bool) -> bool {
    for _ in 0..5 {
        // A visible toast overlays the bottom-right corner and intercepts clicks.
        h.wait_toasts_cleared().await;
        if !set_checkbox(h, "#status-queryable", desired).await {
            continue;
        }
        h.wait_css("#status-save").await.click().await;
        if poll_until_for(Duration::from_secs(10), async || {
            is_present_by(
                h,
                By::XPath("//*[contains(normalize-space(.), 'EHR status updated')]"),
            )
            .await
        })
        .await
        {
            return true;
        }
    }
    false
}

/// The `EHR_STATUS` edit round trip: unticking `is_queryable` commits a new
/// version, the badge flips, and the Status-history tab lists version 2 and
/// opens its document by `OBJECT_VERSION_ID`.
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
    h.goto(&format!("/ehrs/{ehr_id}?tab=status")).await;
    // A fresh EHR is queryable; the badge proves the current-status read landed
    // and the edit form was seeded from it.
    h.wait_css("[data-status-flag='queryable'][data-status-value='true']")
        .await;
    h.wait_css("#status-edit").await;
    // Every control stays disabled until the loaded document has been seeded
    // into the form, so nothing entered here can be lost to the seed.
    wait_enabled(&h, "#status-queryable").await;
    wait_enabled(&h, "#status-other-details").await;
    wait_enabled(&h, "#status-save").await;
    // The form is live once the checkbox reports the SERVED value (see
    // `wait_checkbox`) — only then does clicking it change what the save sends.
    assert!(
        wait_checkbox(&h, "#status-queryable", true).await,
        "the edit form never hydrated with the loaded is_queryable"
    );
    h.shot(1, "status-before-edit").await;

    assert!(
        save_queryable(&h, false).await,
        "unticking is_queryable and saving never reported a committed version"
    );
    h.shot(2, "status-saved").await;
    // The refetched facts card reports the NEW state — the write really landed.
    h.wait_css("[data-status-flag='queryable'][data-status-value='false']")
        .await;
    // …and the screen now warns that AQL cannot see this EHR.
    h.wait_xpath("//*[contains(., 'not queryable')]").await;

    // The history tab: the versioned family, not the current-status endpoint.
    h.wait_toasts_cleared().await;
    h.goto(&format!("/ehrs/{ehr_id}?tab=status-history")).await;
    h.wait_css("[data-versioned-fact='version']").await;
    // A SECOND version exists: version 1 came with the EHR, version 2 is the
    // edit above.
    h.wait_css("[data-status-version$='::2']").await;
    assert!(
        click_until_css(
            &h,
            "[data-status-version$='::2']",
            "#status-version-document",
        )
        .await,
        "opening version 2 never rendered its document"
    );
    // The pinned VERSION's envelope facts follow the selection.
    wait_text_suffix(&h, "[data-versioned-fact='version']", "::2").await;
    h.shot(3, "status-history").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The mid-air collision: a save whose `If-Match` names a superseded version is
/// refused with the CDR's `412`, the viewer says so with the next action, and
/// the earlier version still reads by its own uid.
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
    h.goto(&format!("/ehrs/{ehr_id}?tab=status")).await;
    h.wait_css("[data-status-flag='queryable'][data-status-value='true']")
        .await;
    // The whole card is disabled until the loaded document is seeded into it.
    wait_enabled(&h, "#status-queryable").await;
    assert!(
        wait_checkbox(&h, "#status-queryable", true).await,
        "the edit form never hydrated with the loaded is_queryable"
    );
    assert!(
        save_queryable(&h, false).await,
        "the first edit never reported a committed version"
    );
    h.wait_css("[data-status-flag='queryable'][data-status-value='false']")
        .await;

    // The viewer's screen now holds version 2 as its If-Match. Another client
    // commits version 3 on top of it.
    let mut status = current_status(&http, &v1, &ehr_id).await;
    let version_two = status["uid"]["value"]
        .as_str()
        .expect("the current EHR_STATUS uid")
        .to_owned();
    assert!(
        version_two.ends_with("::2"),
        "the viewer's edit must have produced version 2 (got {version_two})"
    );
    status["is_modifiable"] = serde_json::Value::Bool(false);
    put_status(&http, &v1, &ehr_id, &version_two, &status).await;

    // Saving again from the stale screen must be REFUSED, not silently applied.
    h.wait_toasts_cleared().await;
    h.wait_css("#status-save").await.click().await;
    h.wait_xpath("//*[contains(normalize-space(.), 'EHR status changed on the server')]")
        .await;
    // The CDR's own diagnostic stays beside the form as well.
    h.wait_css("#status-diagnostic").await;
    h.shot(1, "status-conflict").await;

    // The superseded versions are still readable by their own uid — a refused
    // update changes nothing, and the history keeps every version.
    h.wait_toasts_cleared().await;
    h.goto(&format!("/ehrs/{ehr_id}?tab=status-history")).await;
    h.wait_css("[data-status-version$='::1']").await;
    assert!(
        click_until_css(
            &h,
            "[data-status-version$='::1']",
            "#status-version-document",
        )
        .await,
        "version 1 must still open by its own OBJECT_VERSION_ID"
    );
    wait_text_suffix(&h, "[data-versioned-fact='version']", "::1").await;
    h.shot(2, "status-version-one").await;

    // The deliberate 412 reaches the browser as a failed server-fn call.
    h.assert_console_clean(&[
        "412",
        "401",
        "500",
        "server function",
        "Failed to load resource",
    ])
    .await;
    h.finish().await;
}
