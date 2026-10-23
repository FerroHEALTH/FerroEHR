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
//! End-to-end journeys over the EHR detail's **compositions filters** and the
//! **EHR header identity**:
//!
//! - the filter form writes the URL and narrows the rows (template, composer),
//!   and a row opens the composition on its Reading tab;
//! - a shared filter URL reproduces the view, and a one-day window INCLUDES the
//!   compositions recorded during that day;
//! - the header names the EHR's subject, masked until revealed, and its
//!   queryable and modifiable badges on every tab.
//!
//! Isolation: each journey creates its OWN subject-bound EHR over ITS-REST and
//! commits two compositions into it — different templates, different composers,
//! different context start times — so none of them touches the fixtures the
//! other journeys and the documentation-screenshot pass depend on. The
//! composition bodies are the CDR's own generated examples (spec-valid by
//! construction), with only `composer.name` and `context.start_time` rewritten.

use crate::common;

use common::{Harness, env, find_all, login_basic, poll_until, retype};

/// The two templates the journeys distinguish between; both are uploaded here
/// (idempotently) so the filter has something to narrow.
const TEMPLATE_A: &str = "minimal_evaluation.en.v1";

/// The second template — the one the template filter selects.
const TEMPLATE_B: &str = "minimal_instruction.en.v1";

/// The OPT fixture for [`TEMPLATE_A`].
const FIXTURE_A: &str = "minimal_evaluation.opt";

/// The OPT fixture for [`TEMPLATE_B`].
const FIXTURE_B: &str = "minimal_instruction.opt";

/// The composer of the earlier composition.
const COMPOSER_A: &str = "Alice Ashford";

/// The composer of the later composition.
const COMPOSER_B: &str = "Bruno Bell";

/// The earlier composition's `context.start_time`.
const START_A: &str = "2026-03-04T09:15:00Z";

/// The later composition's `context.start_time` — mid-day, so a one-day upper
/// bound only includes it if the viewer completes the bound to end-of-day.
const START_B: &str = "2026-06-11T14:45:00Z";

/// The UTC day [`START_B`] falls on.
const DAY_B: &str = "2026-06-11";

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

/// A run-unique suffix, so a shared stack can hold several runs' subjects.
///
/// The process id distinguishes runs and the counter distinguishes calls within
/// one — the harness needs distinctness, not entropy, and no clock (the
/// wall-clock reads a `disallowed-methods` ban anyway).
fn unique_suffix() -> String {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("{}-{seq}", std::process::id())
}

/// Store an OPT template over ITS-REST, idempotently for a shared stack:
/// `201` created, `409` already there.
///
/// # Panics
/// On any other answer (a broken stack, not a skip).
async fn ensure_template(http: &reqwest::Client, v1: &str, fixture: &str) {
    let (user, pass) = basic_credentials();
    let path = format!(
        "{}/../../crates/openehr-its/tests/fixtures/sdk/{fixture}",
        env!("CARGO_MANIFEST_DIR")
    );
    let opt = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("the fixture OPT {path} exists: {e}"));
    let status = http
        .post(format!("{v1}/definition/template/adl1.4"))
        .basic_auth(user, Some(pass))
        .header("Content-Type", "application/xml")
        .body(opt)
        .send()
        .await
        .expect("store the template")
        .status();
    assert!(
        status == reqwest::StatusCode::CREATED || status == reqwest::StatusCode::CONFLICT,
        "storing {fixture} answered {status}"
    );
}

