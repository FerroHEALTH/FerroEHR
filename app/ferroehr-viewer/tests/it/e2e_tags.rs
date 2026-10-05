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

use common::{Harness, login_basic, retype, wait_css_absent, wait_text};

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

/// Set one tag through whichever tag panel is on screen.
///
/// # Panics
/// When the panel never reports the write.
async fn set_tag(h: &Harness, key: &str, value: &str) {
    // A visible toast overlays the bottom-right corner and intercepts clicks.
    h.wait_toasts_cleared().await;
    retype(h, "#tag-key", key).await;
    retype(h, "#tag-value", value).await;
    h.wait_css("#tag-save").await.click().await;
    assert!(
        wait_text(h, "Tag saved").await,
        "saving tag `{key}` never reported the replaced collection: {}",
        h.evidence_dump("tag-save").await
    );
    h.wait_css(&format!("[data-tag-key='{key}']")).await;
}

/// Tags on a composition: set two through the viewer's panel, find them in the
/// EHR-wide browser grouped under their target, open the composition back from
/// that group, and delete one by key.
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
    // Unique keys, so the browser's filter finds exactly this journey's tags.
    let kept = format!("viewer-kept-{}", jitter());
    let removed = format!("viewer-removed-{}", jitter());

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}/compositions/{composition}"))
        .await;
    // The panel names the collection it edits: on Latest that is the
    // VERSIONED_COMPOSITION container, never a version.
    h.wait_css("#composition-tag-set").await;
    h.wait_css(&format!("[data-tag-collection='{composition}']"))
        .await;
    assert!(
        wait_text(&h, "No tags on this collection").await,
        "a fresh composition carries no tags: {}",
        h.evidence_dump("tags-empty").await
    );
    h.shot(1, "composition-tags-empty").await;

    set_tag(&h, &kept, "follow-up").await;
    set_tag(&h, &removed, "true").await;
    // Both rows are on screen — the second save merged rather than replaced,
    // which is what the read-modify-write around the whole-collection PUT is
    // for.
    h.wait_css(&format!("[data-tag-key='{kept}']")).await;
    h.wait_css(&format!("[data-tag-key='{removed}']")).await;
    h.shot(2, "composition-tags-set").await;

    // The EHR-wide browser lists both, grouped under the composition they are
    // on. The filter is URL state and keeps the tab.
    h.wait_toasts_cleared().await;
    h.goto(&format!("/ehrs/{ehr_id}?tab=tags&tag_key={kept}"))
        .await;
    h.wait_css("#ehr-tag-browser").await;
    let group = format!("[data-tag-target='{composition}']");
    h.wait_css(&group).await;
    h.wait_css(&format!("[data-tag-key='{kept}']")).await;
    // The filter really filtered: the other key is not in this window.
    wait_css_absent(&h, &format!("[data-tag-key='{removed}']")).await;
    h.shot(3, "ehr-tag-browser").await;

    // Opening the group resolves the tagged id back to its owner — a tag names
    // its target without naming that target's kind, so this is a CDR question.
    h.wait_css(&group).await.click().await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}/compositions/{composition}"))
        .await;
    h.wait_css("#composition-tag-set").await;
    h.shot(4, "tag-target-opened").await;

    // Deleting addresses the KEY alone, which is what the openEHR tag delete
    // does — and the other tag stays.
    h.wait_toasts_cleared().await;
    h.wait_css(&format!("[data-tag-delete='{removed}']"))
        .await
        .click()
        .await;
    assert!(
        wait_text(&h, "Tag deleted").await,
        "deleting the tag never reported: {}",
        h.evidence_dump("tag-delete").await
    );
    wait_css_absent(&h, &format!("[data-tag-key='{removed}']")).await;
    h.wait_css(&format!("[data-tag-key='{kept}']")).await;
    h.shot(5, "composition-tag-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The EHR status's own tag collection: the Status tab's panel sets a tag on
/// the `VERSIONED_EHR_STATUS` container, and the EHR-wide browser then lists it
/// under a DIFFERENT target than the composition's — the two objects hold
/// separate collections.
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

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}?tab=status")).await;
    h.wait_css("#ehr-status-tag-set").await;
    set_tag(&h, &key, "reviewed").await;
    h.shot(1, "status-tag-set").await;

    // The EHR-wide browser finds it — on the status container, not on any
    // composition.
    h.wait_toasts_cleared().await;
    h.goto(&format!("/ehrs/{ehr_id}?tab=tags&tag_key={key}"))
        .await;
    h.wait_css("#ehr-tag-browser").await;
    h.wait_css(&format!("[data-tag-key='{key}']")).await;
    h.shot(2, "status-tag-browsed").await;

    // Opening the group lands back on the Status tab, which is the screen that
    // owns the EHR_STATUS.
    h.wait_css("[data-tag-target]").await.click().await;
    h.wait_url_contains("tab=status").await;
    h.wait_css("#ehr-status-tag-set").await;
    h.wait_css(&format!("[data-tag-key='{key}']")).await;
    h.shot(3, "status-tag-target-opened").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
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
