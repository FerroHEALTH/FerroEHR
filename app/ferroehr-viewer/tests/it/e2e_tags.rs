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
//! End-to-end journey over the viewer's **EHR-side `ITEM_TAG` surfaces** — the
//! openEHR tag operations every client has:
//!
//! - the composition viewer's tag panel sets two tags and lists them as rows;
//! - the EHR detail's **Tags** tab finds them grouped under the composition
//!   they are on, and its **Open** action resolves the tagged id back to the
//!   composition viewer (a tag names its target without naming that target's
//!   kind, so the viewer has to ask the CDR);
//! - deleting one tag by key leaves the other, which is the released
//!   delete-by-key contract.
//!
//! Isolation: the journey creates its OWN EHR and composition over ITS-REST, so
//! it never touches the fixtures the other journeys or the
//! documentation-screenshot pass depend on.

use crate::common;

use common::{Harness, fill, login_basic, wait_main_contains, wait_text_contains};

/// The CDR base URL the harness exports for REST-side test setup; `None` skips
/// with a reason.
fn cdr_url() -> Option<String> {
    if let Some(url) = common::env("UI_E2E_CDR_URL") {
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
        common::env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned()),
        common::env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// The operational template `scripts/ui-e2e.sh` uploads before the journeys
/// run; its CDR-generated example is this journey's composition body.
const SEED_TEMPLATE: &str = "minimal_evaluation.en.v1";

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

/// Commit one composition into `ehr_id` over ITS-REST and return its
/// versioned-object uid (the id the composition-viewer route addresses).
///
/// The body is the CDR's OWN generated example for the seeded template
/// (`GET /definition/template/adl1.4/{template_id}/example`) — spec-valid by
/// construction, the same source `scripts/ui-e2e.sh` seeds from, so this
/// journey hand-builds no clinical fixture.
///
/// # Panics
/// When the example read or the commit is refused (a broken stack).
async fn create_composition(http: &reqwest::Client, v1: &str, ehr_id: &str) -> String {
    let (user, pass) = basic_credentials();
    let example = http
        .get(format!(
            "{v1}/definition/template/adl1.4/{SEED_TEMPLATE}/example"
        ))
        .basic_auth(&user, Some(&pass))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("read the template's example composition")
        .text()
        .await
        .expect("example body");
    let response = http
        .post(format!("{v1}/ehr/{ehr_id}/composition"))
        .basic_auth(&user, Some(&pass))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Prefer", "return=representation")
        .body(example)
        .send()
        .await
        .expect("commit the composition");
    let status = response.status();
    let body: serde_json::Value = response.json().await.expect("composition body");
    assert!(
        status.is_success(),
        "the CDR must accept the composition commit (got {status}): {body}"
    );
    let version_uid = body
        .get("uid")
        .and_then(|uid| uid.get("value"))
        .and_then(serde_json::Value::as_str)
        .expect("the committed version uid");
    version_uid
        .split("::")
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// Set the tag `key` to `value` on the Tags tab on screen.
async fn set_tag(h: &Harness, key: &str, value: &str) {
    fill(h, "#tk", key).await;
    fill(h, "#tv", value).await;
    common::submit(h, "button[value='set-tag']").await;
    h.wait_css(".fui-toast-text").await;
    wait_text_contains(h, "main table", key).await;
}

/// Tags on a composition: set two on its Tags tab, find one in the EHR's Tags
/// tab filtered by key, open the composition back from that row, and remove
/// the other by key.
#[tokio::test]
async fn composition_tags_are_set_browsed_and_deleted() {
    let Some(h) = Harness::start("ehr-tags").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let ehr_id = create_ehr(&http, &v1).await;
    let composition = create_composition(&http, &v1, &ehr_id).await;
    let kept = format!("viewer-kept-{}", jitter());
    let removed = format!("viewer-removed-{}", jitter());

    login_basic(&h).await;
    let tags = format!("/ehrs/{ehr_id}/compositions/{composition}?tab=tags");
    h.goto(&tags).await;
    wait_main_contains(&h, "carries no tag").await;
    h.shot(1, "composition-tags-empty").await;

    set_tag(&h, &kept, "follow-up").await;
    set_tag(&h, &removed, "true").await;
    h.shot(2, "composition-tags-set").await;

    h.goto(&format!("/ehrs/{ehr_id}/tags?key={kept}")).await;
    wait_text_contains(&h, "main table", &kept).await;
    let listed = h.wait_css("main table").await.text().await;
    assert!(
        !listed.contains(&removed),
        "the key filter leaves `{removed}` out: {listed}"
    );
    h.shot(3, "ehr-tag-browser").await;
    h.wait_css(&format!("main table a[href*='{composition}']"))
        .await
        .click()
        .await;
    h.wait_url_contains(&format!("/compositions/{composition}"))
        .await;
    h.shot(4, "tag-target-opened").await;

    h.goto(&tags).await;
    common::submit(&h, &format!("button[value='remove-tag:{removed}']")).await;
    h.wait_css(".fui-toast-text").await;
    wait_text_contains(&h, "main table", &kept).await;
    let left = h.wait_css("main table").await.text().await;
    assert!(!left.contains(&removed), "`{removed}` is gone: {left}");
    h.shot(5, "composition-tag-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The EHR status's own tag collection: a tag on the `VERSIONED_EHR_STATUS`
/// is listed on the EHR's Tags tab under the EHR status, a different target
/// from any composition, and its row opens the Status tab.
#[tokio::test]
async fn ehr_status_tags_are_their_own_collection() {
    let Some(h) = Harness::start("ehr-status-tags").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let ehr_id = create_ehr(&http, &v1).await;
    let key = format!("viewer-status-{}", jitter());
    set_status_tag(&http, &v1, &ehr_id, &key).await;

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}/tags?key={key}")).await;
    wait_text_contains(&h, "main table", &key).await;
    wait_text_contains(&h, "main table", "EHR status").await;
    h.shot(1, "status-tag-browsed").await;
    h.wait_css(&format!("main table a[href='/ehrs/{ehr_id}/status']"))
        .await
        .click()
        .await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}/status")).await;
    h.shot(2, "status-tag-target-opened").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Tag the EHR's current `EHR_STATUS` container with `key` over ITS-REST.
async fn set_status_tag(http: &reqwest::Client, v1: &str, ehr_id: &str, key: &str) {
    let (user, pass) = basic_credentials();
    let status: serde_json::Value = http
        .get(format!("{v1}/ehr/{ehr_id}/ehr_status"))
        .basic_auth(&user, Some(&pass))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("read the EHR_STATUS")
        .json()
        .await
        .expect("EHR_STATUS body");
    let uid = status
        .pointer("/uid/value")
        .and_then(serde_json::Value::as_str)
        .expect("its uid");
    let container = uid.split("::").next().expect("the versioned object id");
    let answer = http
        .put(format!("{v1}/ehr/{ehr_id}/ehr_status/{container}/tags"))
        .basic_auth(&user, Some(&pass))
        .header("Content-Type", "application/json")
        .json(&serde_json::json!([{ "key": key, "value": "reviewed" }]))
        .send()
        .await
        .expect("tag the EHR status");
    assert!(
        answer.status().is_success(),
        "the CDR takes the status tag (got {})",
        answer.status()
    );
}

/// A run-unique suffix for the fixtures a journey creates, so a shared stack
/// can hold several runs' tags without collision.
///
/// The process id distinguishes runs and the counter distinguishes calls within
/// one — the harness needs distinctness, not entropy, and no clock (the
/// wall-clock reads a `disallowed-methods` ban anyway).
fn jitter() -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{seq}", std::process::id())
}
