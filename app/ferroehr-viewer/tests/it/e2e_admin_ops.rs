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
//! End-to-end journeys over the viewer's **admin destructive operations**:
//! template delete (from the list row and from the detail screen), stored-query
//! save + namespace grouping + delete against the CDR store, and physical EHR
//! delete — each a create → delete → assert-gone round trip through the real
//! UI.
//!
//! Every affordance is gated on the CDR advertising `/admin` in its System API
//! conformance manifest, and the deletes themselves need the ADMIN role, so
//! these journeys sign in as the quickstart ADMIN user (`UI_E2E_ADMIN_USER`/
//! `UI_E2E_ADMIN_PASS`, defaulting to `ferroehr-admin`/`ferroehr` from
//! `docker/ferroehr.dev.toml`, which also enables the admin group).
//!
//! Capability is not authorization, and the two halves are covered separately:
//! [`plain_user_is_refused_the_admin_delete`] proves that a session WITHOUT the
//! ADMIN role still sees the affordances (the group IS mounted) and gets an
//! actionable refusal instead of a silent failure; the hidden-when-not-mounted
//! half cannot be shown on this stack (the composed CDR always runs
//! admin-ENABLED) and is unit-tested in `ferroehr_viewer::admin` against a
//! manifest without `/admin`.
//!
//! Isolation: each journey creates its OWN object (a template fixture no other
//! journey uses, its own stored-query name, its own EHR), so nothing here
//! deletes data another journey or the documentation-screenshot pass depends
//! on.

use crate::common;

use std::time::Duration;

use common::{
    Harness, confirm_in_dialog, env, is_present, login_basic, login_basic_as, poll_until,
    poll_until_for, wait_css_absent,
};

/// A fixture OPT no other journey touches, and its template id — deleted from
/// the LIST row. The id carries spaces on purpose: it proves the delete path
/// segment is percent-encoded.
const LIST_FIXTURE: &str = "minimal_action2.opt";
/// The template id of [`LIST_FIXTURE`].
const LIST_TEMPLATE_ID: &str = "Minimal action 2";

/// A second fixture OPT, deleted from the DETAIL screen. It must be one no other
/// journey commits compositions under: the CDR refuses to delete a template in
/// use (a 409), so a shared fixture made the outcome depend on run order on a
/// reused stack (#3335). `minimal_instruction` is the authoring journeys'
/// template; this one is nobody else's.
const DETAIL_FIXTURE: &str = "minimal_action3.opt";
/// The template id of [`DETAIL_FIXTURE`].
const DETAIL_TEMPLATE_ID: &str = "minimal_action_3.en.v1";

/// The namespace half of the qualified stored-query name this battery saves.
/// It is typed into its OWN field and is what the viewer groups by (a query's
/// group IS its namespace).
const QUERY_NAMESPACE: &str = "org.example";
/// The bare name half, typed into the query-name field.
const QUERY_BARE_NAME: &str = "e2e-admin-delete";
/// The qualified stored-query name the two fields must compose to — the name
/// the CDR stores and the row is keyed by.
const QUERY_NAME: &str = "org.example::e2e-admin-delete";

/// The bare name of the query the VERSIONING journey stores twice (once per
/// version), kept apart from the delete journey's so the two can run in any
/// order against the same CDR.
const VERSIONED_BARE_NAME: &str = "e2e-admin-versioned";

/// Its qualified name — `{QUERY_NAMESPACE}::{VERSIONED_BARE_NAME}`.
const VERSIONED_QUERY_NAME: &str = "org.example::e2e-admin-versioned";

