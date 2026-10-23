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
//! End-to-end terminology journeys: the Terminology tabs and the query
//! builder's coded criterion, driven by `scripts/ui-e2e.sh`.
//! Each test owns its own [`Harness`] and skips with a printed reason when the
//! harness environment is absent, so a plain `cargo nextest run` stays green.
//!
//! The fixtures are the CDR's own compile-time openEHR bundle, so they are
//! stable on any stack that serves the terminology surface: terminology
//! `openehr`, value set `audit_change_type`, member code `249` (`creation`).

use crate::common;
use crate::e2e_browse::{TEMPLATE_ID, ensure_template_present};

use common::{Harness, fill, login_basic, wait_main_contains, wait_text_contains};

/// The terminology browser: list, describe, define a code, expand a value
/// set, test membership both ways, test subsumption, and report an unknown
/// code as an answer rather than an error.
#[tokio::test]
async fn terminology_browser_defines_terms_and_expands_value_sets() {
    let Some(h) = Harness::start("terminology-browser").await else {
        return;
    };
    login_basic(&h).await;
    h.goto("/terminology").await;
    h.wait_css("main a.fui-row-link[href='/terminology/openehr']")
        .await
        .click()
        .await;
    h.wait_url_contains("/terminology/openehr").await;
    wait_main_contains(&h, "openEHR Foundation").await;
    h.shot(1, "terminology-selected").await;

    h.wait_css("main a[href='/terminology/lookup?terminology=openehr']")
        .await
        .click()
        .await;
    fill(&h, "#l-c", "249").await;
    common::submit(&h, "form[action='/terminology/lookup'] button[type=submit]").await;
    h.wait_url_contains("code=249").await;
    wait_main_contains(&h, "creation").await;
    wait_main_contains(&h, "Preferred term").await;
    h.shot(2, "terminology-term-defined").await;

    h.goto("/terminology/valuesets?terminology=openehr").await;
    fill(&h, "#v-g", "audit_change_type").await;
    common::submit(&h, "form[action='/terminology/valuesets'] button[type=submit]").await;
    wait_main_contains(&h, "amendment").await;
    h.shot(3, "terminology-value-set").await;
    fill(&h, "#v-c", "249").await;
    common::submit(&h, "#v-c ~ * button[type=submit], form:has(#v-c) button[type=submit]").await;
    wait_main_contains(&h, "249 is a member of audit_change_type").await;
    fill(&h, "#v-c", "no-such-code-in-any-value-set").await;
    common::submit(&h, "form:has(#v-c) button[type=submit]").await;
    wait_main_contains(&h, "is not a member of").await;

    h.goto("/terminology/subsumption?terminology=openehr").await;
    fill(&h, "#s-a", "249").await;
    fill(&h, "#s-b", "250").await;
    common::submit(&h, "form[action='/terminology/subsumption'] button[type=submit]").await;
    wait_main_contains(&h, "249 is not broader than 250").await;

    h.goto("/terminology/lookup?terminology=openehr&code=no-such-code").await;
    wait_main_contains(&h, "does not define no-such-code").await;
    h.shot(4, "terminology-unknown-code").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The builder's coded criterion: a condition on a `DV_CODED_TEXT` leaf offers
/// the codes the template allows as a select, and the query then runs with
/// the code chosen.
#[tokio::test]
async fn query_builder_coded_criterion_picks_codes_from_the_terminology() {
    let Some(h) = Harness::start("terminology-code-picker").await else {
        return;
    };
    login_basic(&h).await;
    ensure_template_present(&h).await;
    h.goto(&format!("/query?template={TEMPLATE_ID}")).await;

    h.wait_css("button[name='add-condition'][value='g']")
        .await
        .click()
        .await;
    h.wait_css("select[name='g.c0-path']")
        .await
        .select_by_value("/category")
        .await;
    common::submit(&h, "button[name='intent'][value='edit']").await;
    let value = h.wait_css("select[name='g.c0-value']").await;
    value.select_by_value("433").await;
    wait_text_contains(&h, "select[name='g.c0-value']", "event · 433").await;
    h.shot(1, "builder-coded-criterion").await;

    common::submit(&h, "header.fui-page-header button[form='query-form']").await;
    wait_main_contains(&h, "433").await;
    let aql = h
        .wait_css("#query-form input[name='aql']")
        .await
        .prop("value")
        .await;
    assert!(
        aql.contains("defining_code") && aql.contains("433"),
        "the condition lowers to the code: {aql}"
    );
    h.wait_css("main table tbody tr").await;
    h.shot(2, "builder-coded-results").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
