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
//! End-to-end journeys over the seeded EHR + two-version composition
//! (`scripts/ui-e2e.sh` seeds them over REST): the EHR detail tabs, the
//! composition viewer's format toggle + version history, and the no-JS
//! progressive-enhancement contracts (Basic login, the EHR finder).

use crate::common;

use common::{
    Harness, appears_within, env, is_present, login_basic, poll_until, wait_text_contains,
};
use thirtyfour::prelude::*;

/// The seeded ids, exported by the harness; `None` skips with a reason.
fn seeded() -> Option<(String, String)> {
    if let (Some(ehr), Some(vo)) = (env("UI_E2E_SEEDED_EHR_ID"), env("UI_E2E_SEEDED_VO_ID")) {
        Some((ehr, vo))
    } else {
        println!("SKIP: UI_E2E_SEEDED_EHR_ID/UI_E2E_SEEDED_VO_ID unset (run scripts/ui-e2e.sh)");
        None
    }
}

/// The EHR detail screen renders the seeded EHR: the status tab shows the
/// queryable badge, and the compositions tab lists the seeded composition
/// whose link opens the viewer — in its RENDERED clinical reading, which is
/// what the row's `?view=rendered` deep link asks for.
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
    // Status tab (default): the EHR_STATUS document resolved.
    h.wait_xpath("//*[contains(., 'queryable')]").await;
    h.shot(1, "status-tab").await;

    // Compositions tab: the seeded row links to the viewer.
    h.wait_xpath("//a[contains(., 'Compositions')]")
        .await
        .click()
        .await;
    let link = h.wait_css(&format!("a[href*='{vo_id}']")).await;
    h.shot(2, "compositions-tab").await;
    link.click().await;
    h.wait_url_contains(&format!("/compositions/{vo_id}")).await;
    // The row deep-links the pane's mode, so the document arrives as the
    // clinical reading (label/value rows) rather than the raw text pane.
    h.wait_url_contains("view=rendered").await;
    h.wait_css("[data-doc-row]").await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The composition viewer round-trips all four representations and walks
/// the two-version history: JSON carries `_type`, XML carries the openEHR
/// namespace, FLAT carries flat keys, and switching to version 1 re-renders.
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
    h.goto(&format!("/ehrs/{ehr_id}/compositions/{vo_id}"))
        .await;

    // Canonical JSON (the default): the document pane carries `_type`.
    wait_pre_contains(&h, "\"_type\"").await;
    h.shot(1, "canonical-json").await;

    // Canonical XML: the openEHR namespace appears.
    click_format(&h, "XML").await;
    wait_pre_contains(&h, "schemas.openehr.org").await;
    h.shot(2, "canonical-xml").await;

    // FLAT: the ctx/ key vocabulary is unique to the simplified format —
    // the canonical JSON/XML panes can never match it, so the wait proves
    // the swap actually happened (the template-id string would match the
    // canonical JSON still on screen).
    click_format(&h, "FLAT").await;
    wait_pre_contains(&h, "ctx/language").await;
    h.shot(3, "flat").await;

    // Version history: two versions; selecting v1 re-renders the pane.
    let select = h.wait_css("select").await;
    let options = select.find_all(By::Tag("option")).await;
    assert!(
        options.len() >= 2,
        "the seeded composition has two versions (got {})",
        options.len()
    );
    // Choose the oldest (last listed) version.
    options.last().expect("a version option").click().await;
    click_format(&h, "JSON").await;
    // `::1` is version 1's OBJECT_VERSION_ID suffix — absent from both the
    // FLAT pane still on screen and the version-2 document.
    wait_pre_contains(&h, "::1\"").await;
    h.shot(4, "version-1-json").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The document viewer: a composition opens **highlighted** (pure-Rust syntax
/// tokens, so the pane carries `syntax-*` design-token spans while the text
/// stays byte-exact), the **Rendered** tab shows the template-free clinical
/// reading as label/value rows, and **Raw** drops the tokens again. A copy
/// affordance is present on the pane in every mode.
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
    h.goto(&format!("/ehrs/{ehr_id}/compositions/{vo_id}"))
        .await;

    // Highlighted is the default view: the canonical JSON member names are
    // tokenized, and the document text itself is unchanged.
    wait_pre_contains(&h, "\"_type\"").await;
    let key = h.wait_css("pre span.text-syntax-key").await;
    let key_text = key.text().await;
    assert!(
        key_text.starts_with('"'),
        "a JSON key token carries the quoted member name (got `{key_text}`)"
    );
    // The copy affordance rides along on every document pane.
    h.wait_xpath("//button[contains(., 'Copy')]").await;
    h.shot(1, "highlighted").await;

    // Rendered: the template-free clinical view, with at least one
    // label/value row (the seeded EVALUATION's leaves, plus the composition's
    // own composer/template facts). The tab's `on:click` only wires at
    // hydration, so a single early click can be lost on the inert SSR button —
    // re-click until the rendered rows appear (the save-button retry
    // precedent).
    let mut rendered = false;
    for _ in 0..10 {
        h.wait_xpath("//button[contains(., 'Rendered')]")
            .await
            .click()
            .await;
        if appears_within(&h, "[data-doc-row]", std::time::Duration::from_secs(2)).await {
            rendered = true;
            break;
        }
    }
    assert!(rendered, "the Rendered tab never took effect");
    let row = h.wait_css("[data-doc-row]").await;
    let row_text = row.text().await;
    assert!(
        !row_text.trim().is_empty(),
        "a rendered row shows a label and a value"
    );
    h.shot(2, "rendered").await;

    // Raw: the same text with no syntax tokens at all.
    h.wait_xpath("//button[contains(., 'Raw')]")
        .await
        .click()
        .await;
    wait_pre_contains(&h, "\"_type\"").await;
    assert!(
        !is_present(&h, "pre span.text-syntax-key").await,
        "the raw view must not tokenize the document"
    );
    h.shot(3, "raw").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The viewer's **versioned-object card** reads the `VERSIONED_COMPOSITION`
/// container and the SELECTED version's envelope directly (the direct VERSION
/// read), so switching the version selector changes the envelope facts: the
/// seeded composition's second version names the first as its preceding
/// version, and its first version names none.
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
    h.goto(&format!("/ehrs/{ehr_id}/compositions/{vo_id}"))
        .await;

    // The container's own facts: its uid is the versioned-object id in the
    // route, and its owner is this EHR.
    wait_fact_contains(&h, "object-uid", &vo_id).await;
    wait_fact_contains(&h, "owner", &ehr_id).await;
    // "Latest" is version 2 of the seeded composition, whose envelope names
    // version 1 as its preceding version and still carries data.
    wait_fact_contains(&h, "version", "::2").await;
    wait_fact_contains(&h, "preceding", "::1").await;
    wait_fact_contains(&h, "content", "present").await;
    h.shot(1, "versioned-object-latest").await;

    // Selecting the oldest version re-reads the VERSION directly: version 1 has
    // no preceding version at all.
    let select = h.wait_css("#version-select").await;
    let options = select.find_all(By::Tag("option")).await;
    options.last().expect("a version option").click().await;
    wait_fact_contains(&h, "version", "::1").await;
    wait_fact_contains(&h, "preceding", "—").await;
    h.shot(2, "versioned-object-version-1").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Progressive enhancement: with JavaScript disabled the Basic login (an
/// `ActionForm` — a plain HTML form pre-hydration) still authenticates via
/// a full-page POST + redirect, and the SSR'd dashboard renders.
#[tokio::test]
async fn login_works_with_javascript_disabled() {
    let Some(h) = Harness::start_without_javascript("no-js").await else {
        return;
    };
    let user = env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned());
    let pass = env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned());
    h.goto("/login").await;
    h.wait_css("#login-username").await.send_keys(&user).await;
    h.wait_css("#login-password").await.send_keys(&pass).await;
    h.wait_css("button[type=submit]").await.click().await;
    h.wait_url_not_contains("/login").await;
    // The dashboard streams out-of-order, so without JS the authenticated
    // chrome arrives as inert <template> fragments rather than live DOM
    // (Leptos book, ssr/23: out-of-order "requires JavaScript to be
    // enabled"). The auth proof is the redirect plus the server having
    // rendered the authenticated shell into the response at all — an
    // unauthenticated request is bounced back to /login instead.
    if poll_until(async || h.page_source().await.contains("<footer")).await {
        h.shot(1, "dashboard-no-js").await;
        h.finish().await;
        return;
    }
    panic!("the authenticated shell never appeared in the no-JS response");
}

