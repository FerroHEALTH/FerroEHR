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
//! End-to-end journeys over the ADL2 template family of the Template Manager
//! (`/templates?family=adl2` and `/templates/adl2/{stem}/{release}`).
//!
//! What they pin is the whole ADL2 surface the viewer consumes: the
//! `text/plain` source upload with the openEHR-ADL engine's diagnostics
//! surfaced as the CDR sent them, the listing, the stored source and
//! `OperationalTemplateV2` JSON representations, the path tree the viewer
//! builds from that JSON, the CDR-generated example composition, the release
//! picker over the versioned get, and the per-row delete over the artefact
//! resource.
//!
//! Fixtures are the repository's own authored ADL2 corpus
//! (`corpus/fixtures/adl2/opt`), read repo-relative
//! so the `WebDriver` file-upload `send_keys` receives a real host path. Every
//! scene is SEED-AND-CLEAN: it uploads what it needs (upload-if-absent, so a
//! stack a sibling scene already filled is never re-POSTed into a `409`) and
//! removes exactly those artefacts at scene end over
//! `DELETE definition/artefact/adl2/{artefact_id}`, so a shared stack is left
//! as it was found. The cleanup runs as the ADMIN dev user: that route is
//! Admin-classed by the CDR's coarse RBAC.

use crate::common;

use common::{Harness, env, is_present, login_basic_as, wait_css_absent, wait_main_contains};
use reqwest::StatusCode;

/// The versioned fixture pair: one HRID family, two release versions, so the
/// versioned get has a genuine highest match to resolve a prefix to.
const VERSIONED_V100: &str = "openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.0.0";

/// The higher member of that pair.
const VERSIONED_V110: &str = "openEHR-EHR-COMPOSITION.cnf_adl2_versioned.v1.1.0";

/// The artefact the delete scene owns — its own fixture, so removing it can
/// never race a sibling scene's reads.
const DELETABLE: &str = "openEHR-EHR-COMPOSITION.cnf_minimal_h.v1.0.0";