/// The template `scripts/ui-e2e.sh` seeds (with compositions committed against
/// it), used only by the REFUSED-delete journey — the CDR protects it both by
/// RBAC and by its `409` in-use guard.
const SHARED_TEMPLATE_ID: &str = "minimal_evaluation.en.v1";

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`) — the admin group
/// is RBAC-gated, so the destructive affordances only appear for this session.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// The absolute host path of a fixture OPT under `crates/openehr-its`, for the
/// `WebDriver` file-upload `send_keys` mechanism.
///
/// # Panics
/// When the fixture is missing (a repo-layout error, not a skip).
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

/// Land on `/templates` and guarantee `template_id` is installed, uploading
/// `fixture` through the real upload control when it is absent.
///
/// # Panics
/// On any navigation/interaction failure.
async fn ensure_template(h: &Harness, fixture: &str, template_id: &str) {
    h.goto("/templates").await;
    h.wait_css("#template-upload-open").await;
    // Let the list settle on either rows or the empty-state bar first.
    h.wait_css("a[href^='/templates/'], .thaw-message-bar")
        .await;
    let row_delete = format!("[data-template-delete=\"{template_id}\"]");
    if is_present(h, &row_delete).await {
        return;
    }
    // The file input's `on:change` is a hydrated listener, and a file set
    // BEFORE it exists is unrecoverable by retrying: the value is already
    // that path, so a re-send of the same file fires no change event
    // (#2285 — proven by the input's value never clearing, which the
    // handler always does). Wait for the shell's hydration marker so the
    // FIRST send lands on a live listener; the bounded loop stays as a
    // backstop only.
    h.wait_hydrated().await;
    for _ in 0..4 {
        common::upload_via_dialog(h, &fixture_opt_path(fixture)).await;
        if poll_until_for(Duration::from_secs(8), async || {
            is_present(h, &row_delete).await
        })
        .await
        {
            return;
        }
    }
    let evidence = h.evidence_dump("upload-exhausted").await;
    panic!(
        "`{template_id}` never appeared after uploading `{fixture}` (uploads exhausted; {evidence})"
    );
}

/// Template delete from the list row: upload a fixture OPT, delete it through
/// the confirmation modal, and the row is gone.
#[tokio::test]
async fn admin_deletes_a_template_from_the_list() {
    let Some(h) = Harness::start("admin-template-delete-list").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    ensure_template(&h, LIST_FIXTURE, LIST_TEMPLATE_ID).await;
    h.shot(1, "template-listed").await;

    let selector = format!("[data-template-delete=\"{LIST_TEMPLATE_ID}\"]");
    confirm_in_dialog(&h, &selector, "template-delete-confirm").await;
    wait_css_absent(&h, &selector).await;
    h.shot(2, "template-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Template delete from the detail screen: upload a fixture, open its detail
/// route, delete there, land back on the list without the row.
#[tokio::test]
async fn admin_deletes_a_template_from_the_detail_screen() {
    let Some(h) = Harness::start("admin-template-delete-detail").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    ensure_template(&h, DETAIL_FIXTURE, DETAIL_TEMPLATE_ID).await;
    h.goto(&format!("/templates/{DETAIL_TEMPLATE_ID}")).await;
    h.wait_css("nav[aria-label='Template views']").await;
    h.shot(1, "template-detail").await;

    confirm_in_dialog(&h, "#template-delete", "template-delete-confirm").await;
    // A successful detail delete returns to the list.
    h.wait_url_not_contains(DETAIL_TEMPLATE_ID).await;
    h.wait_css("#template-upload-open").await;
    wait_css_absent(
        &h,
        &format!("[data-template-delete=\"{DETAIL_TEMPLATE_ID}\"]"),
    )
    .await;
    h.shot(2, "template-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Stored-query save + delete: save a query through the raw-AQL editor's
/// namespace + name fields, see it listed on `/queries` under the namespace
/// card DERIVED from the qualified name it composed, then delete that version
/// from the CDR store and watch the row go.
///
/// The namespace card (`data-query-namespace`) is the assertion that the two
/// fields composed exactly the spec's `namespace::name`: the viewer creates no
/// grouping of its own, it reads the namespace back out of the stored name.
#[tokio::test]
async fn admin_saves_a_namespaced_stored_query_and_deletes_it() {
    let Some(h) = Harness::start("admin-stored-query-delete").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    // Save the query through the real editor (its Save button stays disabled
    // until the AQL and the query name hold text, which is also the hydration
    // signal — retry the typing until it enables, the login-submit precedent).
    // The namespace goes into its own field; the viewer composes the qualified
    // name from the two.
    h.goto("/queries/aql").await;
    // Save/Run dispatch is hydrated behaviour (#2285's class).
    h.wait_hydrated().await;
    let mut saved = false;
    for _ in 0..5 {
        // Clear before (re)typing: `send_keys` APPENDS, so a retry after a
        // not-yet-hydrated first pass would otherwise double every field's
        // content and save a mangled query.
        let editor = h.wait_css("#aql-editor").await;
        editor.clear().await;
        editor
            .send_keys("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c")
            .await;
        let namespace = h.wait_css("#aql-save-namespace").await;
        namespace.clear().await;
        namespace.send_keys(QUERY_NAMESPACE).await;
        let name = h.wait_css("#aql-save-name").await;
        name.clear().await;
        name.send_keys(QUERY_BARE_NAME).await;
        let save = h.wait_xpath("//button[normalize-space(.)='Save']").await;
        if poll_until_for(Duration::from_secs(1), async || save.enabled().await).await {
            save.click().await;
            saved = true;
            break;
        }
    }
    assert!(saved, "the Save button never enabled (typing never took)");
    // The dispatch is an in-flight fetch the next navigation would ABORT —
    // wait for the save's own outcome (the success toast) before leaving the
    // page, so the journey never races its own mutation. A failed save shows
    // the failure toast instead and this wait times out with a screenshot.
    h.wait_xpath("//*[contains(normalize-space(.), 'Query saved')]")
        .await;

    // The row appears on /queries with its admin delete affordance (the CDR
    // assigns the version, so match the name prefix) — which also proves the
    // two fields composed exactly `namespace::name`.
    let selector = format!("[data-query-delete^=\"{QUERY_NAME}@\"]");
    h.goto("/queries").await;
    h.wait_css(&selector).await;
    // …and the derived grouping shows the namespace it was saved under, with no
    // group having been created by hand anywhere.
    h.wait_css(&format!("[data-query-namespace=\"{QUERY_NAMESPACE}\"]"))
        .await;
    h.shot(1, "stored-query-listed").await;

    confirm_in_dialog(&h, &selector, "stored-query-delete-confirm").await;
    wait_css_absent(&h, &selector).await;
    h.shot(2, "stored-query-deleted").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Stored-query VERSIONING end to end: store an explicit version, prove the
/// `(name, version)` pair is immutable, store a second version beside it, and
/// prove that loading a version proposes the next one instead of a collision.
///
/// This is the viewer half of the spec's versioning model: a qualified name is
/// `[{namespace}::]{query-name}` and its version is SEMVER-style, with an
/// explicit `(name, version)` store being immutable (ITS-REST
/// `specifications/docs/query/Qualified_query_name.md` §Qualified query name,
/// `operations/definition_query_version_store.yaml`). Both versions are deleted
/// again, so the journey is self-cleaning like its siblings.
#[tokio::test]
async fn admin_versions_a_stored_query() {
    let Some(h) = Harness::start("admin-stored-query-versions").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    h.goto("/queries/aql").await;
    // Save/Run dispatch is hydrated behaviour (#2285's class).
    h.wait_hydrated().await;

    // Store version 1.0.0. The Save button also gates on the version being a
    // storable triple, so its enabling is the hydration + validity signal.
    let stored = save_query_version(&h, "1.0.0", true).await;
    assert!(stored, "the Save button never enabled for version 1.0.0");
    // Wait for the save's own outcome before navigating — navigation aborts
    // the in-flight server-fn fetch (same hardening as the delete journey).
    h.wait_xpath("//*[contains(normalize-space(.), 'Query saved')]")
        .await;
    let v1 = format!("[data-query-delete=\"{VERSIONED_QUERY_NAME}@1.0.0\"]");
    h.goto("/queries").await;
    h.wait_css(&v1).await;
    h.shot(1, "stored-query-v1").await;

    // Re-storing the SAME pair is refused: the pair is immutable, so the CDR
    // answers 409 and the viewer surfaces it inline (role="alert") beside the
    // editor. The listing must still hold exactly the one version.
    h.goto("/queries/aql").await;
    // Save/Run dispatch is hydrated behaviour (#2285's class).
    h.wait_hydrated().await;
    let retried = save_query_version(&h, "1.0.0", true).await;
    assert!(retried, "the Save button never enabled for the retry");
    h.wait_css("[role=\"alert\"]").await;
    h.shot(2, "stored-query-version-conflict").await;

    // A different version stores beside it rather than replacing it.
    h.goto("/queries/aql").await;
    // Save/Run dispatch is hydrated behaviour (#2285's class).
    h.wait_hydrated().await;
    let bumped = save_query_version(&h, "1.1.0", true).await;
    assert!(bumped, "the Save button never enabled for version 1.1.0");
    h.wait_xpath("//*[contains(normalize-space(.), 'Query saved')]")
        .await;
    let v2 = format!("[data-query-delete=\"{VERSIONED_QUERY_NAME}@1.1.0\"]");
    h.goto("/queries").await;
    h.wait_css(&v1).await;
    h.wait_css(&v2).await;
    h.shot(3, "stored-query-two-versions").await;

    // Loading a version into the editor proposes the NEXT version, so an edit
    // saves as a new version instead of colliding with the immutable one it
    // came from.
    h.goto(&format!("/queries/aql?load={VERSIONED_QUERY_NAME}@1.0.0"))
        .await;
    let version_field = h.wait_css("#aql-save-version").await;
    let mut proposed = String::new();
    poll_until(async || {
        proposed = version_field.prop("value").await;
        proposed == "1.1.0"
    })
    .await;
    assert_eq!(
        proposed, "1.1.0",
        "loading version 1.0.0 must propose 1.1.0 in the version field"
    );
    let name_field = h.wait_css("#aql-save-name").await;
    assert_eq!(
        name_field.prop("value").await,
        VERSIONED_BARE_NAME,
        "the loaded name must split back into the bare-name field"
    );
    h.shot(4, "stored-query-load-proposes-next-version").await;

    // Self-cleaning: both versions go.
    h.goto("/queries").await;
    confirm_in_dialog(&h, &v1, "stored-query-delete-confirm").await;
    wait_css_absent(&h, &v1).await;
    confirm_in_dialog(&h, &v2, "stored-query-delete-confirm").await;
    wait_css_absent(&h, &v2).await;
    h.shot(5, "stored-query-versions-deleted").await;

    h.assert_console_clean(&["401", "409", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Type the versioning journey's query into the raw editor at `version` and
/// click Save, retrying the typing until the button enables (the hydration
/// precedent from the sibling journeys). `fresh` clears the fields first, which
/// is what a re-visit of the screen needs.
async fn save_query_version(h: &Harness, version: &str, fresh: bool) -> bool {
    for _ in 0..5 {
        let editor = h.wait_css("#aql-editor").await;
        let namespace = h.wait_css("#aql-save-namespace").await;
        let name = h.wait_css("#aql-save-name").await;
        let version_field = h.wait_css("#aql-save-version").await;
        if fresh {
            // Load-bearing on a RETRY: without it a second typing pass appends
            // to what the first one left, producing a doubled name.
            for field in [&editor, &namespace, &name, &version_field] {
                field.clear().await;
            }
        }
        editor
            .send_keys("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c")
            .await;
        namespace.send_keys(QUERY_NAMESPACE).await;
        name.send_keys(VERSIONED_BARE_NAME).await;
        version_field.send_keys(version).await;
        let save = h.wait_xpath("//button[normalize-space(.)='Save']").await;
        if poll_until_for(Duration::from_secs(1), async || save.enabled().await).await {
            save.click().await;
            return true;
        }
    }
    false
}

/// Physical EHR delete: create an EHR through the UI, delete it from its detail
/// screen, land back on `/ehrs`, and the EHR is gone (its detail route now
/// surfaces the CDR's `404` inline).
#[tokio::test]
async fn admin_deletes_an_ehr() {
    let Some(h) = Harness::start("admin-ehr-delete").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    // Create an anonymous EHR; the viewer navigates to its detail route.
    // The create dispatch + navigation are hydrated behaviour, and a click
    // landing before hydration is silently lost (#2285's class).
    h.goto("/ehrs").await;
    h.wait_hydrated().await;
    h.wait_css("#ehr-create-submit").await.click().await;
    h.wait_url_contains("/ehrs/").await;
    let url = h.current_url().await;
    let ehr_id = url
        .rsplit('/')
        .next()
        .map(|segment| segment.split('?').next().unwrap_or(segment).to_owned())
        .filter(|id| !id.is_empty())
        .expect("the create navigated to /ehrs/{ehr_id}");
    println!("created EHR {ehr_id}");
    h.shot(1, "ehr-created").await;

    confirm_in_dialog(&h, "#ehr-delete", "ehr-delete-confirm").await;
    // A successful delete returns to the EHR list.
    h.wait_url_not_contains(&ehr_id).await;
    h.wait_css("#ehr-lookup").await;
    h.shot(2, "ehr-deleted").await;

    // Assert-gone: the detail route now renders the CDR's 404 inline.
    h.goto(&format!("/ehrs/{ehr_id}")).await;
    h.wait_css("[role='alert']").await;
    h.shot(3, "ehr-gone").await;

    // Deliberate negative step: the CDR 404 for the deleted EHR is expected.
    h.assert_console_clean(&["404", "401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// Capability vs authorization: a session without the ADMIN role still SEES the
/// affordances (the CDR advertises `/admin`, so the group exists), and the
/// refusal arrives as actionable copy on confirm — never a silent no-op, and
/// never a deleted object.
#[tokio::test]
async fn plain_user_is_refused_the_admin_delete() {
    let Some(h) = Harness::start("admin-ops-refused").await else {
        return;
    };
    login_basic(&h).await;

    // The affordance is present for this session: the group is mounted.
    h.goto("/templates").await;
    h.wait_css("[data-template-delete]").await;
    h.shot(1, "templates-delete-visible").await;

    // Confirm a delete of the shared fixture template. The CDR refuses it
    // twice over — RBAC (no ADMIN role) and, even for an admin, the committed
    // compositions that reference it — so this step cannot destroy the fixture.
    let selector = format!("[data-template-delete=\"{SHARED_TEMPLATE_ID}\"]");
    confirm_in_dialog(&h, &selector, "template-delete-confirm").await;

    // The failure is reported with actionable copy, and the row survives.
    h.wait_xpath(
        "//*[contains(@class, 'thaw-toast-body') and (contains(., 'ADMIN') or contains(., 'may not delete'))]",
    )
    .await;
    h.shot(2, "delete-refused").await;
    assert!(
        is_present(&h, &selector).await,
        "the refused delete must leave `{SHARED_TEMPLATE_ID}` in the list"
    );

    // Deliberate negative step: the CDR's refusal reaches the browser as a
    // failed server-fn call, which the browser-console gate must allow.
    h.assert_console_clean(&[
        "403",
        "Forbidden",
        "server function",
        "500",
        "Failed to load resource",
    ])
    .await;
    h.finish().await;
}
