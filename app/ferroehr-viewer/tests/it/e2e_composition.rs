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
//! End-to-end journeys over the seeded EHR and its composition: the EHR's
//! page, the composition's Reading, Document and Versions tabs, and the
//! sign-in and find paths with JavaScript disabled.
//!
//! The seeded composition has two versions (`scripts/ui-e2e.sh` commits it
//! and updates it once).

use crate::common;

use common::{Harness, env, login_basic, wait_main_contains, wait_text_contains};

/// The seeded EHR and composition, or `None` (with a printed skip) when the
/// harness did not seed them.
fn seeded() -> Option<(String, String)> {
    if let (Some(ehr), Some(vo)) = (env("UI_E2E_SEEDED_EHR_ID"), env("UI_E2E_SEEDED_VO_ID")) {
        Some((ehr, vo))
    } else {
        println!("SKIP: UI_E2E_SEEDED_EHR_ID/UI_E2E_SEEDED_VO_ID unset (run scripts/ui-e2e.sh)");
        None
    }
}

/// The document panel of the Document tab.
const DOCUMENT: &str = "#composition-document-text";

/// The page of the composition `vo_id` in the EHR `ehr_id`.
fn composition_path(ehr_id: &str, vo_id: &str) -> String {
    format!("/ehrs/{ehr_id}/compositions/{vo_id}")
}

/// Sign in through the Basic form of a session with no JavaScript.
async fn sign_in_without_javascript(h: &Harness) {
    let user = env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned());
    let pass = env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned());
    h.goto("/login").await;
    h.wait_css("#sign-in-username").await.send_keys(&user).await;
    h.wait_css("#sign-in-password").await.send_keys(&pass).await;
    h.wait_css("main button[type=submit]").await.click().await;
    h.wait_url_not_contains("/login").await;
    h.wait_css(common::SHELL_FOOTER).await;
}

/// The EHR's page shows its status badges and lists the seeded composition,
/// whose row opens the composition on its Reading tab.
#[tokio::test]
async fn ehr_detail_lists_seeded_composition() {
    let Some(h) = Harness::start("ehr-detail").await else {
        return;
    };
    let Some((ehr_id, vo_id)) = seeded() else {
        h.finish().await;
        return;
    };
    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}")).await;
    wait_main_contains(&h, "Queryable").await;
    h.shot(1, "compositions-tab").await;

    h.wait_css(&format!("main a.fui-row-link[href*='{vo_id}']"))
        .await
        .click()
        .await;
    h.wait_url_contains(&format!("/compositions/{vo_id}")).await;
    h.wait_css("a.fui-tab[data-tab='reading'][aria-current='page']")
        .await;
    h.wait_css("h2.fui-reading-title").await;
    h.shot(2, "composition-reading").await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The Document tab shows the four representations the CDR converts to, and
