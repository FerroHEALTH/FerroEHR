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
//! End-to-end journeys over Server's management tabs: Health over the CDR's
//! readiness probe, the build facts on Overview, the metric registry on
//! Metrics, and the live log filter on Logging (apply, assert effective,
//! reset).
//!
//! The tabs need the CDR's management surface, which the composed stack's
//! `docker/ferroehr.dev.toml` enables. The page a CDR without it draws is
//! covered in process (`ssr_app`).

use crate::common;

use common::{
    Harness, confirm_in_dialog, count_matching, env, fill, is_present, login_basic,
    login_basic_as, main_text, wait_main_contains, wait_text_contains,
};
use thirtyfour::prelude::*;

/// A filter distinguishable from any boot filter the stack runs with.
const TEST_FILTER: &str = "ferroehr=debug,sqlx=warn";

/// The admin credentials the log-filter write needs.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// Health lists the CDR's readiness indicators, and Overview reports the
/// version and the spec pins.
#[tokio::test]
async fn operations_panel_reports_dependency_health_and_provenance() {
    let Some(h) = Harness::start("operations-panel").await else {
        return;
    };
    login_basic(&h).await;
    h.wait_css("nav a[href='/server']").await;

    h.goto("/server/health").await;
    let health = h.wait_css("main table tbody").await.text().await;
    for indicator in ["db", "migrations"] {
        assert!(
            health.contains(indicator),
            "Health must list the `{indicator}` indicator: {health}"
        );
    }
    assert!(
        health.contains("UP"),
        "the composed stack's database and migrations must report UP: {health}"
    );
    h.shot(1, "readiness").await;

    h.goto("/server").await;
    let build = main_text(&h).await;
    for fact in ["version", "git_sha", "rustc", "its_rest", "rm"] {
        assert!(
            build.contains(fact),
            "Overview must report `{fact}`: {build}"
        );
    }
    h.shot(2, "build-info").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

/// The metric browser: the registry fills the picker, submitting the GET form
/// puts the choice in the URL, and a `?metric=` URL renders that metric's
/// samples with the picker on it.
#[tokio::test]
async fn metric_browser_inspects_a_metric_from_the_registry() {
    let Some(h) = Harness::start("operations-metrics").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/server/metrics").await;

    let select = h.wait_css("#m-pick").await;
    let options = select.find_all(By::Css("option")).await;
    let first = options
        .first()
        .expect("the CDR's metric registry offers at least one metric")
        .attr("value")
        .await;
    assert!(!first.is_empty(), "an option must carry its metric name");

    common::submit(&h, "form[action='/server/metrics'] button[type=submit]").await;
    h.wait_url_contains(&format!("metric={first}")).await;
    wait_text_contains(&h, "main table caption", &first).await;
    assert!(
        count_matching(&h, "main table tbody tr").await > 0,
        "the samples must render at least one row for `{first}`"
    );
    h.shot(1, "metric-detail-submitted").await;

    let preferred = "db_pool_connections";
    if is_present(&h, &format!("#m-pick option[value='{preferred}']")).await {
        h.goto(&format!("/server/metrics?metric={preferred}")).await;
        wait_main_contains(&h, "state=").await;
        let selected = h
            .wait_css(&format!("#m-pick option[value='{preferred}']"))
            .await
            .selected()
            .await;
        assert!(
            selected,
            "a `?metric=` URL must pre-select that metric in the picker"
        );
        h.shot(2, "metric-detail-from-url").await;
    } else {
        println!("SKIP metric-from-url: the CDR does not register `{preferred}` on this stack");
    }

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

/// The log control round trip: apply a filter through its confirmation
/// dialog, see the CDR report it in effect, then reset to the boot filter.
/// The tab is left as it was found.
#[tokio::test]
async fn admin_applies_and_resets_the_live_log_filter() {
    let Some(h) = Harness::start("operations-log-control").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    h.goto("/server/logging").await;
    wait_main_contains(&h, "At boot").await;
    let before = h.wait_css("#lf").await.prop("value").await;

    fill(&h, "#lf", TEST_FILTER).await;
    h.shot(1, "filter-typed").await;
    confirm_in_dialog(
        &h,
        "button[commandfor='apply-log-filter']",
        "#apply-log-filter button[value='apply-filter']",
    )
    .await;
    h.wait_css("main").await;
    let applied = h.wait_css("#lf").await.prop("value").await;
    assert!(
        applied.contains("ferroehr=debug"),
        "the filter in effect is the one applied: {applied}"
    );
    h.shot(2, "filter-applied").await;

    common::submit(&h, "button[value='reset-filter']").await;
    let reset = h.wait_css("#lf").await.prop("value").await;
    assert!(
        !reset.contains("ferroehr=debug"),
        "the reset brings the boot filter back (was `{before}`, now `{reset}`)"
    );
    h.shot(3, "filter-reset").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