/// Create an EHR bound to an external subject and return `(ehr_id, subject_id)`.
///
/// The subject travels in the `EHR_STATUS`'s `PARTY_SELF.external_ref`, which is
/// the only place an outside identity appears on an EHR — so this is what the
/// header's identity line has to render.
///
/// # Panics
/// When the CDR refuses the create.
async fn create_subject_ehr(http: &reqwest::Client, v1: &str) -> (String, String) {
    let (user, pass) = basic_credentials();
    let subject_id = format!("patient-{}", unique_suffix());
    let body: serde_json::Value = http
        .post(format!("{v1}/ehr"))
        .basic_auth(user, Some(pass))
        .header("Prefer", "return=representation")
        .header("Accept", "application/json")
        .json(&serde_json::json!({
            "_type": "EHR_STATUS",
            "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
            // EHR_STATUS is an archetype root, and RM common
            // `org.openehr.rm.common.locatable.adoc` invariant `Archetyped_valid`
            // makes ARCHETYPED mandatory for one — the CDR refuses `422` without it.
            "archetype_details": {
                "_type": "ARCHETYPED",
                "archetype_id": {
                    "_type": "ARCHETYPE_ID",
                    "value": "openEHR-EHR-EHR_STATUS.generic.v1"
                },
                "rm_version": "1.2.0"
            },
            "name": { "_type": "DV_TEXT", "value": "EHR Status" },
            "subject": {
                "_type": "PARTY_SELF",
                "external_ref": {
                    "_type": "PARTY_REF",
                    "namespace": "e2e-patients",
                    "type": "PERSON",
                    "id": { "_type": "HIER_OBJECT_ID", "value": subject_id }
                }
            },
            "is_queryable": true,
            "is_modifiable": true
        }))
        .send()
        .await
        .expect("create a subject-bound EHR")
        .json()
        .await
        .expect("EHR body");
    let ehr_id = body
        .get("ehr_id")
        .and_then(|id| id.get("value"))
        .and_then(serde_json::Value::as_str)
        .expect("the created ehr_id")
        .to_owned();
    (ehr_id, subject_id)
}

/// Commit the CDR's own example composition for `template_id`, with only the
/// composer and the context start time rewritten.
///
/// # Panics
/// When the template has no example or the CDR refuses the commit.
async fn commit_example(
    http: &reqwest::Client,
    v1: &str,
    ehr_id: &str,
    template_id: &str,
    composer: &str,
    start_time: &str,
) {
    let (user, pass) = basic_credentials();
    let mut document: serde_json::Value = http
        .get(format!(
            "{v1}/definition/template/adl1.4/{template_id}/example"
        ))
        .basic_auth(user.clone(), Some(pass.clone()))
        .header("Accept", "application/json")
        .send()
        .await
        .expect("read the template's example composition")
        .json()
        .await
        .expect("example composition body");
    *document
        .pointer_mut("/composer/name")
        .expect("the example composition names its composer") =
        serde_json::Value::String(composer.to_owned());
    *document
        .pointer_mut("/context/start_time/value")
        .expect("the example composition carries a context start time") =
        serde_json::Value::String(start_time.to_owned());
    let status = http
        .post(format!("{v1}/ehr/{ehr_id}/composition"))
        .basic_auth(user, Some(pass))
        .header("Content-Type", "application/json")
        .header("Accept", "application/json")
        .header("Prefer", "return=minimal")
        .json(&document)
        .send()
        .await
        .expect("commit the composition")
        .status();
    assert_eq!(
        status,
        reqwest::StatusCode::CREATED,
        "committing the {template_id} example answered {status}"
    );
}

/// Seed one journey's fixture: a subject-bound EHR holding two compositions
/// that differ in template, composer AND context start time.
async fn seed(http: &reqwest::Client, v1: &str) -> (String, String) {
    ensure_template(http, v1, FIXTURE_A).await;
    ensure_template(http, v1, FIXTURE_B).await;
    let (ehr_id, subject_id) = create_subject_ehr(http, v1).await;
    commit_example(http, v1, &ehr_id, TEMPLATE_A, COMPOSER_A, START_A).await;
    commit_example(http, v1, &ehr_id, TEMPLATE_B, COMPOSER_B, START_B).await;
    (ehr_id, subject_id)
}

/// The text of every composition row, in order.
async fn row_texts(h: &Harness) -> Vec<String> {
    let mut rows = Vec::new();
    for row in find_all(h, "main table tbody tr").await {
        if let Some(text) = row.read_text().await {
            rows.push(text);
        }
    }
    rows
}

/// Wait until the rows are exactly `expected`, one fragment per row in order.
async fn wait_rows(h: &Harness, expected: &[&str]) {
    let mut last = Vec::new();
    let settled = poll_until(async || {
        last = row_texts(h).await;
        last.len() == expected.len()
            && last
                .iter()
                .zip(expected.iter())
                .all(|(row, fragment)| row.contains(fragment))
    })
    .await;
    assert!(
        settled,
        "the composition rows never settled on {expected:?} (last saw {last:?})"
    );
}