/// Progressive enhancement: with JavaScript disabled the EHR finder's by-id
/// lookup still works. Typing an EHR id and clicking **Find** submits the
/// plain `GET` form natively, the server answers `/ehrs?find=<id>` with a
/// redirect, and the browser lands on that EHR's detail route — no WASM, no
/// listener, no client router. The same URL is then requested directly, which
/// is the shareable-shortcut case.
///
/// The authenticated routes render `SsrMode::Async` (one complete document, no
/// streamed `<template>` fragments), which is what makes the SSR'd form real,
/// clickable DOM without JavaScript.
#[tokio::test]
async fn ehr_finder_by_id_works_with_javascript_disabled() {
    let Some(h) = Harness::start_without_javascript("no-js-ehr-finder").await else {
        return;
    };
    let Some((ehr_id, _)) = seeded() else {
        h.finish().await;
        return;
    };
    let user = env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned());
    let pass = env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned());
    h.goto("/login").await;
    h.wait_css("#login-username").await.send_keys(&user).await;
    h.wait_css("#login-password").await.send_keys(&pass).await;
    h.wait_css("button[type=submit]").await.click().await;
    h.wait_url_not_contains("/login").await;

    // Fill the SSR'd finder and submit it natively — no JavaScript involved.
    h.goto("/ehrs").await;
    h.wait_css("#ehr-lookup").await.send_keys(&ehr_id).await;
    h.shot(1, "ehr-finder-no-js").await;
    h.wait_css("#ehr-find").await.click().await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}")).await;
    h.shot(2, "ehr-finder-no-js-redirected").await;

    // The same request as a shareable shortcut: /ehrs?find=<id> is a link
    // anyone can paste, and it redirects the same way.
    h.goto(&format!("/ehrs?find={ehr_id}")).await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}")).await;
    h.shot(3, "ehr-finder-no-js-shortcut").await;
    h.finish().await;
}

/// Poll one row of the versioned-object card (`data-versioned-fact="{hook}"`)
/// until its text contains `needle`.
///
/// # Panics
/// When it never does, reporting what the row said instead.
async fn wait_fact_contains(h: &Harness, hook: &str, needle: &str) {
    wait_text_contains(h, &format!("[data-versioned-fact='{hook}']"), needle).await;
}

/// Poll the first `<pre>` until it contains `needle`.
///
/// # Panics
/// When it never does, reporting what the pane said instead.
async fn wait_pre_contains(h: &Harness, needle: &str) {
    wait_text_contains(h, "pre", needle).await;
}

/// Click a format-selector button by its label.
async fn click_format(h: &Harness, label: &str) {
    h.wait_xpath(&format!("//button[contains(., '{label}')]"))
        .await
        .click()
        .await;
}
