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
//! End-to-end journeys over the destructive operations: deleting a template
//! from its row and from its page, saving, versioning and deleting stored
//! queries, deleting an EHR, and the refusal a plain user meets.
//!
//! Each delete confirms first: a row's Delete… opens the object's page with
//! the confirmation inline (`?confirm=delete`), and a page's Delete… opens a
//! `<dialog>` by the browser's own command. Both post a plain form.

use crate::common;

use common::{
    Harness, confirm_in_dialog, env, fill, is_present, login_basic, login_basic_as,
    wait_css_absent, wait_main_contains,
};

/// The fixture OPT the list-row delete removes, and the template id it
/// declares (the SDK fixture uses its concept as its id).
const LIST_FIXTURE: &str = "minimal_action2.opt";
const LIST_TEMPLATE_ID: &str = "Minimal action 2";

/// The fixture OPT the detail-page delete removes.
const DETAIL_FIXTURE: &str = "minimal_action3.opt";
const DETAIL_TEMPLATE_ID: &str = "minimal_action_3.en.v1";

/// The stored query the save-and-delete journey writes.
const QUERY_NAMESPACE: &str = "org.example";
const QUERY_BARE_NAME: &str = "e2e-admin-delete";
const QUERY_NAME: &str = "org.example::e2e-admin-delete";

/// The stored query the versioning journey writes twice.
const VERSIONED_BARE_NAME: &str = "e2e-admin-versioned";
const VERSIONED_QUERY_NAME: &str = "org.example::e2e-admin-versioned";

/// The AQL every saved query holds.
const AQL: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";