/// `?version=1` reads the first version.
#[tokio::test]
async fn composition_viewer_switches_formats_and_versions() {
    let Some(h) = Harness::start("composition-viewer").await else {
        return;
    };
    let Some((ehr_id, vo_id)) = seeded() else {
        h.finish().await;
        return;
    };
    login_basic(&h).await;
    let page = composition_path(&ehr_id, &vo_id);
    h.goto(&format!("{page}?tab=document")).await;
    wait_text_contains(&h, DOCUMENT, "\"_type\"").await;
    h.shot(1, "canonical-json").await;

    for (label, needle, slug) in [
        ("XML", "schemas.openehr.org", "canonical-xml"),
        ("FLAT", "ctx/language", "flat"),
        ("STRUCTURED", "ctx", "structured"),
    ] {
        h.wait_xpath(&format!(
            "//main//a[contains(@class, 'fui-segment')][normalize-space(.)='{label}']"
        ))
        .await
        .click()
        .await;
        h.wait_url_contains(&format!("format={}", label.to_lowercase()))
            .await;
        wait_text_contains(&h, DOCUMENT, needle).await;
        h.shot(2, slug).await;
    }

    h.goto(&format!("{page}?tab=document&format=json&version=1"))
        .await;
    wait_text_contains(&h, DOCUMENT, "::1\"").await;
    h.shot(3, "version-1-json").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The document is highlighted (FerroUI's token spans, the text unchanged),
/// the hydrated Copy button appears, and Reading shows the clinical reading as
/// label and value rows.
#[tokio::test]
async fn composition_viewer_highlights_and_renders_the_document() {
    let Some(h) = Harness::start("composition-document-viewer").await else {
        return;
    };
    let Some((ehr_id, vo_id)) = seeded() else {
        h.finish().await;
        return;
    };
    login_basic(&h).await;
    let page = composition_path(&ehr_id, &vo_id);
    h.goto(&format!("{page}?tab=document")).await;
    let key = h
        .wait_css(&format!("{DOCUMENT} span.fui-tok-key"))
        .await
        .text()
        .await;
    assert!(
        key.starts_with('"'),
        "a JSON key token carries the quoted member name (got `{key}`)"
    );
    common::wait_visible(&h, "#composition-document").await;
    h.shot(1, "highlighted").await;

    h.goto(&page).await;
    wait_main_contains(&h, "Example composer").await;
    wait_main_contains(&h, "2022-02-03T04:05:06Z").await;
    h.shot(2, "rendered").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The Versions tab reads the `VERSIONED_COMPOSITION` and the chosen
/// version's envelope: the latest names version 1 as its preceding version,
/// and version 1 has none.
#[tokio::test]
async fn composition_viewer_reads_versioned_object_and_version_envelope() {
    let Some(h) = Harness::start("composition-versioned-object").await else {
        return;
    };
    let Some((ehr_id, vo_id)) = seeded() else {
        h.finish().await;
        return;
    };
    login_basic(&h).await;
    let page = composition_path(&ehr_id, &vo_id);
    h.goto(&format!("{page}?tab=versions")).await;
    wait_main_contains(&h, "The versioned composition").await;
    wait_main_contains(&h, "Version 2").await;
    wait_main_contains(&h, "Preceding version").await;
    h.wait_css("main a.fui-row-link[href$='?version=1']").await;
    h.shot(1, "versioned-object-latest").await;

    h.goto(&format!("{page}?tab=versions&version=1")).await;
    wait_main_contains(&h, "None: this is the first version").await;
    h.shot(2, "versioned-object-version-1").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// With JavaScript disabled the Basic sign-in is a plain form post and the
/// dashboard renders.
#[tokio::test]
async fn login_works_with_javascript_disabled() {
    let Some(h) = Harness::start_without_javascript("no-js").await else {
        return;
    };
    sign_in_without_javascript(&h).await;
    h.wait_css("main a.fui-stat").await;
    h.shot(1, "dashboard-no-js").await;
    h.finish().await;
}

/// With JavaScript disabled the top bar's find is a plain form post: an EHR
/// id finds its EHR, and the match opens it.
#[tokio::test]
async fn ehr_finder_by_id_works_with_javascript_disabled() {
    let Some(h) = Harness::start_without_javascript("no-js-ehr-finder").await else {
        return;
    };
    let Some((ehr_id, _)) = seeded() else {
        h.finish().await;
        return;
    };
    sign_in_without_javascript(&h).await;

    h.goto("/ehrs").await;
    h.wait_css("#find").await.send_keys(&ehr_id).await;
    h.shot(1, "ehr-finder-no-js").await;
    common::submit(&h, "form[action='/find'] button[type=submit]").await;
    wait_main_contains(&h, "1 record matches").await;
    h.wait_css(&format!("main a[href='/ehrs/{ehr_id}']"))
        .await
        .click()
        .await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}")).await;
    wait_main_contains(&h, "Queryable").await;
    h.shot(2, "ehr-finder-no-js-opened").await;
    h.finish().await;
}
