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
//! End-to-end journeys over the viewer's **openEHR EHR/COMPOSITION write
//! paths** — the ones every openEHR client has, as opposed to the CDR's admin
//! extension (covered by `e2e_admin_ops`):
//!
//! - **client-supplied EHR id**: creating an EHR at an id the operator chooses
//!   (`PUT /ehr/{ehr_id}`), and the CDR's refusal when that id is already used;
//! - **a subject-bound EHR**: creating one through the viewer's OWN form, and
//!   the subject identity then rendering on the EHR the viewer navigated to —
//!   the whole path is the viewer's, so nothing here seeds an EHR over REST;
//! - **composition logical delete**: deleting the latest version of a
//!   composition from the viewer (`DELETE` with `If-Match`), after which the
//!   EHR's composition list no longer offers it.
//!
//! Isolation: both journeys create their OWN data over ITS-REST or through the
//! UI (a fresh UUID per run, a fresh EHR + composition), so neither touches the
//! seeded fixtures the other journeys and the documentation-screenshot pass
//! depend on.

use crate::common;

use std::time::Duration;

use common::{
    Harness, confirm_in_dialog, env, is_present_by, login_basic, poll_until_for, retype,
    wait_css_absent, wait_text_contains,
};
use thirtyfour::prelude::*;

/// The template `scripts/ui-e2e.sh` seeds; its CDR-generated example
/// composition is the body the delete journey commits (spec-valid by
/// construction — never a hand-built fixture).
const SEED_TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

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

/// A fresh UUID-shaped id for this run, built from the clock so repeated runs
/// against the same CDR never collide. Version/variant nibbles are fixed so the
/// value is a well-formed UUID string, which is what the EHR API recommends for
/// a client-supplied `ehr_id`.
///
/// # Panics
/// When the system clock sits outside the range `jiff` supports.
fn generated_uuid() -> String {
    // Wall-clock time comes from jiff, the workspace's one time library.
    let nanos = jiff::Timestamp::now().as_nanosecond().unsigned_abs();
    let low = u32::try_from(nanos & 0xffff_ffff).unwrap_or(1);
    let mid = u16::try_from((nanos >> 32) & 0xffff).unwrap_or(1);
    let node = u64::try_from((nanos >> 16) & 0xffff_ffff_ffff).unwrap_or(1);
    format!("{low:08x}-{mid:04x}-4000-8000-{node:012x}")
}

/// Click the EHR-create button until the viewer leaves for the new EHR's
/// detail route, returning whether it did.
///
/// The button is an `on:click` affordance, so a click landing before hydration
/// is simply lost — the bounded retry is the login-submit precedent. Each
/// attempt waits long enough (10 s) that a slow-but-successful create is not
/// clicked twice.
async fn create_ehr_until_navigated(h: &Harness, ehr_id: &str) -> bool {
    for _ in 0..3 {
        h.wait_css("#ehr-create-submit").await.click().await;
        if poll_until_for(Duration::from_secs(10), async || {
            h.current_url().await.contains(ehr_id)
        })
        .await
        {
            return true;
        }
    }
    false
}

/// Click `css` until `xpath` shows up, returning whether it did.
///
/// Same reason as [`create_ehr_until_navigated`]: an `on:click` affordance
/// clicked before hydration loses the click, and re-clicking is safe here (the
/// validation retry sends nothing, and a repeated conflicting create is refused
/// again).
///
/// # Panics
/// On any interaction failure.
async fn click_until_xpath(h: &Harness, css: &str, xpath: &str) -> bool {
    for _ in 0..5 {
        h.wait_css(css).await.click().await;
        if poll_until_for(Duration::from_secs(5), async || {
            is_present_by(h, By::XPath(xpath)).await
        })
        .await
        {
            return true;
        }
    }
    false
}

