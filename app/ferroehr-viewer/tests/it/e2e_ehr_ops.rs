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

use common::{
    Harness, confirm_in_dialog, env, fill, login_basic, wait_css_absent, wait_main_contains,
};

/// The template the seeded compositions use.
const SEED_TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

/// The namespace the subject-bound EHR journey binds its subject in.
const SUBJECT_NAMESPACE: &str = "e2e-viewer-subjects";

/// The CDR origin the journeys seed through, or `None` (with a printed skip).
fn cdr_url() -> Option<String> {
    if let Some(url) = env("UI_E2E_CDR_URL") {
        Some(url)
    } else {
        println!("SKIP: UI_E2E_CDR_URL unset (run scripts/ui-e2e.sh)");
        None
    }
}

/// A fresh UUID-shaped id from the clock, unique per run.
fn generated_uuid() -> String {
    let nanos = jiff::Timestamp::now().as_nanosecond().unsigned_abs();
    let low = u32::try_from(nanos & 0xffff_ffff).unwrap_or(1);
    let mid = u16::try_from((nanos >> 32) & 0xffff).unwrap_or(1);
    let node = u64::try_from((nanos >> 16) & 0xffff_ffff_ffff).unwrap_or(1);
    format!("{low:08x}-{mid:04x}-4000-8000-{node:012x}")
}

/// Fill New EHR and create it.
async fn create_ehr(h: &Harness, ehr_id: &str, subject: Option<(&str, &str)>) {
    h.goto("/ehrs/new").await;
    if let Some((id, namespace)) = subject {
        fill(h, "#s-id", id).await;
        fill(h, "#s-ns", namespace).await;
    }
    fill(h, "#e-id", ehr_id).await;
    common::submit(h, "#editor-dock button[value='save']").await;
}

/// A client-supplied EHR id creates exactly that EHR (`PUT /ehr/{ehr_id}`),
/// a malformed one is refused before anything is sent, and the same id again
/// is refused by the CDR with a link to the EHR that holds it.
#[tokio::test]
async fn client_supplied_ehr_id_creates_that_ehr_and_then_conflicts() {
    let Some(h) = Harness::start("ehr-client-supplied-id").await else {
        return;
    };
    login_basic(&h).await;
    let ehr_id = generated_uuid();

    create_ehr(&h, "not-a-uuid", None).await;
    wait_main_contains(&h, "An EHR id is a UUID").await;
    assert!(h.current_url().await.ends_with("/ehrs/new"));
    h.shot(1, "ehr-id-validation").await;

    create_ehr(&h, &ehr_id, None).await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}")).await;
    h.wait_css(&format!("main a[href^='/ehrs/{ehr_id}/']"))
        .await;
    h.shot(2, "ehr-created-with-supplied-id").await;

    create_ehr(&h, &ehr_id, None).await;
    wait_main_contains(&h, "409").await;
    wait_main_contains(&h, "An EHR with this id exists already").await;
    h.wait_css(&format!("main a[href='/ehrs/{ehr_id}']")).await;
    h.shot(3, "ehr-id-conflict").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The viewer's own create-with-subject path: the form binds the subject, the
/// EHR it lands on carries it in its header and on the Status tab, and the
/// EHR list finds it by subject id and namespace.
#[tokio::test]
async fn a_subject_bound_ehr_created_in_the_viewer_carries_its_subject() {
    let Some(h) = Harness::start("ehr-create-with-subject").await else {
        return;
    };
    login_basic(&h).await;
    let subject_id = format!("patient-{}", generated_uuid());
    create_ehr(&h, "", Some((&subject_id, SUBJECT_NAMESPACE))).await;
    h.wait_url_not_contains("/ehrs/new").await;
    let url = h.current_url().await;
    let ehr_path = url
        .get(url.find("/ehrs/").expect("the create landed on an EHR")..)
        .expect("a path")
        .to_owned();
    let header = h.wait_css(".fui-subject").await.text().await;
    assert!(
        header.contains(SUBJECT_NAMESPACE),
        "the header names the subject's namespace (got `{header}`)"
    );
    let tail = subject_id
        .get(subject_id.len() - 3..)
        .expect("a long subject id");
    assert!(
        header.contains(tail),
        "the masked subject ends in `{tail}`: {header}"
    );
    h.shot(1, "ehr-created-with-subject").await;

    h.wait_css("a.fui-tab[data-tab='status']")
        .await
        .click()
        .await;
    h.wait_url_contains("/status").await;
    wait_main_contains(&h, SUBJECT_NAMESPACE).await;
    h.shot(2, "ehr-status-carries-the-subject").await;

    h.goto("/ehrs").await;
    fill(&h, "#ehrs-find", &subject_id).await;
    fill(&h, "#ehrs-namespace", SUBJECT_NAMESPACE).await;
    common::submit(&h, "form[action='/ehrs'] button[type=submit]").await;
    // The list shows the subject masked, and the one EHR bound to it.
    wait_main_contains(&h, &format!("Subject: ••••••{tail}")).await;
    assert_eq!(common::count_matching(&h, "main table tbody tr").await, 1);
    h.wait_css(&format!("main a.fui-row-link[href='{ehr_path}']"))
        .await;
    h.shot(3, "ehr-found-by-subject").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

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
    h.wait_css("h2.fui-reading-title").await;
    h.shot(1, "composition-before-delete").await;

    h.wait_css("summary[aria-controls='composition-more']")
        .await
        .click()
        .await;
    confirm_in_dialog(
        &h,
        "button[commandfor='delete-composition']",
        "#delete-composition button[type=submit]",
    )
    .await;
    h.wait_css(".fui-toast-text").await;
    h.shot(2, "composition-deleted").await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}")).await;
    wait_main_contains(&h, "This EHR holds no composition yet").await;
    wait_css_absent(&h, &format!("main a[href*='{vo_id}']")).await;
    h.shot(3, "compositions-without-deleted").await;

    h.assert_console_clean(&["401", "404", "Failed to load resource"])
        .await;
    h.finish().await;
}
