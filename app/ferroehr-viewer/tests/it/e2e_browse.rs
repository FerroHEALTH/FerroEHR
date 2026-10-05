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
// e2e journeys are assertive by design; skip-with-reason prints; the shared
// harness module is per-test-binary (the corpus.rs test-file precedent)
//! End-to-end browse journeys — the template manager, the point-and-click
//! query builder, and the EHR finder — driven by `scripts/ui-e2e.sh`. Each
//! test owns its own [`Harness`] and skips with a printed reason when the
//! harness environment is absent, so a plain `cargo nextest run` stays green.

use crate::common;

use std::time::Duration;

use common::{
    Harness, appears_within, count_matching, find_all, is_present, login_basic, poll_until_for,
    wait_text_contains,
};

/// The operational template uploaded (or reused) by these journeys, and its
/// detail-route id. The fixture is a small, real SDK OPT (a minimal
/// EVALUATION with a `DV_QUANTITY` and a `DV_CODED_TEXT` leaf — enough to
/// exercise the path catalog and the query builder without a large tree).
pub(crate) const TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

/// The detail-route anchor the template list renders for [`TEMPLATE_ID`].
const TEMPLATE_LINK: &str = "a[href='/templates/minimal_evaluation.en.v1']";

/// The absolute, canonicalized path to the fixture OPT that
/// [`ensure_template_present`] uploads. Resolved at test runtime relative to
/// this crate so the `WebDriver` file-upload `send_keys` receives a real host path.
///
/// # Panics
/// When the fixture is missing (a repo-layout error, not a skip).
fn fixture_opt_path() -> String {
    let raw = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/minimal_evaluation.opt"
    );
    std::fs::canonicalize(raw)
        .expect("fixture OPT exists")
        .to_string_lossy()
        .into_owned()
}

/// Guarantee [`TEMPLATE_ID`] is present in the CDR: land on `/templates`, wait
/// for the list to settle, and upload the fixture only if the row is absent
/// (so a shared stack that a sibling journey already seeded is not re-POSTed
/// into a `409`). The upload goes through the screen's one dialog
/// ([`common::upload_via_dialog`]), the same control the ADL2 family drives.
///
/// Returns `true` when this call performed the upload.
///
/// # Panics
/// On any navigation/interaction failure (journeys are assertive end-to-end).
pub(crate) async fn ensure_template_present(h: &Harness) -> bool {
    h.goto("/templates").await;
    h.wait_css("#template-upload-open").await;
    // Let the list Transition resolve to either a row link or the empty-state
    // message bar before deciding whether an upload is needed.
    h.wait_css("a[href^='/templates/'], .thaw-message-bar")
        .await;
    if is_present(h, TEMPLATE_LINK).await {
        return false;
    }
    let path = fixture_opt_path();
    // The change event that fills the dialog's editor is a HYDRATED listener,
    // and a file set before it exists is unrecoverable by retrying — a re-send
    // of the same path fires no change event (#2285). Wait for the shell's
    // hydration marker so the first send lands on a live listener; the bounded
    // loop stays as a backstop only.
    h.wait_hydrated().await;
    for _ in 0..4 {
        common::upload_via_dialog(h, &path).await;
        if poll_until_for(Duration::from_secs(8), async || {
            is_present(h, TEMPLATE_LINK).await
        })
        .await
        {
            return true;
        }
    }
    let evidence = h.evidence_dump("upload-exhausted").await;
    panic!("the fixture template never appeared after upload (uploads exhausted; {evidence})");
}

/// Expand every collapsed disclosure in the visible catalog tree so the deep
/// `DV_*` leaves (and their "+ condition"/"+ column" affordances) become
/// interactable — the picker auto-expands only the top two levels, and the
/// data-value leaves sit deeper. The toggle is the `aria-expanded` disclosure
/// button (its chevron is an icon, not text); each pass clicks the collapsed
/// ones, and newly-revealed toggles are clicked on the next pass.
async fn expand_catalog_tree(h: &Harness) {
    for _ in 0..15 {
        let collapsed = find_all(h, "button[aria-expanded='false']").await;
        let mut clicked = false;
        for toggle in collapsed {
            // Expanding a node re-renders that node's own row and mounts its
            // children; it never detaches a sibling, so every handle taken in
            // this pass stays live and both commands are assertive. A driver
            // that stops answering is a failure here, never a toggle that
            // "was not there".
            if toggle.displayed().await {
                toggle.click().await;
                clicked = true;
            }
        }
        if !clicked {
            break;
        }
    }
}

