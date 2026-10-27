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
//! End-to-end journeys over New contribution (`POST
//! /ehr/{ehr_id}/contribution`), the openEHR atomic change set:
//!
//! - the atomic commit: stage a COMPOSITION creation and an `EHR_STATUS`
//!   modification, set the description, commit once, and find one new
//!   CONTRIBUTION carrying two versions;
//! - the refused change set: one change the CDR cannot accept refuses the
//!   whole commit, with its diagnostic on screen, the staging intact and the
//!   EHR's contribution count unchanged.
//!
//! Isolation: each journey creates its OWN EHR over ITS-REST, so neither
//! touches the seeded fixtures the other journeys and the
//! documentation-screenshot pass depend on.

use crate::common;

use common::{Harness, count_matching, env, fill, login_basic, wait_main_contains};

/// The template the E2E harness seeds; its CDR-generated example composition is
/// the create member's document (spec-valid by construction).
const SEED_TEMPLATE: &str = "minimal_evaluation.en.v1";

/// A template id no stack holds — the refusal journey renames the seeded
/// example's `archetype_details.template_id` to it, so the member is
/// well-formed but cannot be validated.
const MISSING_TEMPLATE: &str = "no_such_template.v1";

/// The CDR base URL the harness exports for REST-side test setup; `None` skips
/// with a reason.
fn cdr_url() -> Option<String> {
    if let Some(url) = env("UI_E2E_CDR_URL") {
        Some(url)
    } else {
        println!("SKIP: UI_E2E_CDR_URL unset (run scripts/ui-e2e.sh)");
        None
    }
}

