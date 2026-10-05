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
//! End-to-end journeys over the viewer's **operations panel** (`/operations`):
//! the dependency-health card over the CDR's public readiness probe, the build +
//! spec provenance card, the metric registry browser, and the live log-filter
//! control (apply → assert effective → reset).
//!
//! The whole panel is probe-gated on the CDR serving its management surface, so
//! these journeys run against the composed stack, whose `docker/ferroehr.dev.toml`
//! enables it (`info`/`metrics` at `private`, `env`/`loggers` at `admin_only`).
//! The hidden-when-absent half cannot be shown on this stack (it always runs
//! management-ENABLED) and is unit-tested in `ferroehr_viewer::management`
//! against a probe that found no surface — the same split as the admin-group
//! journeys.
//!
//! Isolation: the log-filter journey restores the CDR's boot filter as its final
//! step, so it leaves the stack exactly as it found it for every later journey
//! and for the documentation-screenshot pass.

use crate::common;

use std::time::Duration;

use common::{
    Harness, confirm_in_dialog, count_matching, env, is_present, login_basic, login_basic_as,
    poll_until_for, wait_text_contains,
};
use thirtyfour::prelude::*;

/// The log filter the journey applies — deliberately narrow (one crate at
/// `debug`) so a busy stack does not drown in output between apply and reset.
const TEST_FILTER: &str = "ferroehr=debug,sqlx=warn";

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`): the `loggers`
/// endpoint is `admin_only`, so log control runs as this session.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// The text content of the first element matching `css`.
///
/// # Panics
/// When the element never appears or its text cannot be read.
async fn text_of(h: &Harness, css: &str) -> String {
    h.wait_css(css).await.text().await
}

/// The panel renders: the probe-gated nav entry exists, the readiness card lists
/// the CDR's indicators, and the build card reports the version + spec pins.
#[tokio::test]
async fn operations_panel_reports_dependency_health_and_provenance() {
    let Some(h) = Harness::start("operations-panel").await else {
        return;
    };
    login_basic(&h).await;

    // The nav entry is present at all: the viewer probed the CDR's management
    // surface and found it mounted (the composed stack enables it).
    h.wait_css("a[href='/operations']").await;

    h.goto("/operations").await;
    h.wait_css("footer").await;

    // Dependency health: the aggregate pill plus one row per indicator the CDR
    // evaluates. The composed stack always has a database and applied
    // migrations, so those two rows must be there and must read UP.
    let health = h.wait_css("#ops-readiness table tbody").await;
    let health_text = health.text().await;
    for indicator in ["db", "migrations"] {
        assert!(
            health_text.contains(indicator),
            "the readiness card must list the `{indicator}` indicator: {health_text}"
        );
    }
    assert!(
        health_text.contains("UP"),
        "the composed stack's database and migrations must report UP: {health_text}"
    );
    h.shot(1, "readiness").await;

    // Build + spec provenance: the CDR's own version and its openEHR pins.
    let build = text_of(&h, "#ops-build-info").await;
    for fact in ["version", "git_sha", "rustc", "its_rest", "rm"] {
        assert!(
            build.contains(fact),
            "the build card must report `{fact}`: {build}"
        );
    }
    h.shot(2, "build-info").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

/// The metric browser: the registry populates the picker, submitting the GET
/// form puts the selection in the URL, and a `?metric=` URL renders that
/// metric's current samples with the picker pre-selected — the round trip in
/// both directions.
///
/// The native select element is deliberately NOT clicked open: a `WebDriver` click on
/// an `<option>` drives Chrome's native popup and measured 120 s per attempt in
/// this headless harness (twice, reproducibly), while asserting nothing the two
/// steps below do not. The form submit is exercised with the default selection,
/// and the picked-metric path through the URL — which is the actual state
/// carrier (rules §9) — is asserted directly.
#[tokio::test]
async fn metric_browser_inspects_a_metric_from_the_registry() {
    let Some(h) = Harness::start("operations-metrics").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/operations").await;

    // The picker is populated from `GET /management/metrics`; the stack has been
    // serving requests, so the registry is never empty.
    let select = h.wait_css("#ops-metric").await;
    let options = select.find_all(By::Css("option")).await;
    assert!(
        !options.is_empty(),
        "the CDR's metric registry must offer at least one metric"
    );
    let first = options
        .first()
        .expect("at least one metric")
        .attr("value")
        .await;
    assert!(!first.is_empty(), "an option must carry its metric name");

    // ── The form round trip: submitting it lands on the URL carrying the
    //    selected metric, and the detail for that metric renders.
    h.wait_css("#ops-metric-inspect").await.click().await;
    h.wait_url_contains(&format!("metric={first}")).await;
    let detail = text_of(&h, "#ops-metric-detail").await;
    assert!(
        detail.contains(&first),
        "the detail must name the submitted metric `{first}`: {detail}"
    );
    assert!(
        count_matching(&h, "#ops-metric-detail table tbody tr").await > 0,
        "the detail must render at least one sample row for `{first}`"
    );
    h.shot(1, "metric-detail-submitted").await;

    // ── The labelled-gauge path: the DB pool gauge the background sampler
    //    feeds is reported per connection state, and a `?metric=` URL is
    //    shareable — it renders that metric with the picker pre-selected.
    let preferred = "db_pool_connections";
    if is_present(&h, &format!("#ops-metric option[value='{preferred}']")).await {
        h.goto(&format!("/operations?metric={preferred}")).await;
        let detail = text_of(&h, "#ops-metric-detail").await;
        assert!(
            detail.contains(preferred) && detail.contains("state="),
            "the DB pool gauge is reported per connection state: {detail}"
        );
        let selected = h
            .wait_css(&format!("#ops-metric option[value='{preferred}']"))
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

/// Log control round trip: apply a filter through its confirmation modal, assert
/// the CDR reports it as EFFECTIVE, then reset and assert the boot filter is
/// back. The panel is left exactly as it was found.
#[tokio::test]
async fn admin_applies_and_resets_the_live_log_filter() {
    let Some(h) = Harness::start("operations-log-control").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    h.goto("/operations").await;

    // The boot filter is the reset target; capture it before changing anything.
    let boot = text_of(&h, "#ops-log-effective").await;
    println!("boot log filter: {boot}");

    // Type the new directives. The Apply button stays disabled until the field
    // holds text, which is also the hydration signal — retry the typing until it
    // enables (the login-submit precedent).
    let mut ready = false;
    for _ in 0..10 {
        h.wait_css("#ops-log-filter")
            .await
            .send_keys(TEST_FILTER)
            .await;
        let apply = h.wait_css("#ops-log-apply").await;
        if poll_until_for(Duration::from_secs(1), async || apply.enabled().await).await {
            ready = true;
            break;
        }
    }
    assert!(ready, "the Apply button never enabled (typing never took)");
    h.shot(1, "filter-typed").await;

    // Two-step: the modal spells out the consequence before anything changes.
    confirm_in_dialog(&h, "#ops-log-apply", "ops-log-apply-confirm").await;

    // The assertion that matters: the CDR reports the new filter as effective.
    wait_text_contains(&h, "#ops-log-effective", "ferroehr=debug").await;
    h.shot(2, "filter-applied").await;

    // …and the reset restores the boot filter, so the stack is unchanged.
    confirm_in_dialog(&h, "#ops-log-reset", "ops-log-reset-confirm").await;
    wait_text_contains(&h, "#ops-log-effective", boot.trim()).await;
    h.shot(3, "filter-reset").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
