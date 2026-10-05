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
//! (`/templates?family=adl2` and `/templates/adl2/{template_id}`).
//!
//! What they pin is the whole ADL2 surface the viewer consumes: the
//! `text/plain` source upload with the openEHR-ADL engine's diagnostics
//! surfaced verbatim, the listing, the stored SOURCE and `OperationalTemplateV2`
//! JSON representations, the viewer-built path catalog over that JSON, the
//! CDR-generated example composition, and the versioned get — both an exact
//! release version and the wire's `{major}` prefix resolution, driven through
//! the screen's own version bar.
//!
//! …and, since #2568, the per-row DELETE the ADL2 rows now carry over the
//! artefact resource.
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

use common::{
    Harness, confirm_in_dialog, env, is_present, login_basic_as, poll_until, retype,
    wait_css_absent, wait_text, wait_text_contains,
};
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

/// The detail-route anchor the ADL2 listing renders for `hrid`.
fn row_link(hrid: &str) -> String {
    format!("a[href='/templates/adl2/{hrid}']")
}

/// Guarantee `hrid` is in the CDR's ADL2 store: land on the listing and upload
/// `relative` only when its row is absent.
///
/// The upload goes through the screen's one dialog
/// ([`common::upload_via_dialog`]), the same control the ADL 1.4 family drives
/// — including its post-condition on the send, so a click that dispatches
/// nothing fails at the click rather than here (#3200).
///
/// Returns `true` when this call performed the upload.
///
/// # Panics
/// When the row never appears after a dispatched upload — which, the send now
/// being post-conditioned, means the CDR accepted nothing or the listing never
/// refetched.
async fn ensure_adl2_template_present(h: &Harness, relative: &str, hrid: &str) -> bool {
    h.goto(LIST_URL).await;
    // Let the listing Transition resolve — to a row link, the rendered table,
    // the empty state, or an inline error — before deciding whether an upload
    // is needed.
    h.wait_css("a[href^='/templates/adl2/'], table, .border-dashed, [role='alert']")
        .await;
    let link = row_link(hrid);
    if is_present(h, &link).await {
        return false;
    }
    common::upload_via_dialog(h, &fixture_adl2_path(relative)).await;
    if poll_until(async || is_present(h, &link).await).await {
        return true;
    }
    let evidence = h.evidence_dump("adl2-upload-never-listed").await;
    panic!("`{hrid}` never appeared in the ADL2 listing after the upload ({evidence})");
}

/// Uploading an ADL2 source lists it under the ADL2 family, and its detail
/// screen serves both stored representations — the artefact SOURCE verbatim and
/// the `OperationalTemplateV2` canonical JSON — plus the path catalog the
/// viewer builds from that JSON.
#[tokio::test]
async fn adl2_upload_lists_and_serves_source_and_json() {
    let Some(h) = Harness::start("adl2-detail").await else {
        return;
    };
    common::login_basic(&h).await;

    // The family switch is URL state: the default screen is ADL 1.4, and the
    // ADL 2 pill navigates to the family this journey drives.
    h.goto("/templates").await;
    h.wait_css("a[data-template-family='adl2']")
        .await
        .click()
        .await;
    h.wait_url_contains("family=adl2").await;
    h.shot(1, "adl2-family-selected").await;

    let uploaded = ensure_adl2_template_present(&h, "versioned.v1_0_0.adls", VERSIONED_V100).await;
    println!("this journey performed the v1.0.0 upload: {uploaded}");
    h.shot(2, "adl2-listed").await;

    h.wait_css(&row_link(VERSIONED_V100)).await.click().await;
    h.wait_url_contains("/templates/adl2/").await;

    // The Source pane is the default: the stored artefact, verbatim, with its
    // HRID on the artefact-identifier line.
    wait_text_contains(&h, "#adl2-source-pane", "operational_template").await;
    wait_text_contains(&h, "#adl2-source-pane", VERSIONED_V100).await;
    h.shot(3, "adl2-source-pane").await;

    // The AOM2 JSON pane serves the OperationalTemplateV2 projection. The
    // pretty-printed spelling (`"key": value`) only exists when the viewer
    // PARSED the body — the wire sends it unformatted.
    h.wait_css("a[data-adl2-tab='json']").await.click().await;
    h.wait_url_contains("tab=json").await;
    wait_text_contains(&h, "#adl2-json-pane", "OPERATIONAL_TEMPLATE").await;
    wait_text_contains(&h, "#adl2-json-pane", "\"release_version\": \"1.0.0\"").await;
    h.shot(4, "adl2-json-pane").await;

    // The path catalog is built by the viewer from that same JSON, and it
    // renders with the ADL 1.4 screen's own tree + inspector components: a tree
    // node exists only once the AOM2 parse and the WebTemplate build both
    // succeeded. The standing "no path catalog" note is gone with it.
    h.wait_css("a[data-adl2-tab='catalog']").await.click().await;
    h.wait_url_contains("tab=catalog").await;
    h.wait_css("#adl2-catalog-pane ul.text-sm li button").await;
    wait_text_contains(&h, "#adl2-catalog-pane", "Node inspector").await;
    assert!(
        !is_present(&h, "#adl2-no-catalog").await,
        "the ADL2 screen no longer claims it has no path catalog"
    );
    h.shot(5, "adl2-catalog-pane").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    remove_adl2_artefacts(&[VERSIONED_V100]).await;
    h.finish().await;
}