/// The Basic credentials the composed stack seeds.
fn basic_credentials() -> (String, String) {
    (
        env("UI_E2E_BASIC_USER").unwrap_or_else(|| "ferroehr".to_owned()),
        env("UI_E2E_BASIC_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// Create a fresh EHR over ITS-REST and return its `ehr_id`.
///
/// # Panics
/// When the CDR refuses the create (a broken stack, not a skip).
async fn create_ehr(http: &reqwest::Client, v1: &str) -> String {
    let (user, pass) = basic_credentials();
    let body: serde_json::Value = http
        .post(format!("{v1}/ehr"))
        .basic_auth(user, Some(pass))
        .header("Prefer", "return=representation")
        .header("Accept", "application/json")
        .send()
        .await
        .expect("create an EHR")
        .json()
        .await
        .expect("EHR body");
    body.get("ehr_id")
        .and_then(|id| id.get("value"))
        .and_then(serde_json::Value::as_str)
        .expect("the created ehr_id")
        .to_owned()
}

/// The CDR's OWN generated example composition for the seeded template — the
/// create member's document, spec-valid by construction (no hand-built
/// fixture).
///
/// # Panics
/// When the template is absent from the stack (the harness seeds it).
async fn example_composition(http: &reqwest::Client, v1: &str) -> String {
    let (user, pass) = basic_credentials();
    let response = http
        .get(format!(
            "{v1}/definition/template/adl1.4/{SEED_TEMPLATE}/example"
        ))
        .basic_auth(user, Some(pass))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("read the template's example composition");
    assert_eq!(
        response.status(),
        http::StatusCode::OK,
        "the seeded template {SEED_TEMPLATE} must serve an example composition"
    );
    response.text().await.expect("example composition body")
}

/// How many CONTRIBUTIONs the EHR holds, read over ITS-REST — the independent
/// check that a refused commit wrote nothing.
///
/// # Panics
/// When the list cannot be read.
async fn contribution_total(http: &reqwest::Client, v1: &str, ehr_id: &str) -> u64 {
    let (user, pass) = basic_credentials();
    let body: serde_json::Value = http
        .get(format!("{v1}/ehr/{ehr_id}/contribution?offset=0&fetch=1"))
        .basic_auth(user, Some(pass))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("list the EHR's contributions")
        .json()
        .await
        .expect("contribution list body");
    body.get("total")
        .and_then(serde_json::Value::as_u64)
        .expect("the contribution total")
}

/// How many changes the builder holds.
async fn staged_rows(h: &Harness) -> usize {
    count_matching(h, "textarea[name^='staged-']").await
}

/// Fill change `n` as a new composition of the seeded template.
async fn stage_composition_create(h: &Harness, n: u8, document: &str) {
    h.wait_css(&format!("#kind-{n}"))
        .await
        .select_by_value("create")
        .await;
    h.wait_css(&format!("#template-{n}"))
        .await
        .select_by_value(SEED_TEMPLATE)
        .await;
    fill(h, &format!("#staged-{n}"), document).await;
}

/// Two correlated changes, one CONTRIBUTION: a composition creation and an
/// `EHR_STATUS` modification commit together, and the EHR gains one
/// contribution carrying two versions.
#[tokio::test]
async fn two_staged_changes_commit_as_one_contribution_with_two_versions() {
    let Some(h) = Harness::start("contribution-authoring").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let http = reqwest::Client::new();
    let ehr_id = create_ehr(&http, &v1).await;
    let document = example_composition(&http, &v1).await;
    let before = contribution_total(&http, &v1, &ehr_id).await;

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}")).await;
    h.wait_css("summary[aria-controls='ehr-more']")
        .await
        .click()
        .await;
    h.wait_css(&format!(
        "#ehr-more a[href='/ehrs/{ehr_id}/contribution/new']"
    ))
    .await
    .click()
    .await;
    h.shot(1, "contribution-new").await;

    stage_composition_create(&h, 1, &document).await;
    common::submit(&h, "button[name='intent'][value='stage']").await;
    assert_eq!(staged_rows(&h).await, 2, "Stage a change opens change 2");
    h.wait_css("#kind-2").await.select_by_value("status").await;
    common::submit(&h, "button[name='intent'][value='edit']").await;
    let preceding = h
        .wait_css("input[name='preceding-2']")
        .await
        .prop("value")
        .await;
    assert!(
        preceding.contains("::"),
        "the status change names the version it supersedes: {preceding}"
    );
    let status = h.wait_css("#staged-2").await.prop("value").await;
    assert!(
        status.contains("EHR_STATUS"),
        "the status change opens the current status"
    );
    h.shot(2, "two-staged").await;

    fill(
        &h,
        "#au-d",
        "Encounter recorded at triage; EHR status refreshed",
    )
    .await;
    let label = h
        .wait_css("#editor-dock button[value='save']")
        .await
        .text()
        .await;
    assert!(
        label.contains("Commit 2 changes"),
        "the commit says the staged count: {label}"
    );
    common::submit(&h, "#editor-dock button[value='save']").await;
    h.wait_url_contains(&format!("/ehrs/{ehr_id}/contributions/"))
        .await;
    h.wait_css(".fui-toast-text").await;
    wait_main_contains(&h, "COMPOSITION").await;
    wait_main_contains(&h, "EHR_STATUS").await;
    h.shot(3, "committed").await;

    assert_eq!(
        contribution_total(&http, &v1, &ehr_id).await,
        before + 1,
        "an atomic commit adds exactly one CONTRIBUTION"
    );
    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// One unacceptable member refuses the whole change set: the CDR's diagnostic
/// is on screen, the staging survives, and nothing was committed.
#[tokio::test]
async fn a_refused_member_commits_nothing_and_keeps_the_staging() {
    let Some(h) = Harness::start("contribution-refusal").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let http = reqwest::Client::new();
    let ehr_id = create_ehr(&http, &v1).await;
    let document = example_composition(&http, &v1).await;
    let before = contribution_total(&http, &v1, &ehr_id).await;
    let unvalidatable = document.replace(SEED_TEMPLATE, MISSING_TEMPLATE);
    assert_ne!(
        unvalidatable, document,
        "the example composition must name the seeded template"
    );

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}/contribution/new")).await;
    stage_composition_create(&h, 1, &document).await;
    common::submit(&h, "button[name='intent'][value='stage']").await;
    // The stage post is a full navigation, so the page hydrates again; typing
    // into the editor island before it does races the WebAssembly start-up.
    h.wait_hydrated().await;
    h.wait_css("#kind-2").await.select_by_value("create").await;
    fill(&h, "#staged-2", &unvalidatable).await;
    common::submit(&h, "#editor-dock button[value='save']").await;
    wait_main_contains(&h, "422").await;
    wait_main_contains(&h, MISSING_TEMPLATE).await;
    h.shot(1, "commit-refused").await;
    assert_eq!(
        staged_rows(&h).await,
        2,
        "a refused commit keeps every staged change"
    );
    assert_eq!(
        contribution_total(&http, &v1, &ehr_id).await,
        before,
        "an all-or-nothing change set must commit nothing when one member is refused"
    );
    h.assert_console_clean(&[]).await;
    h.finish().await;
}