/// A client-supplied EHR id creates exactly that EHR (`PUT /ehr/{ehr_id}`), the
/// detail screen's summary header reports the EHR resource's own facts, and
/// re-using the same id is refused by the CDR with actionable copy — nothing is
/// silently overwritten.
#[tokio::test]
async fn client_supplied_ehr_id_creates_that_ehr_and_then_conflicts() {
    let Some(h) = Harness::start("ehr-client-supplied-id").await else {
        return;
    };
    login_basic(&h).await;
    let ehr_id = generated_uuid();

    // A non-UUID id is refused BEFORE any round-trip (client-side validation).
    h.goto("/ehrs").await;
    retype(&h, "#ehr-create-id", "not-a-uuid").await;
    assert!(
        click_until_xpath(
            &h,
            "#ehr-create-submit",
            "//*[@role='alert' and contains(., 'UUID')]",
        )
        .await,
        "a non-UUID EHR id must be refused inline before anything is sent"
    );
    h.shot(1, "ehr-id-validation").await;

    // The real create: the viewer lands on THAT id's detail route.
    retype(&h, "#ehr-create-id", &ehr_id).await;
    assert!(
        create_ehr_until_navigated(&h, &ehr_id).await,
        "creating EHR {ehr_id} never navigated to its detail route"
    );
    // The summary header is the EHR resource read (`GET /ehr/{ehr_id}`): it
    // names the very id that was supplied.
    let summary = h.wait_css("#ehr-summary").await;
    let text = summary.text().await;
    assert!(
        text.contains(&ehr_id),
        "the EHR summary must name the created id (got `{text}`)"
    );
    h.shot(2, "ehr-created-with-supplied-id").await;

    // The same id again: the CDR answers 409 and the viewer says so instead of
    // pretending the create worked.
    h.goto("/ehrs").await;
    retype(&h, "#ehr-create-id", &ehr_id).await;
    assert!(
        click_until_xpath(
            &h,
            "#ehr-create-submit",
            "//*[contains(@class, 'thaw-toast-body') and contains(., 'conflicting')]",
        )
        .await,
        "re-using EHR id {ehr_id} must surface the CDR's conflict, not a success"
    );
    h.shot(3, "ehr-id-conflict").await;

    // Deliberate negative steps: the refused create reaches the browser as a
    // failed server-fn call.
    h.assert_console_clean(&[
        "409",
        "500",
        "server function",
        "401",
        "Failed to load resource",
    ])
    .await;
    h.finish().await;
}

/// The namespace the subject-bound create journey issues its subject ids in.
const SUBJECT_NAMESPACE: &str = "e2e-viewer-subjects";

/// Click the EHR-create button until the viewer leaves `/ehrs`, returning
/// whether it did.
///
/// The subject-bound create lets the CDR mint the id, so there is nothing to
/// watch the URL for except leaving the finder. Same bounded retry as
/// [`create_ehr_until_navigated`]: a click landing before hydration is lost.
async fn create_ehr_until_left_finder(h: &Harness) -> bool {
    for _ in 0..3 {
        h.wait_css("#ehr-create-submit").await.click().await;
        if poll_until_for(Duration::from_secs(10), async || {
            !h.current_url().await.ends_with("/ehrs")
        })
        .await
        {
            return true;
        }
    }
    false
}