/// The version bar reaches every stored version of one HRID family through the
/// versioned get: an exact release version by chip, the wire's `{major}` prefix
/// by the free-text box, and back to the artefact the route names.
#[tokio::test]
async fn adl2_versioned_get_reaches_both_stored_versions() {
    let Some(h) = Harness::start("adl2-versions").await else {
        return;
    };
    common::login_basic(&h).await;
    ensure_adl2_template_present(&h, "versioned.v1_0_0.adls", VERSIONED_V100).await;
    ensure_adl2_template_present(&h, "versioned.v1_1_0.adls", VERSIONED_V110).await;

    // Both versions are separate rows in the listing: the viewer asks for the
    // FULL inventory (`?version=*`), not the latest of each family.
    h.wait_css(&row_link(VERSIONED_V100)).await;
    h.wait_css(&row_link(VERSIONED_V110)).await;
    h.shot(1, "adl2-both-versions-listed").await;

    // Open the 1.0.0 artefact; its source is the one the route names.
    h.goto(&format!("/templates/adl2/{VERSIONED_V100}")).await;
    wait_text_contains(&h, "#adl2-source-pane", VERSIONED_V100).await;

    // The version bar offers what the CDR holds. Pinning 1.1.0 drives the
    // versioned get from the SAME route id, and the pane follows.
    h.wait_css("a[data-adl2-version='1.1.0']")
        .await
        .click()
        .await;
    h.wait_url_contains("version=1.1.0").await;
    wait_text_contains(&h, "#adl2-source-pane", VERSIONED_V110).await;
    h.shot(2, "adl2-version-1-1-0").await;

    // "As stored" clears the pin and returns to the artefact the route names.
    h.wait_css("a[data-adl2-version='stored']")
        .await
        .click()
        .await;
    h.wait_url_not_contains("version=").await;
    wait_text_contains(&h, "#adl2-source-pane", VERSIONED_V100).await;

    // A bare `1` is a MAJOR prefix on the wire: it resolves to the HIGHEST
    // match, so the pane must move off the 1.0.0 it is currently showing.
    // The moved pane is the assertion — a `?version=1` substring would also
    // match `?version=1.1.0` and could pass without a navigation.
    retype(&h, "#adl2-version-input", "1").await;
    h.wait_css("#adl2-version-apply").await.click().await;
    wait_text_contains(&h, "#adl2-source-pane", VERSIONED_V110).await;

    // A `1.0` minor prefix resolves the other way, back off 1.1.0.
    retype(&h, "#adl2-version-input", "1.0").await;
    h.wait_css("#adl2-version-apply").await.click().await;
    wait_text_contains(&h, "#adl2-source-pane", VERSIONED_V100).await;
    h.shot(3, "adl2-version-prefix").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    remove_adl2_artefacts(&[VERSIONED_V100, VERSIONED_V110]).await;
    h.finish().await;
}