/// The ADL2 listing, as URL state.
const LIST_URL: &str = "/templates?family=adl2";

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`): the artefact
/// delete is Admin-classed, so both the delete scene and every scene-end
/// cleanup run as this session.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// Remove the artefacts a scene seeded, over the same route the viewer's
/// delete affordance drives (`204` deleted, `404` already gone).
///
/// Without `UI_E2E_CDR_URL` the scene has no way to reach the CDR directly and
/// says so rather than failing: the assertions above it have already run.
///
/// # Panics
/// On any answer other than `204`/`404`.
async fn remove_adl2_artefacts(hrids: &[&str]) {
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("NOTE adl2 cleanup skipped: UI_E2E_CDR_URL unset");
        return;
    };
    let http = reqwest::Client::new();
    let (user, pass) = admin_credentials();
    for hrid in hrids {
        let status = http
            .delete(format!(
                "{cdr}/ferroehr/rest/openehr/v1/definition/artefact/adl2/{hrid}"
            ))
            .basic_auth(&user, Some(&pass))
            .send()
            .await
            .expect("delete an ADL2 fixture artefact")
            .status();
        assert!(
            status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND,
            "ADL2 artefact cleanup -> {status}"
        );
    }
}

/// The absolute, canonicalized path of one fixture under the repository's
/// authored ADL2 corpus.
///
/// # Panics
/// When the fixture is missing (a repo-layout error, not a skip).
fn fixture_adl2_path(relative: &str) -> String {
    let dir = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../corpus/fixtures/adl2/opt/"
    );
    std::fs::canonicalize(format!("{dir}{relative}"))
        .expect("the ADL2 corpus fixture exists")
        .to_string_lossy()
        .into_owned()
}

/// The stem of an HRID, without its `.v{release}`.
fn stem(hrid: &str) -> &str {
    hrid.rsplit_once(".v").map_or(hrid, |(stem, _)| stem)
}

/// The release of an HRID.
fn release(hrid: &str) -> &str {
    hrid.rsplit_once(".v").map_or("", |(_, release)| release)
}

/// The listing's link to the release `hrid`.
fn row_link(hrid: &str) -> String {
    format!(
        "main a.fui-row-link[href='/templates/adl2/{}/{}']",
        stem(hrid),
        release(hrid)
    )
}

/// Make sure `hrid` is in the CDR's ADL2 store, uploading `relative` through
/// Upload a template only when the listing does not show it. Returns whether
/// this call uploaded it.
async fn ensure_adl2_template_present(h: &Harness, relative: &str, hrid: &str) -> bool {
    h.goto(LIST_URL).await;
    if is_present(h, &row_link(hrid)).await {
        return false;
    }
    common::upload_template(h, "adl2", &fixture_adl2_path(relative)).await;
    h.goto(LIST_URL).await;
    h.wait_css(&row_link(hrid)).await;
    true
}

/// Uploading an ADL2 source lists it under ADL 2, and its page serves the
/// stored source, the `OperationalTemplateV2` JSON and the path tree the
/// viewer builds from that JSON.
#[tokio::test]
async fn adl2_upload_lists_and_serves_source_and_json() {
    let Some(h) = Harness::start("adl2-upload").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    remove_adl2_artefacts(&[VERSIONED_V100, VERSIONED_V110]).await;
    h.goto("/templates").await;
    h.wait_css("a.fui-tab[data-tab='adl2']").await.click().await;
    h.wait_url_contains("family=adl2").await;
    let uploaded = ensure_adl2_template_present(&h, "versioned.v1_0_0.adls", VERSIONED_V100).await;
    assert!(uploaded, "the scene uploads its own fixture");
    h.shot(1, "adl2-listed").await;

    let page = format!("/templates/adl2/{}", stem(VERSIONED_V100));
    h.goto(&format!("{page}?tab=source")).await;
    wait_main_contains(&h, "operational_template").await;
    wait_main_contains(&h, VERSIONED_V100).await;
    h.shot(2, "adl2-source").await;

    h.goto(&format!("{page}?tab=json")).await;
    wait_main_contains(&h, "OPERATIONAL_TEMPLATE").await;
    wait_main_contains(&h, "\"release_version\": \"1.0.0\"").await;
    h.shot(3, "adl2-json").await;

    h.goto(&format!("{page}?tab=paths")).await;
    h.wait_css("ul.fui-tree[role='tree'] li[role='treeitem']")
        .await;
    h.shot(4, "adl2-paths").await;

    remove_adl2_artefacts(&[VERSIONED_V100]).await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Both releases of one HRID family are listed, the latest's page picks
/// either release, and an older release names the latest to go back to.
#[tokio::test]
async fn adl2_versioned_get_reaches_both_stored_versions() {
    let Some(h) = Harness::start("adl2-versions").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    ensure_adl2_template_present(&h, "versioned.v1_0_0.adls", VERSIONED_V100).await;
    ensure_adl2_template_present(&h, "versioned.v1_1_0.adls", VERSIONED_V110).await;
    h.goto(LIST_URL).await;
    h.wait_css(&row_link(VERSIONED_V100)).await;
    h.wait_css(&row_link(VERSIONED_V110)).await;
    h.shot(1, "adl2-both-versions-listed").await;

    let page = format!("/templates/adl2/{}", stem(VERSIONED_V100));
    h.goto(&page).await;
    wait_main_contains(&h, VERSIONED_V110).await;
    h.wait_css("#ver").await.select_by_value("1.0.0").await;
    common::submit(&h, "form.fui-title-select button[type=submit]").await;
    h.wait_url_contains("1.0.0").await;
    wait_main_contains(&h, "You are reading release 1.0.0").await;
    wait_main_contains(&h, VERSIONED_V100).await;
    h.shot(2, "adl2-release-1-0-0").await;

    h.wait_css(&format!("main a[href='{page}']"))
        .await
        .click()
        .await;
    wait_main_contains(&h, VERSIONED_V110).await;
    h.shot(3, "adl2-latest-again").await;

    remove_adl2_artefacts(&[VERSIONED_V100, VERSIONED_V110]).await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The Example tab renders the CDR-generated example COMPOSITION for an ADL2
/// template, in the format chosen.
#[tokio::test]
async fn adl2_example_composition_renders() {
    let Some(h) = Harness::start("adl2-example").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    ensure_adl2_template_present(&h, "versioned.v1_0_0.adls", VERSIONED_V100).await;
    let page = format!("/templates/adl2/{}?tab=example", stem(VERSIONED_V100));
    h.goto(&page).await;
    wait_main_contains(&h, "COMPOSITION").await;
    h.shot(1, "adl2-example-json").await;
    h.wait_css("#x-fmt").await.select_by_value("xml").await;
    common::submit(&h, "form:has(#x-fmt) button[type=submit]").await;
    h.wait_url_contains("format=xml").await;
    wait_main_contains(&h, "<composition").await;
    h.shot(2, "adl2-example-xml").await;

    remove_adl2_artefacts(&[VERSIONED_V100]).await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// An unparseable ADL2 source is refused, the openEHR-ADL engine's
/// diagnostic reaches the reader beside the source, and nothing is listed.
#[tokio::test]
async fn an_unparseable_adl2_source_surfaces_the_engine_diagnostic() {
    let Some(h) = Harness::start("adl2-refusal").await else {
        return;
    };
    common::login_basic(&h).await;
    common::upload_template(&h, "adl2", &fixture_adl2_path("invalid/unparseable.adls")).await;
    h.wait_url_contains("/templates/upload").await;
    wait_main_contains(&h, "syntactically invalid ADL2 content").await;
    wait_main_contains(&h, "missing terminology section").await;
    h.shot(1, "adl2-refusal").await;

    h.goto(LIST_URL).await;
    assert!(
        !is_present(
            &h,
            &row_link("openEHR-EHR-COMPOSITION.cnf_unparseable.v1.0.0")
        )
        .await,
        "a refused ADL2 source must not appear in the listing"
    );
    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}

/// The ADL2 rows carry Delete… over the artefact resource: the row opens the
/// release's page with the confirmation inline, and the row goes.
#[tokio::test]
async fn an_adl2_artefact_is_deleted_from_the_listing() {
    let Some(h) = Harness::start("adl2-delete").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    ensure_adl2_template_present(&h, "minimal_h.adls", DELETABLE).await;
    h.shot(1, "adl2-delete-listed").await;

    h.wait_xpath(&format!(
        "//main//tr[.//a[contains(@href, '{}')]]//summary",
        stem(DELETABLE)
    ))
    .await
    .click()
    .await;
    h.wait_css(&format!(
        "main a.fui-menu-danger[href*='{}']",
        stem(DELETABLE)
    ))
    .await
    .click()
    .await;
    h.wait_url_contains("confirm=delete").await;
    common::submit(&h, "main form.fui-cluster button.fui-button-danger").await;
    h.wait_css(".fui-toast-text").await;
    h.goto(LIST_URL).await;
    wait_css_absent(&h, &row_link(DELETABLE)).await;
    h.shot(2, "adl2-delete-gone").await;

    remove_adl2_artefacts(&[DELETABLE]).await;
    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
