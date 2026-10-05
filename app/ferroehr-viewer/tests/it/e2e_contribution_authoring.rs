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
//! End-to-end journeys over the viewer's **CONTRIBUTION authoring** path —
//! `POST /ehr/{ehr_id}/contribution`, the openEHR-native atomic change set:
//!
//! - **the atomic commit**: stage a COMPOSITION creation AND an `EHR_STATUS`
//!   modification, set the change-set description, commit once — and find ONE
//!   new CONTRIBUTION on the Contributions tab carrying TWO versions;
//! - **the refused change set**: one staged change whose document the CDR
//!   cannot accept must refuse the WHOLE commit — the diagnostic verbatim on
//!   screen, the staging list intact, and the EHR's contribution count
//!   unchanged (all-or-nothing: nothing was committed).
//!
//! Isolation: each journey creates its OWN EHR over ITS-REST, so neither
//! touches the seeded fixtures the other journeys and the
//! documentation-screenshot pass depend on.

use crate::common;

use common::{
    Harness, count_matching, env, login_basic, poll_until, retype, wait_enabled, wait_text,
    wait_text_contains,
};

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

/// Pick a `<select>` option by value.
///
/// # Panics
/// When the control is not a select or the option is absent.
async fn pick(h: &Harness, css: &str, value: &str) {
    h.wait_css(css).await.select_by_value(value).await;
}

/// How many rows the staging list currently holds.
async fn staged_rows(h: &Harness) -> usize {
    count_matching(h, "[data-staged]").await
}

/// Poll until the staging list holds exactly `expected` rows — an explicit
/// condition, never a sleep: staging is a reactive DOM update, so an immediate
/// count races the render.
///
/// # Panics
/// When it never does, reporting what the list held instead.
async fn wait_staged_rows(h: &Harness, expected: usize, what: &str) {
    let mut last = usize::MAX;
    let settled = poll_until(async || {
        last = staged_rows(h).await;
        last == expected
    })
    .await;
    assert!(
        settled,
        "{what}: the staging list never held {expected} row(s) (last saw {last}): {}",
        h.evidence_dump("staging").await
    );
}

/// Click the Stage button once it is enabled (it is inert while the draft is
/// empty or a seed is still loading, and a click on a disabled control is
/// silently lost).
async fn stage_the_draft(h: &Harness, what: &str) {
    wait_enabled(h, "#stage-add-change").await;
    println!("staging {what}");
    h.wait_css("#stage-add-change").await.click().await;
}

/// Stage a COMPOSITION creation from the CDR's own example document.
async fn stage_composition_create(h: &Harness, document: &str) {
    pick(h, "#stage-kind", "create").await;
    pick(h, "#stage-template", SEED_TEMPLATE).await;
    retype(h, "#stage-body", document).await;
    stage_the_draft(h, "the composition creation").await;
}