/// The viewer's OWN **create-EHR-with-subject** path: the form binds the
/// subject, and the EHR the viewer lands on carries it — in the detail
/// header's identity strip and on the Status tab, both from the one
/// current-`EHR_STATUS` read.
///
/// Nothing here is seeded over REST: the point of the journey is that the
/// viewer can create a subject-bound EHR through its own form at all. It could
/// not, once — the `EHR_STATUS` it built carried no `archetype_details`, so the
/// CDR refused it `422` — and every journey seeding EHRs over REST is why that
/// shipped unnoticed.
#[tokio::test]
async fn a_subject_bound_ehr_created_in_the_viewer_carries_its_subject() {
    let Some(h) = Harness::start("ehr-create-with-subject").await else {
        return;
    };
    login_basic(&h).await;
    // A run-unique subject id, so a shared stack can hold several runs'.
    let subject_id = format!("patient-{}", generated_uuid());

    h.goto("/ehrs").await;
    retype(&h, "#ehr-create-subject-id", &subject_id).await;
    retype(&h, "#ehr-create-subject-namespace", SUBJECT_NAMESPACE).await;
    assert!(
        create_ehr_until_left_finder(&h).await,
        "creating a subject-bound EHR never navigated to its detail route — the viewer's own \
         create path is broken (a 422 from the CDR toasts here)"
    );

    // The header's identity strip names the subject id AND its namespace.
    let identity = h.wait_css("#ehr-identity").await;
    let text = identity.text().await;
    assert!(
        text.contains(&subject_id) && text.contains(SUBJECT_NAMESPACE),
        "the header must name the subject the form bound (got `{text}`)"
    );
    h.shot(1, "ehr-created-with-subject").await;

    // The Status tab reads the same current EHR_STATUS and spells the subject
    // identically — one read, two renderings. It is also the tab the detail
    // screen opens on, so the assertion below runs on the landing page; opening
    // it explicitly first puts the tab in the URL.
    h.wait_css("a[href$='tab=status']").await.click().await;
    h.wait_url_contains("tab=status").await;
    wait_text_contains(&h, "[data-status-fact='subject']", &subject_id).await;
    wait_text_contains(&h, "[data-status-fact='subject']", SUBJECT_NAMESPACE).await;
    h.shot(2, "ehr-status-carries-the-subject").await;

    // The same subject now resolves through the finder's subject lookup, which
    // is what binding it was for.
    h.goto("/ehrs").await;
    retype(&h, "#ehr-subject-id", &subject_id).await;
    retype(&h, "#ehr-subject-namespace", SUBJECT_NAMESPACE).await;
    assert!(
        click_until_xpath(&h, "#ehr-subject-find", "//*[@id='ehr-identity']").await,
        "the subject the viewer bound must be findable by subject id + namespace"
    );
    h.shot(3, "ehr-found-by-subject").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The composition viewer's **logical delete**: a freshly committed composition
/// is deleted from the viewer (with `If-Match` on its latest version), the
/// viewer reports the outcome and returns to the EHR's compositions tab, and
/// the deleted composition is no longer listed there.
#[tokio::test]
async fn composition_logical_delete_leaves_the_list_without_it() {
    let Some(h) = Harness::start("composition-logical-delete").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    // Seed this journey's OWN EHR + composition over ITS-REST, so no other
    // journey's fixture can be destroyed here.
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let ehr_body: serde_json::Value = http
        .post(format!("{v1}/ehr"))
        .basic_auth("ferroehr", Some("ferroehr"))
        .header("Prefer", "return=representation")
        .header("Accept", "application/json")
        .send()
        .await
        .expect("create an EHR")
        .json()
        .await
        .expect("EHR body");
    let ehr_id = ehr_body["ehr_id"]["value"]
        .as_str()
        .expect("the created ehr_id")
        .to_owned();
    let example: String = http
        .get(format!(
            "{v1}/definition/template/adl1.4/{SEED_TEMPLATE_ID}/example"
        ))
        .basic_auth("ferroehr", Some("ferroehr"))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("the template's example composition")
        .text()
        .await
        .expect("example body");
    let committed = http
        .post(format!("{v1}/ehr/{ehr_id}/composition"))
        .basic_auth("ferroehr", Some("ferroehr"))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Prefer", "return=minimal")
        .body(example)
        .send()
        .await
        .expect("commit a composition");
    assert_eq!(
        committed.status().as_u16(),
        201,
        "the seed composition must commit"
    );
    let version_uid = committed
        .headers()
        .get("etag")
        .and_then(|v| v.to_str().ok())
        .map(|etag| etag.trim_start_matches("W/").trim_matches('"').to_owned())
        .expect("the committed version's ETag");
    let vo_id = version_uid
        .split("::")
        .next()
        .expect("the versioned-object id")
        .to_owned();

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}/compositions/{vo_id}"))
        .await;
    // The revision history must have loaded: the delete targets the version the
    // history reports as latest.
    h.wait_css("#version-select").await;
    // The versioned-object card proves the direct VERSION read resolved too.
    h.wait_css("[data-versioned-fact='lifecycle']").await;
    h.shot(1, "composition-before-delete").await;

    confirm_in_dialog(&h, "#composition-delete", "composition-delete-confirm").await;
    // Wait for the mutation's OWN outcome before anything else — the success
    // toast. (The viewer navigates to the compositions tab on success; a
    // failure would toast the CDR's diagnostic here instead.)
    h.wait_xpath("//*[contains(normalize-space(.), 'Composition deleted')]")
        .await;
    h.shot(2, "composition-deleted").await;
    h.wait_url_contains("tab=compositions").await;
    // The list reloads with the deleted composition gone: this EHR held exactly
    // one, so the tab settles on its empty state (waiting for that first is what
    // makes the assert-gone below a real assertion rather than a race with the
    // still-loading table).
    h.wait_xpath("//*[contains(., 'No compositions in this EHR')]")
        .await;
    wait_css_absent(&h, &format!("a[href*='{vo_id}']")).await;
    h.shot(3, "compositions-without-deleted").await;

    h.assert_console_clean(&["401", "404", "Failed to load resource"])
        .await;
    h.finish().await;
}