/// The template the seeded compositions use, which the plain user tries to
/// delete.
const SHARED_TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`).
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// The absolute path of an SDK fixture OPT.
fn fixture_opt_path(name: &str) -> String {
    let raw = format!(
        "{}/../../crates/openehr-its/tests/fixtures/sdk/{name}",
        env!("CARGO_MANIFEST_DIR")
    );
    std::fs::canonicalize(&raw)
        .unwrap_or_else(|e| panic!("fixture OPT {raw} exists: {e}"))
        .to_string_lossy()
        .into_owned()
}

/// The list's link to the template `id`.
fn template_link(id: &str) -> String {
    format!(
        "main a.fui-row-link[href='/templates/{}']",
        urlencoding::encode(id)
    )
}

/// Make sure the template `id` is listed, uploading `fixture` when it is not.
async fn ensure_template(h: &Harness, fixture: &str, id: &str) {
    h.goto("/templates").await;
    if is_present(h, &template_link(id)).await {
        return;
    }
    common::upload_template(h, "adl14", &fixture_opt_path(fixture)).await;
    h.goto("/templates").await;
    h.wait_css(&template_link(id)).await;
}

/// Save the query on Query's AQL tab under `name` in [`QUERY_NAMESPACE`],
/// at `version` (empty for the version the CDR assigns).
async fn save_query(h: &Harness, name: &str, version: &str) {
    h.goto("/query/aql").await;
    fill(h, "#aql", AQL).await;
    h.wait_css("button[commandfor='save-query']")
        .await
        .click()
        .await;
    common::wait_visible(h, "#sv-n").await;
    fill(h, "#sv-ns", QUERY_NAMESPACE).await;
    fill(h, "#sv-n", name).await;
    fill(h, "#sv-v", version).await;
    common::submit(h, "#save-query button[value='save']").await;
}

/// The list row of the stored query `name` at `version`.
fn stored_link(name: &str, version: &str) -> String {
    format!(
        "main a.fui-row-link[href='/queries/{}/{version}']",
        urlencoding::encode(name)
    )
}

/// Delete one stored-query version from its page.
async fn delete_stored(h: &Harness, name: &str, version: &str) {
    h.goto(&format!("/queries/{}/{version}", urlencoding::encode(name)))
        .await;
    h.wait_css("summary[aria-controls='stored-more']")
        .await
        .click()
        .await;
    confirm_in_dialog(
        h,
        "button[commandfor='delete-stored-query']",
        "#delete-stored-query button[type=submit]",
    )
    .await;
    h.wait_url_contains("/queries").await;
}

/// A template deleted from its list row: Delete… opens the template's page
/// with the confirmation inline, and confirming removes the row.
#[tokio::test]
async fn admin_deletes_a_template_from_the_list() {
    let Some(h) = Harness::start("admin-template-delete-list").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    ensure_template(&h, LIST_FIXTURE, LIST_TEMPLATE_ID).await;

    let confirm = format!(
        "main a.fui-menu-danger[href='/templates/{}?confirm=delete']",
        urlencoding::encode(LIST_TEMPLATE_ID)
    );
    let row = h
        .wait_xpath(&format!(
            "//main//tr[.//a[@class='fui-row-link'][normalize-space(.)='{LIST_TEMPLATE_ID}']]//summary"
        ))
        .await;
    row.click().await;
    h.wait_css(&confirm).await.click().await;
    h.wait_url_contains("confirm=delete").await;
    h.shot(1, "template-delete-confirm").await;
    common::submit(&h, "main form.fui-cluster button.fui-button-danger").await;
    h.wait_url_contains("/templates").await;
    wait_css_absent(&h, &template_link(LIST_TEMPLATE_ID)).await;
    h.shot(2, "template-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// A template deleted from its own page through the dialog lands back on the
/// list without it.
#[tokio::test]
async fn admin_deletes_a_template_from_the_detail_screen() {
    let Some(h) = Harness::start("admin-template-delete-detail").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    ensure_template(&h, DETAIL_FIXTURE, DETAIL_TEMPLATE_ID).await;
    h.goto(&format!("/templates/{DETAIL_TEMPLATE_ID}")).await;
    h.wait_css("summary[aria-controls='template-more']")
        .await
        .click()
        .await;
    h.shot(1, "template-detail").await;
    confirm_in_dialog(
        &h,
        "button[commandfor='delete-template']",
        "#delete-template button[type=submit]",
    )
    .await;
    h.wait_url_not_contains(DETAIL_TEMPLATE_ID).await;
    wait_css_absent(&h, &template_link(DETAIL_TEMPLATE_ID)).await;
    h.shot(2, "template-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// A query saved with a namespace is listed under that namespace, and its
/// version can be deleted from the CDR.
#[tokio::test]
async fn admin_saves_a_namespaced_stored_query_and_deletes_it() {
    let Some(h) = Harness::start("admin-stored-query-delete").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    save_query(&h, QUERY_BARE_NAME, "").await;
    // A save lands back on the AQL tab with the stored query open.
    h.wait_url_contains("name=org.example").await;
    h.wait_css(".fui-toast-text").await;

    h.goto("/queries").await;
    let row = h
        .wait_xpath(&format!(
            "//main//a[@class='fui-row-link'][starts-with(@href, '/queries/{}/')]",
            urlencoding::encode(QUERY_NAME)
        ))
        .await;
    wait_main_contains(&h, QUERY_NAMESPACE).await;
    h.shot(1, "stored-query-listed").await;
    let href = row.attr("href").await;
    let version = href
        .rsplit('/')
        .next()
        .expect("a version segment")
        .to_owned();

    delete_stored(&h, QUERY_NAME, &version).await;
    wait_css_absent(&h, &stored_link(QUERY_NAME, &version)).await;
    h.shot(2, "stored-query-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Stored-query versions: an explicit version stores, the same `(name,
/// version)` again is refused, a second version stores beside the first, and
/// opening a version proposes the next one in Save.
#[tokio::test]
async fn admin_versions_a_stored_query() {
    let Some(h) = Harness::start("admin-stored-query-versions").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    save_query(&h, VERSIONED_BARE_NAME, "1.0.0").await;
    h.wait_url_contains("version=1.0.0").await;
    h.goto("/queries").await;
    h.wait_css(&stored_link(VERSIONED_QUERY_NAME, "1.0.0"))
        .await;
    h.shot(1, "stored-query-v1").await;

    // The same pair again: the CDR refuses, and the popover says why.
    save_query(&h, VERSIONED_BARE_NAME, "1.0.0").await;
    wait_main_contains(&h, "already").await;
    h.shot(2, "stored-query-version-conflict").await;

    save_query(&h, VERSIONED_BARE_NAME, "1.1.0").await;
    h.wait_url_contains("version=1.1.0").await;
    h.goto("/queries").await;
    h.wait_css(&stored_link(VERSIONED_QUERY_NAME, "1.0.0"))
        .await;
    h.wait_css(&stored_link(VERSIONED_QUERY_NAME, "1.1.0"))
        .await;
    h.shot(3, "stored-query-two-versions").await;

    // Opening 1.0.0 in Query proposes 1.1.0, with the name split back.
    h.goto(&format!(
        "/query/aql?name={}&version=1.0.0",
        urlencoding::encode(VERSIONED_QUERY_NAME)
    ))
    .await;
    assert_eq!(h.wait_css("#sv-v").await.prop("value").await, "1.1.0");
    assert_eq!(
        h.wait_css("#sv-n").await.prop("value").await,
        VERSIONED_BARE_NAME
    );
    assert_eq!(
        h.wait_css("#sv-ns").await.prop("value").await,
        QUERY_NAMESPACE
    );
    h.shot(4, "stored-query-load-proposes-next-version").await;

    for version in ["1.0.0", "1.1.0"] {
        delete_stored(&h, VERSIONED_QUERY_NAME, version).await;
        wait_css_absent(&h, &stored_link(VERSIONED_QUERY_NAME, version)).await;
    }
    h.shot(5, "stored-query-versions-deleted").await;

    h.assert_console_clean(&["401", "409", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// An EHR created in the viewer and deleted from its page: the delete asks for
/// the first eight characters of its id, lands on `/ehrs`, and the EHR's page
/// then shows the CDR's `404`.
#[tokio::test]
async fn admin_deletes_an_ehr() {
    let Some(h) = Harness::start("admin-ehr-delete").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto("/ehrs/new").await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_contains("/ehrs/").await;
    let url = h.current_url().await;
    let ehr_id = url
        .split("/ehrs/")
        .nth(1)
        .map(|rest| rest.split(['?', '/']).next().unwrap_or(rest).to_owned())
        .filter(|id| id.len() > 8)
        .expect("the create landed on /ehrs/{ehr_id}");
    println!("created EHR {ehr_id}");
    h.shot(1, "ehr-created").await;

    h.wait_css("summary[aria-controls='ehr-more']")
        .await
        .click()
        .await;
    h.wait_css("button[commandfor='delete-ehr']")
        .await
        .click()
        .await;
    common::wait_visible(&h, "#delete-confirm").await;
    fill(
        &h,
        "#delete-confirm",
        ehr_id
            .get(..8)
            .expect("an id of more than eight characters"),
    )
    .await;
    common::submit(&h, "#delete-ehr button[type=submit]").await;
    h.wait_url_not_contains(&ehr_id).await;
    h.shot(2, "ehr-deleted").await;

    h.goto(&format!("/ehrs/{ehr_id}")).await;
    wait_main_contains(&h, "404").await;
    h.shot(3, "ehr-gone").await;

    h.assert_console_clean(&["404", "401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// A session without the ADMIN role still sees Delete… (the CDR advertises
/// `/admin`), and the refusal arrives as the CDR's words on confirm: never a
/// silent no-op, and never a deleted object.
#[tokio::test]
async fn plain_user_is_refused_the_admin_delete() {
    let Some(h) = Harness::start("admin-ops-refused").await else {
        return;
    };
    login_basic(&h).await;

    h.goto(&format!("/templates/{SHARED_TEMPLATE_ID}?confirm=delete"))
        .await;
    h.shot(1, "template-delete-confirm").await;
    common::submit(&h, "main form.fui-cluster button.fui-button-danger").await;
    wait_main_contains(&h, "ADMIN").await;
    h.shot(2, "delete-refused").await;

    h.goto("/templates").await;
    assert!(
        is_present(&h, &template_link(SHARED_TEMPLATE_ID)).await,
        "the refused delete must leave `{SHARED_TEMPLATE_ID}` in the list"
    );

    h.assert_console_clean(&["403", "Failed to load resource"])
        .await;
    h.finish().await;
}
