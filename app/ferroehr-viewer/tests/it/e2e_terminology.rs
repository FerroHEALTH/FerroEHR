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
//! End-to-end terminology journeys — the `/terminology` browser and the query
//! builder's value-set-backed code picker — driven by `scripts/ui-e2e.sh`.
//! Each test owns its own [`Harness`] and skips with a printed reason when the
//! harness environment is absent, so a plain `cargo nextest run` stays green.
//!
//! The fixtures are the CDR's own compile-time openEHR bundle, so they are
//! stable on any stack that serves the terminology surface: terminology
//! `openehr`, value set `audit_change_type`, member code `249` (`creation`).

use crate::common;
use crate::e2e_browse::{TEMPLATE_ID, ensure_template_present};

use common::{Harness, click_until_css, is_visible, login_basic, retype, wait_text_contains};

/// The viewer origin's terminology screen, with `openehr` already selected.
const OPENEHR_ROW: &str = "[data-terminology-id='openehr']";

/// Refuse to run against a CDR that serves the terminology routes as if
/// unmounted: the screen is then honestly one empty-state card, and there is
/// nothing for this battery to drive.
///
/// A loud failure rather than a skip on purpose — the e2e stack is supposed to
/// enable the surface, so a disabled one is a harness defect to fix, not a
/// condition to tolerate.
async fn assert_terminology_enabled(h: &Harness) {
    assert!(
        !is_visible(h, "#terminology-disabled").await,
        "the CDR under test runs with [terminology] api_enabled = false — the e2e stack must \
         enable it (docker/viewer/e2e-env.yml, FERROEHR__TERMINOLOGY__API_ENABLED)"
    );
}

/// The terminology browser: list, describe, define a code, expand a value set,
/// test membership both ways, test subsumption, and report an unknown code
/// inline rather than as an error.
#[tokio::test]
async fn terminology_browser_defines_terms_and_expands_value_sets() {
    let Some(h) = Harness::start("terminology-browser").await else {
        return;
    };
    login_basic(&h).await;

    h.goto("/terminology").await;
    assert_terminology_enabled(&h).await;

    // Selecting a terminology is a plain link, so the choice lands in the URL.
    h.wait_css(OPENEHR_ROW).await.click().await;
    h.wait_url_contains("terminology=openehr").await;
    wait_text_contains(&h, "#terminology-descriptor", "openEHR Foundation").await;
    h.shot(1, "terminology-selected").await;

    // Define a term: `249` is `creation` in the openEHR bundle.
    retype(&h, "#terminology-code", "249").await;
    h.wait_css("#terminology-term-lookup").await.click().await;
    h.wait_css("[data-extract='term'] [data-term-code='249']")
        .await;
    wait_text_contains(&h, "[data-extract='term']", "249 — creation").await;
    h.shot(2, "terminology-term-defined").await;

    // Expand a value set and prove the code is one of its members.
    retype(&h, "#terminology-value-set-id", "audit_change_type").await;
    h.wait_css("#terminology-value-set-expand")
        .await
        .click()
        .await;
    h.wait_css("[data-extract='value-set-member'] [data-term-code='249']")
        .await;
    h.shot(3, "terminology-value-set").await;

    // Membership, both verdicts.
    retype(&h, "#terminology-value-set-candidate", "249").await;
    h.wait_css("#terminology-value-set-validate")
        .await
        .click()
        .await;
    wait_text_contains(&h, "#terminology-value-set-verdict", "is a member of").await;
    retype(
        &h,
        "#terminology-value-set-candidate",
        "no-such-code-in-any-value-set",
    )
    .await;
    h.wait_css("#terminology-value-set-validate")
        .await
        .click()
        .await;
    wait_text_contains(&h, "#terminology-value-set-verdict", "is not a member of").await;

    // Subsumption: the openEHR vocabulary is flat and the test is strict, so
    // the honest verdict for any pair is "does not subsume".
    retype(&h, "#terminology-subsumes-ref", "249").await;
    retype(&h, "#terminology-subsumes-candidate", "250").await;
    h.wait_css("#terminology-subsumes-run").await.click().await;
    wait_text_contains(&h, "#terminology-subsumes-verdict", "does not subsume").await;

    // An unknown code is an inline note on the card that asked, never an error
    // bar and never a toast.
    retype(&h, "#terminology-code", "no-such-code").await;
    h.wait_css("#terminology-term-lookup").await.click().await;
    h.wait_css("#terminology-term-absent").await;
    h.shot(4, "terminology-unknown-code").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The query builder's coded criterion: the terminology comes from the
/// datalist, the code is added through a lookup that renders `code — text`, a
/// value set contributes a second code, and the query then RUNS.
#[tokio::test]
async fn query_builder_coded_criterion_picks_codes_from_the_terminology() {
    let Some(h) = Harness::start("terminology-code-picker").await else {
        return;
    };
    login_basic(&h).await;
    // The builder needs the template present; seed it if a sibling journey has
    // not (each test is independent and order is not guaranteed).
    ensure_template_present(&h).await;

    // The disabled-extension check runs on the browser screen, which states it
    // plainly — the builder's datalist would just be silently empty.
    h.goto("/terminology").await;
    assert_terminology_enabled(&h).await;

    h.goto("/queries/builder").await;
    // Selecting the <option> both selects it and fires change; the bounded
    // re-click covers a click landing before hydration attaches the listener.
    let picked = click_until_css(
        &h,
        &format!("#qb-template option[value='{TEMPLATE_ID}']"),
        "ul.text-sm li",
    )
    .await;
    assert!(
        picked,
        "the template selection never took (pre-hydration clicks exhausted)"
    );

    // The terminology datalist is populated from the CDR's own list.
    h.wait_css("#qb-terminology-options option[value='openehr']")
        .await;

    // Add a condition on the template's DV_CODED_TEXT leaf. The criterion lands
    // as the root group's first child, so its editor ids end `-0`.
    h.wait_clickable_xpath(
        "//div[span[contains(., 'DV_CODED_TEXT')]]/button[contains(., '+ condition')]",
    )
    .await
    .click()
    .await;
    h.wait_css("#qb-coded-terminology-0").await;
    h.shot(1, "builder-coded-criterion").await;

    // Name the terminology the datalist offers, then add a code through the
    // lookup: the chip reads `code — text` while the model keeps the bare code.
    retype(&h, "#qb-coded-terminology-0", "openehr").await;
    retype(&h, "#qb-coded-code-0", "249").await;
    h.wait_css("#qb-coded-lookup-0").await.click().await;
    wait_text_contains(&h, "[data-coded-chip='249']", "249 — creation").await;

    // A second code, this time picked out of a value set's members.
    retype(&h, "#qb-coded-value-set-0", "audit_change_type").await;
    h.wait_css("#qb-coded-expand-0").await.click().await;
    h.wait_css("[data-value-set-code='250']")
        .await
        .click()
        .await;
    wait_text_contains(&h, "[data-coded-chip='250']", "250 — amendment").await;
    h.shot(2, "builder-coded-chips").await;

    // The criterion lowers to runnable AQL over the picked codes, and it runs.
    // The first `<pre>` on the screen is the live AQL preview.
    wait_text_contains(&h, "pre", "MATCHES").await;
    h.wait_clickable_xpath("//button[normalize-space(.)='Run']")
        .await
        .click()
        .await;
    // contains(., …): leptos interleaves hydration comment markers with text
    // nodes, so text()= comparisons are unreliable.
    h.wait_xpath("//div[contains(., 'Results')] | //p[contains(., 'No rows')]")
        .await;
    h.shot(3, "builder-coded-results").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
