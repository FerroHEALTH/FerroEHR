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
#![expect(
    clippy::disallowed_types,
    reason = "test fixtures and wire assertions are raw JSON by the testing rule \
              (.claude/rules/testing.md §Test-fixture construction)"
)]
//! End-to-end journeys over the viewer's **event subscriptions**
//! (`/subscriptions`): the CRUD round trip, and the CDR's duplicate-name
//! refusal surfacing verbatim beside the failure toast.
//!
//! The screen is probe-gated on the CDR serving its event-subscription admin
//! API, which the composed E2E stack enables (`docker/viewer/e2e-env.yml`
//! sets `FERROEHR__EVENTS__ADMIN_API`); a stack without it hides the nav entry
//! entirely, and these journeys say so in their failure message rather than
//! passing vacuously. The hidden-when-absent half is unit-tested in
//! `ferroehr_viewer::subscriptions` against a probe that found no surface —
//! the same split as the admin-group and management journeys.
//!
//! The group is mounted under `/admin`, so the CDR's coarse RBAC classes every
//! call here as admin work: every scene signs in as the ADMIN dev user.
//!
//! Isolation: every fixture subscription is removed over the admin API before
//! AND after the scene that owns it, so a shared stack is left exactly as it
//! was found.

use crate::common;

use reqwest::StatusCode;

use common::{
    Harness, clear_field, confirm_in_dialog, env, is_present, login_basic_as, poll_until,
    read_enabled, retype, wait_css_absent, wait_enabled, wait_text, wait_text_contains,
};

/// The subscription the CRUD round trip owns.
const ROUND_TRIP_SUBSCRIPTION: &str = "e2e-viewer-subscription";

/// The subscription the conflict scene seeds, then tries to create again.
const CONFLICT_SUBSCRIPTION: &str = "e2e-viewer-duplicate";

/// Every fixture subscription this file may leave behind.
const FIXTURE_SUBSCRIPTIONS: [&str; 2] = [ROUND_TRIP_SUBSCRIPTION, CONFLICT_SUBSCRIPTION];

/// The kind predicate the round trip creates with.
const CREATED_KIND: &str = "COMPOSITION";

/// The kind predicate the edit stores over it.
const EDITED_KIND: &str = "EHR_STATUS";

/// The audit change-type predicate the round trip creates with (the CDR's
/// change-type group code for a creation).
const CREATED_CHANGE_TYPE: &str = "249";

