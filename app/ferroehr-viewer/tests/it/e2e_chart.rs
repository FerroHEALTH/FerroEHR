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
#![allow(
    clippy::too_many_lines,
    reason = "one linear journey: seed, run, assert legend/axis/toggle"
)]
// e2e journeys are assertive by design; skip-with-reason prints; the shared
// harness module is per-test-binary (the corpus.rs test-file precedent)
//! End-to-end journey over the results pane's multi-series chart: Query's AQL
//! tab runs a two-numeric-column query against the seeded compositions, and
//! the chart draws one legend entry per column over the ISO-8601 column as its
//! time axis, and hides a series when its legend chip is pressed.

use crate::common;

use common::{Harness, env, fill, find_all, login_basic, poll_until};

/// A two-numeric-column query over the harness-seeded compositions
/// (`scripts/ui-e2e.sh` commits three FLAT compositions carrying a quantity
/// magnitude, plus the template's own generated example). Column 1 is the
/// composition's start time — the ISO-8601 column the chart offers as its time
/// axis — and columns 2 and 3 project the same magnitude under two aliases: the
/// fixture template has exactly ONE numeric leaf, and two projections of it are
/// two result columns, which is all the multi-series derivation reads.
const CHART_AQL: &str = concat!(
    "SELECT c/context/start_time/value AS observed, ",
    "c/content[openEHR-EHR-EVALUATION.minimal.v1]/data[at0001]/items[at0002]/value/magnitude AS magnitude, ",
    "c/content[openEHR-EHR-EVALUATION.minimal.v1]/data[at0001]/items[at0002]/value/magnitude AS magnitude_repeat ",
    "FROM EHR e CONTAINS COMPOSITION c ",
    "WHERE c/archetype_details/template_id/value = 'minimal_evaluation.en.v1'"
);

/// Whether the harness seeded clinical data (the chart needs real values, and
/// the seeded ids are the signal that the seeding step ran).
fn seeded() -> bool {
    if env("UI_E2E_SEEDED_EHR_ID").is_some() {
        return true;
    }
    println!("SKIP: UI_E2E_SEEDED_EHR_ID unset (run scripts/ui-e2e.sh)");
    false
}

/// The plot of the chart the results pane draws.
const CHART: &str = "#results-chart-plot";

/// The chart draws one series per numeric result column, names each in the
/// legend, reads the temporal column as its X axis, and hides a series when
/// its hydrated legend chip is pressed.
#[tokio::test]
async fn results_chart_groups_series_and_toggles_them() {
    let Some(h) = Harness::start("results-chart").await else {
        return;
    };
    if !seeded() {
        h.finish().await;
        return;
    }
    login_basic(&h).await;
    h.goto("/query/aql").await;
    fill(&h, "#aql", CHART_AQL).await;
    common::submit(&h, "header.fui-page-header button[form='query-form']").await;
    h.wait_css("main table tbody tr").await;
    h.shot(1, "results-table").await;

    common::submit(&h, "main button[name='shown'][value='chart']").await;
    h.wait_css(&format!("{CHART} [data-series-lines='1']"))
        .await;
    common::wait_main_contains(&h, "observed").await;
    h.wait_hydrated().await;
    let chips = find_all(&h, ".fui-legend button.fui-legend-chip").await;
    let mut names = Vec::new();
    for chip in &chips {
        names.push(chip.text().await.trim().to_owned());
    }
    names.sort();
    assert_eq!(
        names,
        vec!["magnitude".to_owned(), "magnitude_repeat".to_owned()],
        "the legend names the two numeric result columns"
    );
    h.shot(2, "chart-two-series").await;

    let chip = h
        .wait_css(".fui-legend button.fui-legend-chip[data-series='magnitude']")
        .await;
    chip.click().await;
    assert_eq!(
        chip.attr("aria-pressed").await,
        "false",
        "the chip is released"
    );
    let hidden = poll_until(async || {
        common::read_attr(&h, &format!("{CHART} [data-series-lines='0']"), "hidden")
            .await
            .is_some()
    })
    .await;
    assert!(hidden, "the released series is hidden in the drawing");
    // The last series shown cannot be hidden, so the chart never empties.
    let last = h
        .wait_css(".fui-legend button.fui-legend-chip[data-series='magnitude_repeat']")
        .await;
    assert!(!last.enabled().await, "the last series shown stays pressed");
    h.shot(3, "chart-one-series").await;

    // The switch says the chart is shown, and the next Run keeps it.
    h.wait_css("main button[name='shown'][value='chart'][aria-pressed='true']")
        .await;
    common::submit(&h, "header.fui-page-header button[form='query-form']").await;
    h.wait_css(&format!("{CHART} [data-series-lines='1']"))
        .await;
    h.wait_css("main button[name='shown'][value='chart'][aria-pressed='true']")
        .await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