/// Uploading an OPT lists it in the template manager, and its detail screen
/// renders the Web-Template path catalog tree (proving the OPT parsed and the
/// `WebTemplate` built browser-visible).
#[tokio::test]
async fn template_upload_lists_and_inspects_path_catalog() {
    let Some(h) = Harness::start("template-catalog").await else {
        return;
    };
    login_basic(&h).await;

    // The list before any upload this journey performs (best-effort: the table
    // is absent on a fresh stack, so an empty count is expected).
    h.goto("/templates").await;
    h.wait_css("#template-upload-open").await;
    // The rendered tab title: the ONE browser-level pin of the app shell's
    // Title formatter (the unit tests pin the pure fn; only WebDriver sees
    // what the tab actually says).
    assert_eq!(
        h.title().await,
        "Templates · FerroEHR Viewer",
        "the shell's Title formatter suffixes every page title"
    );
    h.wait_css("a[href^='/templates/'], .thaw-message-bar")
        .await;
    let before = count_matching(&h, "table tbody tr").await;
    println!("template rows before upload: {before}");
    h.shot(1, "templates-before-upload").await;

    let uploaded = ensure_template_present(&h).await;
    println!("this journey performed the upload: {uploaded}");
    h.shot(2, "templates-listed").await;

    // Open the detail screen and prove the WT path-catalog tree renders.
    h.wait_css(TEMPLATE_LINK).await.click().await;
    h.wait_url_contains(&format!("/templates/{TEMPLATE_ID}"))
        .await;
    // The tab bar plus a catalog tree node (a selectable label / RM-type span
    // or a disclosure toggle) — present only once the WebTemplate is built.
    h.wait_css("nav[aria-label='Template views']").await;
    h.wait_css("ul.text-sm li button, ul.text-sm li span").await;
    h.shot(3, "template-detail-catalog").await;

    // The OPT tab is fed by the SAME page-level read as the catalog, so the
    // switch shows the operational template's own source with no second fetch.
    h.wait_xpath("//nav[@aria-label='Template views']//a[normalize-space(.)='OPT']")
        .await
        .click()
        .await;
    wait_text_contains(&h, "pre", "template_id").await;
    h.shot(4, "template-detail-opt").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The point-and-click builder lowers a template + a condition into runnable
/// AQL (a `CONTAINS COMPOSITION` query) and executes it against the CDR.
#[tokio::test]
async fn query_builder_generates_and_runs_aql() {
    let Some(h) = Harness::start("query-builder").await else {
        return;
    };
    login_basic(&h).await;
    // The builder needs the template present; seed it if a sibling journey has
    // not (each test is independent and order is not guaranteed).
    ensure_template_present(&h).await;

    h.goto("/queries/builder").await;
    // The template <select> loads under Suspense; pick our template by clicking
    // its <option> (a native option click both selects it and fires change).
    // Bounded retry (the login-submit precedent): a click that lands before
    // hydration mutates the DOM select but no listener fires, so the catalog
    // never loads — re-click until the selection takes.
    let mut selected = false;
    for _ in 0..5 {
        h.wait_css(&format!("#qb-template option[value='{TEMPLATE_ID}']"))
            .await
            .click()
            .await;
        if appears_within(&h, "ul.text-sm li", Duration::from_secs(3)).await {
            selected = true;
            break;
        }
    }
    assert!(
        selected,
        "the template selection never took (pre-hydration clicks exhausted)"
    );
    // The catalog tree loads once a template is chosen.
    h.wait_css("ul.text-sm li").await;
    h.shot(1, "builder-template-picked").await;

    // Reveal the deep data-value leaves, then add the first as a condition.
    expand_catalog_tree(&h).await;
    h.wait_xpath("//button[contains(., '+ condition')]")
        .await
        .click()
        .await;
    h.shot(2, "builder-condition-added").await;

    // The empty criterion correctly renders the typed validation error in
    // the preview ("a range condition needs at least one bound") — fill the
    // range's `from` bound so the lowering produces real AQL.
    h.wait_css("input[placeholder='2026-01-01T00:00:00Z']")
        .await
        .send_keys("2020-01-01T00:00:00Z")
        .await;

    // The live preview is a real, runnable AQL over compositions.
    wait_text_contains(&h, "pre", "CONTAINS COMPOSITION").await;
    h.shot(3, "builder-aql-preview").await;

    // Run it: the results card resolves to a table or the zero-rows state.
    h.wait_xpath("//button[contains(., 'Run')]")
        .await
        .click()
        .await;
    // contains(., …): leptos interleaves hydration comment markers with text
    // nodes, so text()= comparisons are unreliable.
    h.wait_xpath("//div[contains(., 'Results')] | //p[contains(., 'No rows')]")
        .await;
    h.shot(4, "builder-results").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The EHR finder navigates to a detail route, and an unknown EHR renders the
/// CDR's `404` inline on the status tab (a graceful error surface, not a blank
/// screen or a client crash).
#[tokio::test]
async fn ehr_finder_navigates_and_unknown_ehr_shows_error() {
    let Some(h) = Harness::start("ehr-finder").await else {
        return;
    };
    login_basic(&h).await;

    h.goto("/ehrs").await;
    // A syntactically-valid but almost-certainly-absent EHR id.
    let unknown = "00000000-0000-4000-8000-0000000000ff";
    h.wait_css("#ehr-lookup").await.send_keys(unknown).await;
    h.shot(1, "ehr-finder-typed").await;
    h.wait_xpath("//button[contains(., 'Find')]")
        .await
        .click()
        .await;

    // The detail screen renders; the status tab surfaces the CDR 404 inline
    // (the shared inline_error renders as a [role='alert'] danger box).
    h.wait_url_contains(unknown).await;
    h.wait_css("[role='alert']").await;
    h.shot(2, "ehr-unknown-error").await;

    // Deliberate negative step: the CDR 404 (and its network log line) is the
    // expected outcome, so allow it; any OTHER SEVERE entry still fails.
    h.assert_console_clean(&["404", "Failed to load resource", "401"])
        .await;
    h.finish().await;
}