/// Two correlated changes, one CONTRIBUTION: the acceptance criterion.
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
    h.goto(&format!("/ehrs/{ehr_id}?tab=commit")).await;
    h.wait_css("#stage-notice").await;
    h.shot(1, "commit-tab").await;

    // 1. A brand-new COMPOSITION, from the CDR's own template example.
    stage_composition_create(&h, &document).await;
    wait_staged_rows(&h, 1, "the composition creation").await;

    // 2. The EHR's own status, seeded from the CDR. The draft stays inert
    //    until that seed lands — waiting on it is the condition, not a sleep.
    pick(&h, "#stage-kind", "status").await;
    wait_enabled(&h, "#stage-body").await;
    wait_text_contains(&h, "#stage-preceding", "::").await;
    stage_the_draft(&h, "the status modification").await;
    wait_staged_rows(&h, 2, "the status modification").await;
    h.shot(2, "two-staged").await;

    // The button states the count it is about to commit.
    let label = h.wait_css("#stage-commit").await.text().await;
    assert!(
        label.contains("Commit 2 changes as one contribution"),
        "the commit button must state the staged count (read `{label}`)"
    );

    retype(
        &h,
        "#stage-description",
        "Encounter recorded at triage; EHR status refreshed",
    )
    .await;
    h.wait_toasts_cleared().await;
    wait_enabled(&h, "#stage-commit").await;
    h.wait_css("#stage-commit").await.click().await;

    // One CONTRIBUTION, two versions — the result pane names both.
    let result = h.wait_css("#stage-result").await;
    let reported = result.text().await;
    assert!(
        reported.contains("committed with 2 versions"),
        "the commit must report ONE contribution carrying TWO versions (read `{reported}`): {}",
        h.evidence_dump("commit-result").await
    );
    h.shot(3, "committed").await;

    // Exactly one new CONTRIBUTION exists.
    let after = contribution_total(&http, &v1, &ehr_id).await;
    assert_eq!(
        after,
        before + 1,
        "an atomic commit adds exactly one CONTRIBUTION"
    );

    // The staging list is empty again, and the committed contribution opens in
    // the EXISTING contributions tab — the viewer has one contribution viewer.
    wait_staged_rows(&h, 0, "a successful commit").await;
    let contribution_uid = reported
        .split_whitespace()
        .nth(1)
        .expect("the reported contribution uid")
        .to_owned();
    h.goto(&format!("/ehrs/{ehr_id}?tab=contributions")).await;
    wait_text_contains(&h, "#contribution-total", &format!("{after} ")).await;
    retype(&h, "#contribution-uid", &contribution_uid).await;
    h.wait_xpath("//button[normalize-space(.)='Look up']")
        .await
        .click()
        .await;
    // The CONTRIBUTION's own `versions` are the OBJECT_REFs of the versions
    // this commit minted — one COMPOSITION and one EHR_STATUS.
    assert!(
        wait_text(&h, "\"COMPOSITION\"").await,
        "the committed contribution must carry the COMPOSITION version: {}",
        h.evidence_dump("contribution-view").await
    );
    assert!(
        wait_text(&h, "\"EHR_STATUS\"").await,
        "the committed contribution must carry the EHR_STATUS version: {}",
        h.evidence_dump("contribution-view").await
    );
    h.shot(4, "contribution-opened").await;

    h.assert_console_clean(&[]).await;
    h.finish().await;
}

/// One unacceptable member refuses the WHOLE change set: the diagnostic stays
/// on screen verbatim, the staging survives, and nothing was committed.
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

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}?tab=commit")).await;
    h.wait_css("#stage-notice").await;

    // A good member first, so the refusal is provably about the SECOND one and
    // the all-or-nothing property is what is being measured.
    stage_composition_create(&h, &document).await;
    // The SAME document naming a template the CDR does not hold: well-formed
    // (so the viewer stages it and the envelope parses) but unvalidatable, so
    // the refusal is the per-version validation branch rather than a parse
    // failure — and its diagnostic names the template verbatim.
    let unvalidatable = document.replace(SEED_TEMPLATE, MISSING_TEMPLATE);
    assert_ne!(
        unvalidatable, document,
        "the example composition must name the seeded template"
    );
    retype(&h, "#stage-body", &unvalidatable).await;
    stage_the_draft(&h, "the unacceptable composition").await;
    wait_staged_rows(&h, 2, "both members before the commit").await;

    h.wait_toasts_cleared().await;
    wait_enabled(&h, "#stage-commit").await;
    h.wait_css("#stage-commit").await.click().await;

    // The CDR's diagnostic, verbatim, beside the failure toast: the status it
    // answered plus the reason IN ITS OWN WORDS, naming the template.
    let diagnostic = h.wait_css("#stage-diagnostic").await;
    let text = diagnostic.text().await;
    assert!(
        text.contains("422") && text.contains(MISSING_TEMPLATE),
        "the CDR's own diagnostic must render verbatim (read `{text}`): {}",
        h.evidence_dump("commit-refused").await
    );
    assert!(
        wait_text(&h, "Commit failed").await,
        "a refused commit toasts as well as rendering inline: {}",
        h.evidence_dump("commit-refused-toast").await
    );

    // The staging survived, and NOTHING was committed.
    assert_eq!(
        staged_rows(&h).await,
        2,
        "a refused commit keeps every staged change: {}",
        h.evidence_dump("staging-after-refusal").await
    );
    assert_eq!(
        contribution_total(&http, &v1, &ehr_id).await,
        before,
        "an all-or-nothing change set must commit NOTHING when one member is refused"
    );
    h.goto(&format!("/ehrs/{ehr_id}?tab=contributions")).await;
    wait_text_contains(&h, "#contribution-total", &format!("{before} ")).await;

    // The deliberate refusal is a server-fn error, which leptos transports as a
    // 500 the browser logs as a failed fetch — allow-listed like every other
    // journey with a deliberate negative step, never the hydration errors the
    // gate exists for.
    h.assert_console_clean(&["Failed to load resource"]).await;
    h.finish().await;
}