/// The example pane renders the CDR-generated example COMPOSITION for an ADL2
/// template, in the representation the format selector picks.
#[tokio::test]
async fn adl2_example_composition_renders() {
    let Some(h) = Harness::start("adl2-example").await else {
        return;
    };
    common::login_basic(&h).await;
    ensure_adl2_template_present(&h, "versioned.v1_0_0.adls", VERSIONED_V100).await;

    h.goto(&format!("/templates/adl2/{VERSIONED_V100}?tab=example"))
        .await;
    // Canonical JSON is the default representation the example resource
    // negotiates; the generated composition names the template it came from.
    wait_text_contains(&h, "#adl2-example-pane", "COMPOSITION").await;
    wait_text_contains(&h, "#adl2-example-pane", VERSIONED_V100).await;
    h.shot(1, "adl2-example-json").await;

    // The same resource in canonical XML — a real content negotiation, not a
    // client-side re-render.
    h.wait_xpath("//button[normalize-space(text())='XML']")
        .await
        .click()
        .await;
    wait_text_contains(&h, "#adl2-example-pane", "<composition").await;
    h.shot(2, "adl2-example-xml").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    remove_adl2_artefacts(&[VERSIONED_V100]).await;
    h.finish().await;
}

/// An unparseable ADL2 source is refused, and the openEHR-ADL engine's
/// diagnostic reaches the reader VERBATIM — inline beside the failure toast,
/// exactly as the ADL 1.4 upload does.
#[tokio::test]
async fn an_unparseable_adl2_source_surfaces_the_engine_diagnostic() {
    let Some(h) = Harness::start("adl2-refusal").await else {
        return;
    };
    common::login_basic(&h).await;
    h.goto(LIST_URL).await;
    h.wait_css("#template-upload-open").await;

    common::upload_via_dialog(&h, &fixture_adl2_path("invalid/unparseable.adls")).await;

    // The engine reports AOM2 rule codes with their positions; both the code
    // and its message must survive the trip to the screen unedited.
    wait_text_contains(
        &h,
        ".thaw-message-bar",
        "syntactically invalid ADL2 content",
    )
    .await;
    wait_text_contains(&h, ".thaw-message-bar", "missing terminology section").await;
    wait_text_contains(&h, ".thaw-message-bar", "SAON").await;
    // The failure ALSO toasts — an inline-only refusal reads as "nothing
    // happened" (the viewer's mutation-feedback rule).
    assert!(
        wait_text(&h, "Upload failed").await,
        "a refused upload must toast as well as render the diagnostic inline"
    );
    h.shot(1, "adl2-refusal").await;

    // Nothing was stored: the refused artefact has no row.
    assert!(
        !is_present(
            &h,
            &row_link("openEHR-EHR-COMPOSITION.cnf_unparseable.v1.0.0")
        )
        .await,
        "a refused ADL2 source must not appear in the listing"
    );

    // The 400 the CDR answered the server fn with is the point of this journey.
    h.assert_console_clean(&["400", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The ADL2 rows carry the same two-step delete the ADL 1.4 rows do, over the
/// artefact resource: seed an artefact, remove it through the confirmation
/// dialog, and its row is gone from the listing.
#[tokio::test]
async fn an_adl2_artefact_is_deleted_from_the_listing() {
    let Some(h) = Harness::start("adl2-delete").await else {
        return;
    };
    // The artefact delete is Admin-classed by the CDR's coarse RBAC, so this
    // scene drives it as the admin dev user — the delete BUTTON renders for any
    // session while the admin group is mounted (capability is not
    // authorization), but only this one may use it.
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    ensure_adl2_template_present(&h, "minimal_h.adls", DELETABLE).await;
    h.shot(1, "adl2-delete-listed").await;

    let trigger = format!("[data-template-delete=\"{DELETABLE}\"]");
    confirm_in_dialog(&h, &trigger, "template-delete-confirm").await;

    // Gone from the listing, both as a row and as its delete trigger — the
    // list refetches on the action's version, so this is the CDR's own answer.
    wait_css_absent(&h, &trigger).await;
    wait_css_absent(&h, &row_link(DELETABLE)).await;
    assert!(
        wait_text(&h, "Template deleted").await,
        "a successful delete must toast"
    );
    h.shot(2, "adl2-delete-gone").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    remove_adl2_artefacts(&[DELETABLE]).await;
    h.finish().await;
}
