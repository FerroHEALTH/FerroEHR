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
//! refusal on the form that caused it.
//!
//! The screen is probe-gated on the CDR serving its event-subscription admin
//! API, which the composed E2E stack enables (`docker/viewer/e2e-env.yml`
//! sets `FERROEHR__EVENTS__ADMIN_API`); a stack without it hides the nav entry
//! entirely, and the fixture clean-up says so rather than passing vacuously.
//! The hidden-when-absent half is unit-tested in
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
    Harness, confirm_in_dialog, env, fill, login_basic_as, wait_css_absent, wait_main_contains,
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
    format!("{cdr}/ferroehr/rest/extensions/v1/admin/event_subscription")
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

/// The list row of the subscription `name`.
fn row(name: &str) -> String {
    format!("//main//tr[.//a[@href='/subscriptions/{name}/edit']]")
}

/// The CRUD round trip: create a subscription, see its row, edit what it
/// selects (clearing one predicate and switching delivery off), then delete it
/// through the confirmation dialog and watch the row go.
#[tokio::test]
async fn the_viewer_creates_edits_and_deletes_a_subscription() {
    let Some(h) = Harness::start("event-subscriptions-crud").await else {
        return;
    };
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("SKIP: UI_E2E_CDR_URL unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    };
    remove_fixture_subscriptions(&cdr).await;
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto("/subscriptions").await;
    h.wait_css("nav a[href='/subscriptions']").await;
    h.wait_css("main a[href='/subscriptions/new']")
        .await
        .click()
        .await;
    fill(&h, "#sn", ROUND_TRIP_SUBSCRIPTION).await;
    h.wait_css("#sk").await.select_by_value(CREATED_KIND).await;
    h.wait_css("#sc")
        .await
        .select_by_value(CREATED_CHANGE_TYPE)
        .await;
    h.wait_css("#st")
        .await
        .select_by_value(CREATED_TEMPLATE)
        .await;
    h.shot(1, "subscription-new").await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_contains("/subscriptions").await;
    let created = h
        .wait_xpath(&row(ROUND_TRIP_SUBSCRIPTION))
        .await
        .text()
        .await;
    for words in ["Creation", "Composition", CREATED_TEMPLATE, "Enabled"] {
        assert!(created.contains(words), "the row says `{words}`: {created}");
    }
    h.shot(2, "subscription-created").await;

    h.goto(&format!("/subscriptions/{ROUND_TRIP_SUBSCRIPTION}/edit"))
        .await;
    h.wait_css("#sk").await.select_by_value(EDITED_KIND).await;
    h.wait_css("#st").await.select_by_value("").await;
    h.wait_css("#enabled").await.click().await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    let edited = h
        .wait_xpath(&row(ROUND_TRIP_SUBSCRIPTION))
        .await
        .text()
        .await;
    assert!(edited.contains("EHR status"), "the kind changed: {edited}");
    assert!(
        !edited.contains(CREATED_TEMPLATE),
        "the template cleared: {edited}"
    );
    assert!(!edited.contains("Enabled"), "delivery is off: {edited}");
    h.shot(3, "subscription-edited").await;

    h.goto(&format!("/subscriptions/{ROUND_TRIP_SUBSCRIPTION}/edit"))
        .await;
    h.wait_css("summary[aria-controls='subscription-more']")
        .await
        .click()
        .await;
    confirm_in_dialog(
        &h,
        "button[commandfor='delete-subscription']",
        "#delete-subscription button[type=submit]",
    )
    .await;
    h.wait_url_contains("/subscriptions").await;
    wait_css_absent(
        &h,
        &format!("main a[href='/subscriptions/{ROUND_TRIP_SUBSCRIPTION}/edit']"),
    )
    .await;
    h.shot(4, "subscription-deleted").await;

    remove_fixture_subscriptions(&cdr).await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// A name the CDR cannot accept is refused before anything is sent, and a
/// name it already holds is refused with its `409`, the form kept as typed.
#[tokio::test]
async fn a_refused_subscription_name_is_answered_on_its_form() {
    let Some(h) = Harness::start("event-subscriptions-conflict").await else {
        return;
    };
    let Some(cdr) = env("UI_E2E_CDR_URL") else {
        println!("SKIP: UI_E2E_CDR_URL unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    };
    remove_fixture_subscriptions(&cdr).await;
    seed_subscription(&cdr, CONFLICT_SUBSCRIPTION).await;
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;

    h.goto("/subscriptions/new").await;
    fill(&h, "#sn", "not a valid name!").await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    wait_main_contains(&h, "Use letters, digits, dots, dashes and underscores only").await;
    assert_eq!(h.wait_css("#sn").await.attr("aria-invalid").await, "true");
    h.shot(1, "subscription-name-invalid").await;

    fill(&h, "#sn", CONFLICT_SUBSCRIPTION).await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    wait_main_contains(
        &h,
        &format!("The CDR already has a subscription called {CONFLICT_SUBSCRIPTION}"),
    )
    .await;
    assert_eq!(
        h.wait_css("#sn").await.prop("value").await,
        CONFLICT_SUBSCRIPTION,
        "the form keeps what was typed"
    );
    h.shot(2, "subscription-name-taken").await;

    // Discard leaves without creating anything.
    common::submit(&h, "#editor-dock button[value='discard']").await;
    h.wait_url_contains("/subscriptions").await;

    remove_fixture_subscriptions(&cdr).await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