/// The template predicate the round trip creates with — cleared by the edit, to
/// prove a cleared field really becomes the CDR's wildcard.
const CREATED_TEMPLATE: &str = "minimal_evaluation.en.v1";

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`): the group sits
/// under `/admin`, so the RBAC gate classes every fixture call as admin work.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// The subscription group's base URL on the CDR under test.
fn group_url(cdr: &str) -> String {
    format!("{cdr}/ferroehr/rest/openehr/v1/admin/event_subscription")
}

/// Remove every fixture subscription (absent = nothing to do), so a scene both
/// starts and ends from a known state.
///
/// # Panics
/// On any answer other than `200` to the listing, or `204`/`404` to a delete.
async fn remove_fixture_subscriptions(cdr: &str) {
    let http = reqwest::Client::new();
    let (user, pass) = admin_credentials();
    let response = http
        .get(group_url(cdr))
        .basic_auth(&user, Some(&pass))
        .send()
        .await
        .expect("list the event subscriptions");
    assert_eq!(
        response.status(),
        StatusCode::OK,
        "the event-subscription admin API must be served for these journeys to mean anything — \
         set FERROEHR__EVENTS__ADMIN_API=true on the composed `ferroehr` service"
    );
    let rows = response
        .json::<serde_json::Value>()
        .await
        .expect("the group answers a JSON array");
    let ids: Vec<String> = rows
        .as_array()
        .map(|rows| {
            rows.iter()
                .filter(|row| {
                    row.get("name")
                        .and_then(serde_json::Value::as_str)
                        .is_some_and(|name| FIXTURE_SUBSCRIPTIONS.contains(&name))
                })
                .filter_map(|row| row.get("id").and_then(serde_json::Value::as_str))
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    for id in ids {
        let status = http
            .delete(format!("{}/{id}", group_url(cdr)))
            .basic_auth(&user, Some(&pass))
            .send()
            .await
            .expect("delete a fixture subscription")
            .status();
        assert!(
            status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND,
            "subscription cleanup -> {status}"
        );
    }
}

/// Create one fixture subscription over the admin API (test setup deliberately
/// bypasses the UI, whose create path has its own scene).
///
/// # Panics
/// When the CDR refuses the create.
async fn seed_subscription(cdr: &str, name: &str) {
    let (user, pass) = admin_credentials();
    let status = reqwest::Client::new()
        .post(group_url(cdr))
        .basic_auth(&user, Some(&pass))
        .json(&serde_json::json!({ "name": name, "kind": CREATED_KIND }))
        .send()
        .await
        .expect("seed a fixture subscription")
        .status();
    assert_eq!(status, StatusCode::CREATED, "subscription seed -> {status}");
}

/// Land on `/subscriptions` with the group actually served, or fail naming the
/// switch that turns it on.
///
/// # Panics
/// When the CDR under test runs with the event-subscription admin API disabled
/// — the screen then renders its disabled card and every assertion below would
/// be vacuous.
async fn open_subscriptions(h: &Harness) {
    // The nav entry is the probe's own verdict: it renders only when the CDR
    // answered the list probe with something other than a 404.
    h.wait_css("a[href='/subscriptions']").await;
    h.goto("/subscriptions").await;
    h.wait_css("#subscriptions-screen").await;
    assert!(
        !is_present(h, "#subscriptions-disabled").await,
        "the CDR under test runs with the event-subscription admin API disabled — set \
         FERROEHR__EVENTS__ADMIN_API=true on the composed `ferroehr` service"
    );
}

/// Poll until the control at `css` is present and DISABLED — the inert-form
/// condition, the mirror of [`wait_enabled`].
///
/// A poll rather than an immediate read: the property that carries the live
/// disabled state is applied by a render effect, which is queued, so an
/// assertion fired the instant typing returns can beat it.
///
/// # Panics
/// When it never becomes disabled within 15 s.
async fn wait_disabled(h: &Harness, css: &str) {
    assert!(
        poll_until(async || read_enabled(h, css).await == Some(false)).await,
        "`{css}` never went inert"
    );
}

/// The CSS selector of one row's predicate cell.
fn row_cell(name: &str, cell: &str) -> String {
    format!("tr[data-subscription='{name}'] [data-subscription-cell='{cell}']")
}

/// Fill the create card and send it, then assert the row the CDR answered
/// with — its predicates, its wildcard, its summary, its state, and the toast.
///
/// The submit is inert until the name can be accepted, so `wait_enabled` IS the
/// "the form is complete" condition — never a sleep.
///
/// # Panics
/// On any interaction failure, or when the row never appears as the CDR stored
/// it.
async fn create_and_assert_row(h: &Harness) {
    retype(h, "#subscription-create-name", ROUND_TRIP_SUBSCRIPTION).await;
    retype(h, "#subscription-create-kind", CREATED_KIND).await;
    retype(h, "#subscription-create-change-type", CREATED_CHANGE_TYPE).await;
    retype(h, "#subscription-create-template", CREATED_TEMPLATE).await;
    wait_enabled(h, "#subscription-create-submit").await;
    h.wait_css("#subscription-create-submit")
        .await
        .click()
        .await;

    // The row is the CDR's answer, not the form's: the listing refetches on the
    // action's version and renders what the CDR now holds.
    wait_text_contains(h, &row_cell(ROUND_TRIP_SUBSCRIPTION, "kind"), CREATED_KIND).await;
    wait_text_contains(
        h,
        &row_cell(ROUND_TRIP_SUBSCRIPTION, "change-type"),
        CREATED_CHANGE_TYPE,
    )
    .await;
    wait_text_contains(
        h,
        &row_cell(ROUND_TRIP_SUBSCRIPTION, "template"),
        CREATED_TEMPLATE,
    )
    .await;
    // A predicate never set reads as the wildcard, not as an empty cell.
    // …and the plain-words summary says what the row selects.
    wait_text_contains(
        h,
        &row_cell(ROUND_TRIP_SUBSCRIPTION, "summary"),
        "Matches kind COMPOSITION",
    )
    .await;
    h.wait_css(&format!(
        "tr[data-subscription='{ROUND_TRIP_SUBSCRIPTION}'] [data-subscription-state='true']"
    ))
    .await;
    assert!(
        wait_text(h, "Subscription created").await,
        "a successful create must toast"
    );
}

/// Open the editor on the round-trip row, change its kind, CLEAR its template
/// and disable it, save, and assert the CDR stored exactly that.
///
/// Clearing a predicate is the case that proves the update replaces the whole
/// set: an emptied field must come back as the CDR's wildcard, not as the value
/// it held before.
///
/// # Panics
/// On any interaction failure, or when the row never catches up.
async fn edit_and_assert_row(h: &Harness) {
    h.wait_toasts_cleared().await;
    h.wait_css(&format!(
        "[data-subscription-edit='{ROUND_TRIP_SUBSCRIPTION}']"
    ))
    .await
    .click()
    .await;
    h.wait_css("#subscription-edit").await;
    // The editor came up holding the row's stored values.
    assert_eq!(
        h.wait_css("#subscription-edit-kind")
            .await
            .prop("value")
            .await,
        CREATED_KIND,
        "the editor must be seeded from the row it opened on"
    );
    retype(h, "#subscription-edit-kind", EDITED_KIND).await;
    clear_field(h, "#subscription-edit-template").await;
    h.wait_css("#subscription-edit-enabled").await.click().await;
    h.wait_css("#subscription-edit-save").await.click().await;

    wait_text_contains(h, &row_cell(ROUND_TRIP_SUBSCRIPTION, "kind"), EDITED_KIND).await;
    // The cleared template really became the wildcard on the CDR.
    wait_text_contains(h, &row_cell(ROUND_TRIP_SUBSCRIPTION, "template"), "any").await;
    h.wait_css(&format!(
        "tr[data-subscription='{ROUND_TRIP_SUBSCRIPTION}'] [data-subscription-state='false']"
    ))
    .await;
    // The editor closes on success, so the screen is back to its resting state.
    wait_css_absent(h, "#subscription-edit").await;
    assert!(
        wait_text(h, "Subscription updated").await,
        "a successful update must toast"
    );
}

/// The CRUD round trip: create a subscription, see its row, edit its predicates
/// (including clearing one and disabling it), then delete it through the
/// confirmation dialog and watch the row go.
#[tokio::test]
async fn the_viewer_creates_edits_and_deletes_a_subscription() {
    let Some(h) = Harness::start("event-subscriptions").await else {
        return;
    };
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("SKIP event-subscriptions: fixture cleanup needs UI_E2E_CDR_URL");
        h.finish().await;
        return;
    };
    remove_fixture_subscriptions(&cdr).await;
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    open_subscriptions(&h).await;

    create_and_assert_row(&h).await;
    h.shot(1, "subscription-created").await;

    edit_and_assert_row(&h).await;
    h.shot(2, "subscription-edited").await;

    // Delete: two steps — the row's button opens the shared confirmation
    // dialog, and only the dialog's own button dispatches.
    confirm_in_dialog(
        &h,
        &format!("[data-subscription-delete='{ROUND_TRIP_SUBSCRIPTION}']"),
        "subscription-delete-confirm",
    )
    .await;
    wait_css_absent(
        &h,
        &format!("tr[data-subscription='{ROUND_TRIP_SUBSCRIPTION}']"),
    )
    .await;
    assert!(
        wait_text(&h, "Subscription deleted").await,
        "a successful delete must toast"
    );
    h.shot(3, "subscription-deleted").await;

    h.assert_console_clean(&["Failed to load resource"]).await;
    remove_fixture_subscriptions(&cdr).await;
    h.finish().await;
}

/// A name the CDR cannot accept never leaves the browser (the create button
/// stays inert), and a name it CAN accept but already holds is refused with its
/// `409` diagnostic reaching the reader VERBATIM — inline beside the failure
/// toast.
#[tokio::test]
async fn a_refused_subscription_name_never_leaves_or_reaches_the_reader_verbatim() {
    let Some(h) = Harness::start("event-subscription-refusals").await else {
        return;
    };
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("SKIP event-subscription-refusals: fixture seeding needs UI_E2E_CDR_URL");
        h.finish().await;
        return;
    };
    remove_fixture_subscriptions(&cdr).await;
    seed_subscription(&cdr, CONFLICT_SUBSCRIPTION).await;
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    open_subscriptions(&h).await;

    // A name outside the CDR's `[A-Za-z0-9_.-]` rule keeps the submit inert:
    // the viewer refuses it before any round trip.
    retype(&h, "#subscription-create-name", "not a valid name").await;
    wait_disabled(&h, "#subscription-create-submit").await;
    h.shot(1, "subscription-name-inert").await;

    // A valid name the CDR already holds: the button enables, the create is
    // sent, and the CDR's own words come back unedited.
    retype(&h, "#subscription-create-name", CONFLICT_SUBSCRIPTION).await;
    wait_enabled(&h, "#subscription-create-submit").await;
    h.wait_css("#subscription-create-submit")
        .await
        .click()
        .await;

    wait_text_contains(
        &h,
        ".thaw-message-bar",
        "an event subscription with that name exists",
    )
    .await;
    // The refusal ALSO toasts: an inline-only failure reads as "nothing
    // happened" (the viewer's mutation-feedback rule).
    assert!(
        wait_text(&h, "Create failed").await,
        "a refused create must toast as well as render the diagnostic inline"
    );
    h.shot(2, "subscription-conflict").await;

    // Nothing was stored twice: the seeded row still carries its original kind.
    wait_text_contains(&h, &row_cell(CONFLICT_SUBSCRIPTION, "kind"), CREATED_KIND).await;

    // The 409 the CDR answered the server fn with is the point of this journey.
    h.assert_console_clean(&["409", "Failed to load resource"])
        .await;
    remove_fixture_subscriptions(&cdr).await;
    h.finish().await;
}
