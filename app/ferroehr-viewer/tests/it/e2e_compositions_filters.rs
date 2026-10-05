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
//!   and a row opens the composition viewer's RENDERED clinical reading;
//! - a shared filter URL reproduces the view, and a one-day window INCLUDES the
//!   compositions recorded during that day;
//! - the header names the EHR's subject and its queryable/modifiable badges
//!   from the same status read the Status tab shows.
//!
//! Isolation: each journey creates its OWN subject-bound EHR over ITS-REST and
//! commits two compositions into it — different templates, different composers,
//! different context start times — so none of them touches the fixtures the
//! other journeys and the documentation-screenshot pass depend on. The
//! composition bodies are the CDR's own generated examples (spec-valid by
//! construction), with only `composer.name` and `context.start_time` rewritten.

use crate::common;

use common::{Harness, env, find_all, login_basic, poll_until, retype};
use thirtyfour::prelude::*;

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

/// The text of every composition row currently on screen, newest first — a row
/// being a table row that carries a link into the composition viewer.
async fn row_texts(h: &Harness) -> Vec<String> {
    let mut rows = Vec::new();
    for link in find_all(h, "tr a[href*='/compositions/']").await {
        // The row is the link's grandparent (<a> inside <td> inside <tr>).
        // NOTE: probe tier on both hops — a handle the `<Transition>` detaches
        // mid-swap is "not yet" for the poll loop around this read.
        if let Some(row) = link.read_find(By::XPath("./ancestor::tr[1]")).await
            && let Some(text) = row.read_text().await
        {
            rows.push(text);
        }
    }
    rows
}

/// Poll until the visible composition rows are exactly the ones whose text
/// contains each `expected` fragment, in order — a `<Transition>` keeps the
/// PREVIOUS rows on screen while the filtered read runs, so an immediate count
/// would assert against the unfiltered page.
///
/// # Panics
/// When the rows never settle on the expected set, reporting what was on screen.
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

/// The filter form writes its fields into the URL, the rows narrow to what the
/// CDR matched, and a row opens the composition viewer already in its RENDERED
/// clinical reading.
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
    h.goto(&format!("/ehrs/{ehr_id}?tab=compositions")).await;
    // Unfiltered: both compositions, newest first — the tab's behaviour with an
    // empty filter set is exactly what it always was.
    wait_rows(&h, &[COMPOSER_B, COMPOSER_A]).await;
    h.shot(1, "unfiltered").await;

    // The template filter, typed into the form and submitted.
    retype(&h, "#composition-filter-template", "instruction").await;
    h.wait_css("#composition-filter-apply").await.click().await;
    h.wait_url_contains("template=instruction").await;
    // The submission stays on the tab it came from and drops any page offset.
    h.wait_url_contains("tab=compositions").await;
    wait_rows(&h, &[COMPOSER_B]).await;
    h.shot(2, "template-filtered").await;

    // The composer filter, from the URL — the same state a shared link carries.
    h.goto(&format!("/ehrs/{ehr_id}?tab=compositions&composer=Ashford"))
        .await;
    wait_rows(&h, &[COMPOSER_A]).await;
    // …and the form came back filled from the URL, so the view is reproducible.
    let composer_field = h.wait_css("#composition-filter-composer").await;
    assert_eq!(
        composer_field.prop("value").await,
        "Ashford",
        "a shared filter URL must refill the form it came from"
    );
    h.shot(3, "composer-filtered").await;

    // Clearing puts every row back AND empties the boxes: the fields follow the
    // address bar, so none of them can keep claiming a filter the URL dropped.
    h.wait_css("#composition-filter-clear").await.click().await;
    wait_rows(&h, &[COMPOSER_B, COMPOSER_A]).await;
    for field in [
        "#composition-filter-template",
        "#composition-filter-from",
        "#composition-filter-to",
        "#composition-filter-composer",
    ] {
        let value = h.wait_css(field).await.prop("value").await;
        assert!(
            value.is_empty(),
            "`{field}` still reads `{value}` after Clear"
        );
    }

    // A row opens the viewer's RENDERED clinical reading — no tab click.
    h.wait_css("tr a[href*='/compositions/']")
        .await
        .click()
        .await;
    h.wait_url_contains("view=rendered").await;
    h.wait_css("[data-doc-row]").await;
    h.shot(4, "row-opens-rendered").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// A date window in the URL bounds the composition's context start time, and a
/// ONE-DAY window includes what was recorded during that day: the upper bound
/// is completed to the end of its UTC day, not read as midnight.
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
    // An upper bound before the later composition keeps only the earlier one.
    h.goto(&format!("/ehrs/{ehr_id}?tab=compositions&to=2026-04-01"))
        .await;
    wait_rows(&h, &[COMPOSER_A]).await;
    h.shot(1, "upper-bound").await;

    // A lower bound after it keeps only the later one.
    h.goto(&format!("/ehrs/{ehr_id}?tab=compositions&from=2026-04-01"))
        .await;
    wait_rows(&h, &[COMPOSER_B]).await;
    h.shot(2, "lower-bound").await;

    // The whole-day case: the later composition was recorded at 14:45 UTC, so a
    // window of exactly its day must include it.
    h.goto(&format!(
        "/ehrs/{ehr_id}?tab=compositions&from={DAY_B}&to={DAY_B}"
    ))
    .await;
    wait_rows(&h, &[COMPOSER_B]).await;
    h.shot(3, "one-day-window").await;

    // A window neither composition falls in is the filtered empty state, not
    // the "this EHR holds nothing" one.
    h.goto(&format!(
        "/ehrs/{ehr_id}?tab=compositions&from=2027-01-01&to=2027-01-02"
    ))
    .await;
    wait_rows(&h, &[]).await;
    h.wait_xpath("//*[contains(., 'No compositions match these filters')]")
        .await;
    h.shot(4, "no-match").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The EHR-detail header names the EHR's subject and shows its capability
/// badges, on EVERY tab — from the same current-`EHR_STATUS` read the Status
/// tab renders, never a second one.
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
    h.goto(&format!("/ehrs/{ehr_id}?tab=compositions")).await;

    // The identity line carries the external_ref's id AND its namespace.
    let identity = h.wait_css("#ehr-identity").await;
    let text = identity.text().await;
    assert!(
        text.contains(&subject_id) && text.contains("e2e-patients"),
        "the header must name the subject id and its namespace (got `{text}`)"
    );
    // Both capability badges, scoped to the header so the Status tab's own pair
    // cannot satisfy this on its behalf.
    h.wait_css(
        "[data-status-scope='header'][data-status-flag='queryable'][data-status-value='true']",
    )
    .await;
    h.wait_css(
        "[data-status-scope='header'][data-status-flag='modifiable'][data-status-value='true']",
    )
    .await;
    h.shot(1, "header-on-compositions-tab").await;

    // The Status tab shows the same two flags from the same read; the header
    // keeps them while that tab is open.
    h.goto(&format!("/ehrs/{ehr_id}?tab=status")).await;
    h.wait_css("[data-status-scope='tab'][data-status-flag='queryable'][data-status-value='true']")
        .await;
    h.wait_css("#ehr-identity").await;
    h.shot(2, "header-on-status-tab").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
