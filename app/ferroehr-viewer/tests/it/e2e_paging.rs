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
// e2e journeys are assertive by design; skip-with-reason prints; the shared
// harness module is per-test-binary (the corpus.rs test-file precedent)
//! End-to-end journeys over the listings that page: the EHRs list, one
//! EHR's compositions and the audit log.
//!
//! A listing that can show patient data keeps its page out of the address
//! (FerroUI `leptos-ui.md` §0): the pager posts the page to `/paging`, which
//! keeps it in a session cookie and sends the browser back to the listing. So
//! the journeys assert the window moves while the URL keeps only the
//! listing's own filters, and that the position survives a fresh load.
//!
//! Fixtures: each journey seeds its own over ITS-REST (`UI_E2E_CDR_URL`); the
//! compositions journey commits a full page and one more.

use crate::common;

use reqwest::StatusCode;

use common::{
    Harness, count_matching, env, login_basic, login_basic_as, poll_until, wait_text_contains,
};

/// The viewer-wide page size — also the AQL fetch window the offset controls
/// step by (`components::data_table::PAGE_SIZE`). A full page plus one is the
/// smallest fixture that produces a second page.
const PAGE_SIZE: usize = 25;

/// The template the composition fixtures are built from (uploaded here
/// idempotently, so the journey does not depend on another one having run).
const PAGING_TEMPLATE: &str = "minimal_evaluation.en.v1";

/// That template's OPT fixture.
const PAGING_FIXTURE: &str = "minimal_evaluation.opt";

/// The composer of the compositions this journey pages through — distinct
/// enough that the filter it sets can only match its own fixtures.
const PAGED_COMPOSER: &str = "E2E Paging Alpha";

/// The substring the compositions filter is set to. One word on purpose: it
/// travels in the URL, and a space would be re-encoded by the paging link's own
/// query builder, leaving the assertions measuring an encoding rather than the
/// surviving parameter.
const PAGED_COMPOSER_FILTER: &str = "Alpha";

/// A second composer, on ONE newer composition, so the filter above is proven
/// to narrow rather than merely to survive.
const OTHER_COMPOSER: &str = "E2E Paging Beta";

/// Store the OPT fixture, idempotently for a shared stack: `201` created, `409`
/// already there.
///
/// # Panics
/// On any other answer (a broken stack, not a skip).
async fn ensure_paging_template(http: &reqwest::Client, v1: &str, user: &str, pass: &str) {
    let path = format!(
        "{}/../../crates/openehr-its/tests/fixtures/sdk/{PAGING_FIXTURE}",
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
        status == StatusCode::CREATED || status == StatusCode::CONFLICT,
        "storing {PAGING_FIXTURE} answered {status}"
    );
}

/// Create an EHR and return its id.
///
/// # Panics
/// When the CDR refuses the create.
async fn create_ehr(http: &reqwest::Client, v1: &str, user: &str, pass: &str) -> String {
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

/// Commit the CDR's own example composition with `composer` and `start_time`
/// rewritten.
///
/// Every fixture gets a DISTINCT start time on purpose: the tab orders by
/// `c/context/start_time/value DESC`, and a page window over tied sort keys is
/// not a stable order — the rows could reshuffle between offsets and the
/// window-moved assertion would be measuring the database's whim.
///
/// # Panics
/// When the template has no example or the CDR refuses the commit.
async fn commit_paging_example(
    http: &reqwest::Client,
    v1: &str,
    credentials: (&str, &str),
    ehr_id: &str,
    composer: &str,
    start_time: &str,
) {
    let (user, pass) = credentials;
    let mut document: serde_json::Value = http
        .get(format!(
            "{v1}/definition/template/adl1.4/{PAGING_TEMPLATE}/example"
        ))
        .basic_auth(user, Some(pass))
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
        StatusCode::CREATED,
        "composition commit -> {status}"
    );
}

/// How many EHRs the CDR holds, over the Query API.
///
/// # Panics
/// When the CDR refuses the query or answers no row.
async fn ehr_count(http: &reqwest::Client, v1: &str, user: &str, pass: &str) -> usize {
    let body: serde_json::Value = http
        .post(format!("{v1}/query/aql"))
        .basic_auth(user, Some(pass))
        .header("Accept", "application/json")
        .json(&serde_json::json!({ "q": "SELECT COUNT(*) FROM EHR e" }))
        .send()
        .await
        .expect("count the EHRs")
        .json()
        .await
        .expect("result set");
    let count = body
        .pointer("/rows/0/0")
        .and_then(serde_json::Value::as_u64)
        .expect("the count row");
    usize::try_from(count).unwrap_or(usize::MAX)
}

/// How many rows matching `css` are currently rendered.
async fn link_count(h: &Harness, css: &str) -> usize {
    count_matching(h, css).await
}

/// Poll until exactly `expected` rows matching `css` are rendered.
///
/// # Panics
/// When the count never settles, reporting what was on screen.
async fn wait_link_count(h: &Harness, css: &str, expected: usize) {
    let mut last = 0;
    let settled = poll_until(async || {
        last = link_count(h, css).await;
        last == expected
    })
    .await;
    assert!(
        settled,
        "`{css}` never settled on {expected} rows (last saw {last})"
    );
}

/// The listing's pager, which posts and comes back.
const PAGER: &str = "nav.fui-pager";

/// Step the pager `which` way (`next`, `prev`, `first`, `last`).
async fn step(h: &Harness, which: &str) {
    common::submit(h, &format!("{PAGER} button[data-page='{which}']")).await;
}

