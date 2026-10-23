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
//! End-to-end browse journeys: the template list and a template's tabs, the
//! query builder, and the EHR finder, driven by `scripts/ui-e2e.sh`. Each test
//! owns its own [`Harness`] and skips with a printed reason when the harness
//! environment is absent, so a plain `cargo nextest run` stays green.

use crate::common;

use common::{
    Harness, count_matching, fill, is_present, login_basic, wait_main_contains,
    wait_text_contains,
};

/// The operational template these journeys use: a small SDK OPT, a minimal
/// EVALUATION with a `DV_QUANTITY` and a `DV_CODED_TEXT` leaf.
pub(crate) const TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

/// The list's link to [`TEMPLATE_ID`].
const TEMPLATE_LINK: &str = "a[href='/templates/minimal_evaluation.en.v1']";

/// The absolute path of the fixture OPT, for the file field.
///
/// # Panics
/// When the fixture is missing (a repo-layout error, not a skip).
pub(crate) fn fixture_path(name: &str) -> String {
    let raw = format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"));
    std::fs::canonicalize(raw)
        .expect("the fixture exists")
        .to_string_lossy()
        .into_owned()
}

/// Make sure [`TEMPLATE_ID`] is in the CDR, uploading the fixture only when
/// the list does not show it. Returns whether this call uploaded it.
///
/// # Panics
/// When the upload does not land on the template's page.
pub(crate) async fn ensure_template_present(h: &Harness) -> bool {
    h.goto("/templates").await;
    if is_present(h, TEMPLATE_LINK).await {
        return false;
    }
    common::upload_template(h, "adl14", &fixture_path("minimal_evaluation.opt")).await;
    h.wait_url_contains(&format!("/templates/{TEMPLATE_ID}")).await;
    true
}

/// Uploading an OPT lists it, and its tabs read it back: Paths draws the
/// Web Template tree (the OPT parsed and the `WebTemplate` built), a node opens
/// its path, and Source shows the OPT as stored.
#[tokio::test]
async fn template_upload_lists_and_inspects_path_catalog() {
    let Some(h) = Harness::start("template-catalog").await else {
        return;
    };
    login_basic(&h).await;

    h.goto("/templates").await;
    assert_eq!(
        h.title().await,
        "Templates · FerroEHR Viewer",
        "the shell's Title formatter suffixes every page title"
    );
    h.shot(1, "templates").await;
    let uploaded = ensure_template_present(&h).await;
    println!("this journey performed the upload: {uploaded}");

    h.goto("/templates").await;
    h.wait_css(TEMPLATE_LINK).await.click().await;
    h.wait_url_contains(&format!("/templates/{TEMPLATE_ID}"))
        .await;
    wait_main_contains(&h, "openEHR-EHR-COMPOSITION.minimal.v1").await;
    h.shot(2, "template-overview").await;

    h.wait_css("a.fui-tab[data-tab='paths']").await.click().await;
    h.wait_css("ul.fui-tree[role='tree'] li[role='treeitem']").await;
    h.wait_hydrated().await;
    // The hydrated tree expands in place: the toggle is live once the island
    // runs, and a collapsed branch opens on a click.
    let toggle = h
        .wait_css("li[aria-expanded='false'] > div button.fui-tree-toggle, li[aria-expanded='false'] button.fui-tree-toggle")
        .await;
    toggle.click().await;
    h.wait_css("li[aria-level='3']").await;
    h.shot(3, "template-paths").await;
    // A node's link opens its path, type and codes.
    h.wait_css("a.fui-tree-label[href*='node=n-2']").await.click().await;
    h.wait_url_contains("node=n-2").await;
    wait_main_contains(&h, "/category").await;
    h.shot(4, "template-path-node").await;

    h.wait_css("a.fui-tab[data-tab='source']").await.click().await;
    wait_text_contains(&h, "#template-source-text", "template_id").await;
    h.shot(5, "template-source").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The builder lowers a template, a data point and a condition into runnable
/// AQL (a `CONTAINS COMPOSITION` query) and runs it against the CDR.
#[tokio::test]
async fn query_builder_generates_and_runs_aql() {
    let Some(h) = Harness::start("query-builder").await else {
        return;
    };
    login_basic(&h).await;
    ensure_template_present(&h).await;

    h.goto("/templates").await;
    h.wait_css(&format!("a[href='/query?template={TEMPLATE_ID}']"))
        .await;
    h.goto(&format!("/query?template={TEMPLATE_ID}")).await;
    let selected = h.wait_css("#q-t").await.prop("value").await;
    assert_eq!(selected, TEMPLATE_ID, "the template link picks the template");
    h.shot(1, "builder-template-picked").await;

    // A condition on the composition's start time. Add a condition posts the
    // form before the page hydrates and adds the row in place after.
    h.wait_css("button[name='add-condition'][value='g']")
        .await
        .click()
        .await;
    h.wait_css("select[name='g.c0-path']")
        .await
        .select_by_value("/context/start_time")
        .await;
    h.wait_css("select[name='g.c0-op']").await.select_by_value("ge").await;
    fill(&h, "input[name='g.c0-value']", "2020-01-01T00:00:00Z").await;
    h.shot(2, "builder-condition-added").await;

    common::submit(&h, "header.fui-page-header button[form='query-form']").await;
    wait_main_contains(&h, "CONTAINS COMPOSITION").await;
    wait_main_contains(&h, "2020-01-01T00:00:00Z").await;
    h.wait_css("main table tbody tr").await;
    h.shot(3, "builder-results").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The EHR finder opens an EHR by id, and an unknown id renders the CDR's
/// `404` as the page's refusal rather than a blank screen.
#[tokio::test]
async fn ehr_finder_navigates_and_unknown_ehr_shows_error() {
    let Some(h) = Harness::start("ehr-finder").await else {
        return;
    };
    login_basic(&h).await;

    h.goto("/ehrs").await;
    let rows = count_matching(&h, "main table tbody tr").await;
    assert!(rows > 0, "the seeded stack lists EHRs");
    let unknown = "00000000-0000-4000-8000-0000000000ff";
    fill(&h, "#find", unknown).await;
    h.shot(1, "ehr-finder-typed").await;
    common::submit(&h, "form[action='/find'] button[type=submit]").await;
    wait_main_contains(&h, "No EHR, composition or party has the id").await;
    h.wait_css(&format!("main a[href='/ehrs/{unknown}']")).await;
    h.goto(&format!("/ehrs/{unknown}")).await;
    wait_main_contains(&h, "404").await;
    h.shot(2, "ehr-unknown-error").await;

    h.assert_console_clean(&["404", "Failed to load resource", "401"])
        .await;
    h.finish().await;
}
