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
//! End-to-end journey over the results pane's multi-series chart: the raw AQL
//! editor runs a two-numeric-column query against the seeded compositions, and
//! the chart renders one legend entry per column, offers the ISO-8601 column as
//! the time axis, and hides a series when its legend chip is clicked.

use crate::common;

use std::time::Duration;

use common::{Harness, env, find_all, is_visible, login_basic, poll_until, poll_until_for};
use thirtyfour::prelude::*;

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

/// How many chartistry line paths currently carry geometry. A hidden series
/// keeps its `<path>` element but draws nothing (its Y values are `NaN`, which
/// chartistry renders as missing data), so this is the DOM-visible count of
/// series actually on screen.
async fn drawn_lines(h: &Harness) -> usize {
    let paths = find_all(h, "g._chartistry_line path").await;
    let mut drawn = 0;
    for path in paths {
        // NOTE: probe tier — chartistry re-renders the whole plot on every
        // toggle, so a path found a moment ago may already be detached, and a
        // path with no `d` yet is simply not drawn.
        if let Some(geometry) = path.read_attr("d").await
            && !geometry.trim().is_empty()
        {
            drawn += 1;
        }
    }
    drawn
}

/// Wait until exactly `expected` line paths carry geometry.
///
/// # Panics
/// When the count never settles on `expected` within the budget.
async fn wait_drawn_lines(h: &Harness, expected: usize) {
    if poll_until(async || drawn_lines(h).await == expected).await {
        return;
    }
    panic!(
        "the chart never drew exactly {expected} series (last count: {})",
        drawn_lines(h).await
    );
}

/// The chart draws one series per numeric result column, names each in the
/// legend, defaults to the temporal X axis, and hides a series on demand.
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

    // Run the query from the raw editor (the results pane both query screens
    // share — the builder renders the same component).
    h.goto("/queries/aql").await;
    // Save/Run dispatch is hydrated behaviour (#2285's class).
    h.wait_hydrated().await;
    h.wait_css("#aql-editor").await.send_keys(CHART_AQL).await;
    // Run stays DISABLED until the editor's content reaches the signal, so the
    // wait has to include the condition — a click on the disabled button is
    // intercepted by the toolbar above it, not merely lost.
    h.wait_clickable_xpath("//button[normalize-space(.)='Run']")
        .await
        .click()
        .await;
    h.wait_css("table tbody tr").await;
    h.shot(1, "results-table").await;

    // Switch to the chart pane. A click landing before hydration is simply
    // lost (the login-submit precedent), so re-click until the pane shows.
    let mut charted = false;
    for _ in 0..5 {
        h.wait_xpath("//button[normalize-space(.)='Chart']")
            .await
            .click()
            .await;
        charted = poll_until_for(Duration::from_secs(3), async || {
            is_visible(&h, "[data-results-chart]").await
        })
        .await;
        if charted {
            break;
        }
    }
    assert!(
        charted,
        "the Chart toggle never revealed the chart pane (pre-hydration clicks exhausted)"
    );

    // Two numeric columns → two derived series (the hook carries the count).
    let pane = h.wait_css("[data-results-chart]").await;
    assert_eq!(
        pane.attr("data-results-chart").await,
        "2",
        "the two numeric result columns must derive two series"
    );

    // The legend names both series.
    h.wait_css("[data-chart-legend]").await;
    let chips = find_all(&h, "[data-chart-series]").await;
    assert_eq!(chips.len(), 2, "one legend entry per series");
    let mut names = Vec::new();
    for chip in &chips {
        names.push(chip.attr("data-chart-series").await);
    }
    names.sort();
    assert_eq!(
        names,
        vec!["magnitude".to_owned(), "magnitude_repeat".to_owned()],
        "the legend names the result-set columns"
    );

    // The ISO-8601 column is offered as the X axis, and is the default.
    let axis = h.wait_css("select[data-chart-axis]").await;
    let options = axis.find_all(By::Tag("option")).await;
    assert_eq!(
        options.len(),
        2,
        "the timestamp column plus the row-order fallback"
    );
    assert_eq!(
        axis.prop("value").await,
        "0",
        "the temporal axis is the default"
    );
    // contains(): leptos interleaves hydration markers with text nodes, so an
    // exact text comparison is unreliable.
    let first_axis = options.first().expect("the first axis option").text().await;
    assert!(
        first_axis.contains("observed"),
        "the temporal axis is named after its column (got `{first_axis}`)"
    );

    // Both series are drawn.
    wait_drawn_lines(&h, 2).await;
    h.shot(2, "chart-two-series").await;

    // Toggling one legend chip hides exactly that series.
    let chip = chips.first().expect("a legend chip");
    let hidden = chip.attr("data-chart-series").await;
    chip.click().await;
    // NOTE: probe tier — the chip is re-rendered by the toggle it just
    // dispatched, so a stale handle mid-poll is "not yet", not a failure.
    poll_until(async || chip.read_attr("data-visible").await.as_deref() == Some("false")).await;
    assert_eq!(
        chip.attr("data-visible").await,
        "false",
        "the clicked legend chip reports `{hidden}` as hidden"
    );
    assert_eq!(
        chip.attr("aria-pressed").await,
        "false",
        "the chip's pressed state follows its series"
    );
    wait_drawn_lines(&h, 1).await;
    h.shot(3, "chart-one-series-hidden").await;

    // The last visible series cannot be hidden — the chart never empties.
    let survivor = chips.get(1).expect("the second legend chip");
    assert!(
        !survivor.enabled().await,
        "the only remaining series is not hideable"
    );

    // Showing it again brings the line back.
    chip.click().await;
    wait_drawn_lines(&h, 2).await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