/// The href of the first row link of `css`.
async fn first_row(h: &Harness, css: &str) -> String {
    h.wait_css(css).await.attr("href").await
}

/// The admin credentials the audit log needs.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// The audit log's pager: Next moves the window, the address stays the plain
/// path, a fresh load keeps the page, and First returns.
#[tokio::test]
async fn the_audit_log_pages_through_the_post_pager() {
    let Some(h) = Harness::start("audit-paging").await else {
        return;
    };
    let (user, pass) = admin_credentials();
    login_basic_as(&h, &user, &pass).await;
    let rows = "main table tbody a.fui-row-link";
    h.goto("/audit").await;
    wait_text_contains(&h, &format!("{PAGER} [role='status']"), "1-25 of").await;
    let first = first_row(&h, rows).await;
    h.shot(1, "audit-first-page").await;

    step(&h, "next").await;
    wait_text_contains(&h, &format!("{PAGER} [role='status']"), "26-50 of").await;
    assert!(
        h.current_url().await.ends_with("/audit"),
        "the page is kept out of the address"
    );
    assert_ne!(first_row(&h, rows).await, first, "the window moved");
    h.goto("/audit").await;
    wait_text_contains(&h, &format!("{PAGER} [role='status']"), "26-50 of").await;
    h.shot(2, "audit-second-page-kept").await;

    step(&h, "first").await;
    wait_text_contains(&h, &format!("{PAGER} [role='status']"), "1-25 of").await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The compositions tab pages and keeps the filter that produced the view in
/// both directions.
#[tokio::test]
async fn the_compositions_tab_pages_and_keeps_its_filter() {
    let Some(h) = Harness::start("compositions-paging").await else {
        return;
    };
    let (Some(cdr), Some(user), Some(pass)) = (
        env("UI_E2E_CDR_URL"),
        env("UI_E2E_BASIC_USER"),
        env("UI_E2E_BASIC_PASS"),
    ) else {
        println!("SKIP compositions-paging: seeding needs UI_E2E_CDR_URL/UI_E2E_BASIC_*");
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    ensure_paging_template(&http, &v1, &user, &pass).await;
    let ehr_id = create_ehr(&http, &v1, &user, &pass).await;
    for minute in 0..=PAGE_SIZE {
        commit_paging_example(
            &http,
            &v1,
            (&user, &pass),
            &ehr_id,
            PAGED_COMPOSER,
            &format!("2026-03-04T09:{minute:02}:00Z"),
        )
        .await;
    }
    commit_paging_example(
        &http,
        &v1,
        (&user, &pass),
        &ehr_id,
        OTHER_COMPOSER,
        "2026-03-05T09:00:00Z",
    )
    .await;

    login_basic(&h).await;
    let rows = "main table tbody a.fui-row-link";
    let filtered = format!("/ehrs/{ehr_id}?composer={PAGED_COMPOSER_FILTER}");
    h.goto(&filtered).await;
    wait_link_count(&h, rows, PAGE_SIZE).await;
    let first = first_row(&h, rows).await;
    let page = h.wait_css("main table").await.text().await;
    assert!(
        !page.contains(OTHER_COMPOSER),
        "the composer filter narrows the rows"
    );
    h.shot(1, "compositions-first-page").await;

    step(&h, "next").await;
    wait_link_count(&h, rows, 1).await;
    assert!(
        h.current_url().await.ends_with(&filtered),
        "the step comes back to the filtered listing (at {})",
        h.current_url().await
    );
    let page = h.wait_css("main table").await.text().await;
    assert!(
        !page.contains(OTHER_COMPOSER),
        "the filter still narrows the second page"
    );
    h.shot(2, "compositions-second-page").await;

    step(&h, "prev").await;
    wait_link_count(&h, rows, PAGE_SIZE).await;
    assert_eq!(
        first_row(&h, rows).await,
        first,
        "paging back restores the first window"
    );
    assert!(h.current_url().await.ends_with(&filtered));
    h.shot(3, "compositions-back-on-the-first-page").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The EHRs list pages the same way.
#[tokio::test]
async fn the_ehrs_list_pages() {
    let Some(h) = Harness::start("ehrs-paging").await else {
        return;
    };
    let (Some(cdr), Some(user), Some(pass)) = (
        env("UI_E2E_CDR_URL"),
        env("UI_E2E_BASIC_USER"),
        env("UI_E2E_BASIC_PASS"),
    ) else {
        println!("SKIP ehrs-paging: seeding needs UI_E2E_CDR_URL/UI_E2E_BASIC_*");
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    let held = ehr_count(&http, &v1, &user, &pass).await;
    for _ in held..=PAGE_SIZE {
        create_ehr(&http, &v1, &user, &pass).await;
    }

    login_basic(&h).await;
    let rows = "main table tbody a.fui-row-link";
    h.goto("/ehrs").await;
    wait_link_count(&h, rows, PAGE_SIZE).await;
    let first = first_row(&h, rows).await;
    h.shot(1, "ehrs-first-page").await;
    step(&h, "next").await;
    let moved = poll_until(async || {
        common::read_attr(&h, rows, "href")
            .await
            .is_some_and(|href| href != first)
    })
    .await;
    assert!(moved, "the window moved");
    assert!(h.current_url().await.ends_with("/ehrs"));
    h.shot(2, "ehrs-second-page").await;
    step(&h, "prev").await;
    wait_link_count(&h, rows, PAGE_SIZE).await;
    assert_eq!(
        first_row(&h, rows).await,
        first,
        "paging back restores the first window"
    );
    h.shot(3, "ehrs-back-on-the-first-page").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