/// The filter form writes its fields into the URL, the rows narrow to what
/// the CDR matched, Clear filters empties it, and a row opens the composition
/// on its Reading tab.
#[tokio::test]
async fn composition_filters_travel_in_the_url_and_narrow_the_rows() {
    let Some(h) = Harness::start("compositions-filters").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let (ehr_id, _subject) = seed(&http, &v1).await;

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}")).await;
    wait_rows(&h, &[COMPOSER_B, COMPOSER_A]).await;
    h.shot(1, "unfiltered").await;

    retype(&h, "#f-template", "instruction").await;
    common::submit(&h, "main form[role='search'] button[type=submit]").await;
    h.wait_url_contains("template=instruction").await;
    wait_rows(&h, &[COMPOSER_B]).await;
    h.shot(2, "template-filtered").await;

    h.goto(&format!("/ehrs/{ehr_id}?composer=Ashford")).await;
    wait_rows(&h, &[COMPOSER_A]).await;
    assert_eq!(
        h.wait_css("#f-composer").await.prop("value").await,
        "Ashford",
        "a shared filter URL must refill the form it came from"
    );
    h.shot(3, "composer-filtered").await;

    h.wait_css(&format!("main a[href='/ehrs/{ehr_id}']"))
        .await
        .click()
        .await;
    wait_rows(&h, &[COMPOSER_B, COMPOSER_A]).await;
    for field in ["#f-template", "#f-start-from", "#f-start-to", "#f-composer"] {
        let value = h.wait_css(field).await.prop("value").await;
        assert!(
            value.is_empty(),
            "`{field}` still reads `{value}` after Clear filters"
        );
    }

    h.wait_css("main table tbody a.fui-row-link")
        .await
        .click()
        .await;
    h.wait_url_contains("/compositions/").await;
    h.wait_css("h2.fui-reading-title").await;
    h.shot(4, "row-opens-reading").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// A date window in the URL bounds the context start time, and a one-day
/// window includes what was recorded during that day: the upper bound is
/// completed to the end of its UTC day.
#[tokio::test]
async fn a_shared_date_window_bounds_the_context_start_time_inclusively() {
    let Some(h) = Harness::start("compositions-date-window").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let (ehr_id, _subject) = seed(&http, &v1).await;

    login_basic(&h).await;
    h.goto(&format!("/ehrs/{ehr_id}?to=2026-04-01")).await;
    wait_rows(&h, &[COMPOSER_A]).await;
    h.shot(1, "upper-bound").await;

    h.goto(&format!("/ehrs/{ehr_id}?from=2026-04-01")).await;
    wait_rows(&h, &[COMPOSER_B]).await;
    h.shot(2, "lower-bound").await;

    h.goto(&format!("/ehrs/{ehr_id}?from={DAY_B}&to={DAY_B}"))
        .await;
    wait_rows(&h, &[COMPOSER_B]).await;
    h.shot(3, "one-day-window").await;

    h.goto(&format!("/ehrs/{ehr_id}?from=2027-01-01&to=2027-01-02"))
        .await;
    wait_rows(&h, &[]).await;
    common::wait_main_contains(&h, "No composition matches").await;
    h.shot(4, "no-match").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The EHR header names the subject, masked until revealed, and shows its
/// status badges on every tab, from the one current-`EHR_STATUS` read.
#[tokio::test]
async fn the_ehr_header_names_the_subject_and_its_status_badges() {
    let Some(h) = Harness::start("ehr-header-identity").await else {
        return;
    };
    let Some(cdr) = cdr_url() else {
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let (ehr_id, subject_id) = seed(&http, &v1).await;

    login_basic(&h).await;
    for tab in ["", "/status", "/tags"] {
        h.goto(&format!("/ehrs/{ehr_id}{tab}")).await;
        let header = h.wait_css(".fui-subject").await.text().await;
        assert!(
            header.contains("e2e-patients") && !header.contains(&subject_id),
            "the header names the namespace and masks the subject id (got `{header}`)"
        );
        for badge in ["Queryable", "Modifiable"] {
            assert!(
                header.contains(badge),
                "the header shows `{badge}`: {header}"
            );
        }
    }
    h.shot(1, "header-masked").await;

    // Reveal is the hydrated island's button; it shows the whole id.
    h.wait_xpath("//div[contains(@class, 'fui-subject')]//button[normalize-space(.)='Reveal']")
        .await
        .click()
        .await;
    common::wait_text_contains(&h, ".fui-subject", &subject_id).await;
    h.shot(2, "header-revealed").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
