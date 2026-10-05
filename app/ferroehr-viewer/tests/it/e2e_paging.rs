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
//! End-to-end journeys over the viewer's TWO paging surfaces.
//!
//! **The shared table footer** (`?page=`/`?size=`), driven on the
//! stored-queries screen (`/queries`), for tables whose rows are all in hand.
//! Its whole contract is that the page window is URL state, not a private
//! signal: paging forward writes `?page=`, the row window follows the URL (so
//! the browser's back/forward and a reload land on the same rows), a page link
//! preserves the screen's other parameters (`?size=` here), and the first page
//! is the plain path again.
//!
//! **The offset controls** (`?offset=`), on the AQL-windowed tables where only
//! the current page is in hand — the EHR detail's compositions tab and the
//! `/ehrs` finder. Their contract adds one thing the footer's does not: the
//! step link merges into the LIVE query map, so the tab a table sits on and the
//! filters that produced it survive both directions. A control that rebuilt the
//! query from scratch would silently navigate back to the Status tab, which is
//! exactly what shipped once with no journey driving it.
//!
//! Fixtures: each journey seeds and removes its own over ITS-REST
//! (`UI_E2E_CDR_URL`) — test setup deliberately bypasses the UI, whose
//! save/delete paths have their own journeys. The footer journey uses `?size=1`
//! so two rows are enough to prove the window moves; the offset journeys have
//! no such knob (the AQL fetch window is the viewer-wide page size), so the
//! compositions journey commits a full page and one more.

use crate::common;

use reqwest::StatusCode;

use common::{
    Harness, count_matching, env, is_present_by, login_basic, poll_some, poll_until, wait_attr,
    wait_attr_change,
};
use thirtyfour::prelude::*;

/// The two stored queries this journey owns, qualified `namespace::name` (the
/// namespace also keeps them out of every other journey's grouping card).
const QUERY_NAMES: [&str; 2] = ["e2e.paging::page-alpha", "e2e.paging::page-beta"];

/// Their stored version — explicit, so the seed and the cleanup address the
/// exact `(name, version)` pair.
const QUERY_VERSION: &str = "1.0.0";

/// The AQL the fixtures hold; the journey never runs them, it only lists them.
const SEED_AQL: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";

/// The admin dev user (quickstart `docker/ferroehr.dev.toml`) — the fixture
/// cleanup goes through the RBAC-gated admin stored-query delete.
fn admin_credentials() -> (String, String) {
    (
        env("UI_E2E_ADMIN_USER").unwrap_or_else(|| "ferroehr-admin".to_owned()),
        env("UI_E2E_ADMIN_PASS").unwrap_or_else(|| "ferroehr".to_owned()),
    )
}

/// Remove the journey's fixtures from the CDR store (404 = already absent), so
/// the journey both starts and ends from a known state.
///
/// # Panics
/// On any answer other than `204`/`404`.
async fn delete_fixtures(cdr: &str) {
    let http = reqwest::Client::new();
    let (user, pass) = admin_credentials();
    for name in QUERY_NAMES {
        let status = http
            .delete(format!(
                "{cdr}/ferroehr/rest/openehr/v1/admin/query/{name}/{QUERY_VERSION}"
            ))
            .basic_auth(&user, Some(&pass))
            .send()
            .await
            .expect("delete a stored-query fixture")
            .status();
        assert!(
            status == StatusCode::NO_CONTENT || status == StatusCode::NOT_FOUND,
            "stored-query cleanup -> {status}"
        );
    }
}

/// Store the journey's fixtures over the Definition API.
///
/// # Panics
/// When the CDR refuses the store.
async fn seed_fixtures(cdr: &str, user: &str, pass: &str) {
    let http = reqwest::Client::new();
    for name in QUERY_NAMES {
        let status = http
            .put(format!(
                "{cdr}/ferroehr/rest/openehr/v1/definition/query/{name}/{QUERY_VERSION}"
            ))
            .basic_auth(user, Some(pass))
            .header("Content-Type", "text/plain")
            .body(SEED_AQL)
            .send()
            .await
            .expect("seed a stored-query fixture")
            .status();
        assert!(status.is_success(), "stored-query seed -> {status}");
    }
}

/// How many stored-query rows the table currently renders.
async fn row_count(h: &Harness) -> usize {
    count_matching(h, "[data-stored-query]").await
}

/// The `name@version` key of the first rendered row — the window's identity.
///
/// # Panics
/// When no row is rendered within the wait budget.
async fn first_row_key(h: &Harness) -> String {
    h.wait_css("[data-stored-query]")
        .await
        .attr("data-stored-query")
        .await
}

/// Poll until the first rendered row is no longer `previous`, and return the
/// new key. This is the window-moved condition (never a sleep): a page link is
/// a real navigation, so the swap is asynchronous either side of hydration.
///
/// # Panics
/// When the window has not moved after 15 s.
async fn wait_row_change(h: &Harness, previous: &str) -> String {
    let moved = poll_some(async || {
        let current = first_row_key(h).await;
        (current != previous).then_some(current)
    })
    .await;
    let Some(current) = moved else {
        let url = h.current_url().await;
        panic!("the row window never moved off `{previous}` (at {url})");
    };
    current
}

/// Poll until the footer's range line starts with `prefix` (`1–1 of `, …).
///
/// # Panics
/// When it never does — reporting the line it actually showed.
async fn wait_range_prefix(h: &Harness, prefix: &str) {
    let mut seen = String::new();
    // NOTE: probe tier — the footer re-renders as the window loads, so a
    // handle detached between the find and the read is "not yet".
    let settled = poll_until(async || {
        seen = h
            .wait_css("[data-page=\"range\"]")
            .await
            .read_text()
            .await
            .unwrap_or_default();
        seen.starts_with(prefix)
    })
    .await;
    assert!(
        settled,
        "the footer range line never started with `{prefix}` (last: `{seen}`)"
    );
}

/// Page the stored-queries table forward and back: the URL carries the page,
/// the rendered window follows it, `?size=` survives the link, and the
/// page-size choice re-pages from the top.
#[tokio::test]
async fn stored_queries_page_through_the_shared_footer() {
    let Some(h) = Harness::start("table-paging").await else {
        return;
    };
    let (Some(cdr), Some(user), Some(pass)) = (
        env("UI_E2E_CDR_URL"),
        env("UI_E2E_BASIC_USER"),
        env("UI_E2E_BASIC_PASS"),
    ) else {
        println!("SKIP table-paging: fixture seeding needs UI_E2E_CDR_URL/UI_E2E_BASIC_*");
        h.finish().await;
        return;
    };
    delete_fixtures(&cdr).await;
    seed_fixtures(&cdr, &user, &pass).await;
    login_basic(&h).await;

    // `?size=1` — one row per page, so two stored queries prove the window
    // moves regardless of what else the stack holds. The size is user input the
    // footer clamps to a sane range; any positive window is honoured, the
    // presets are only links.
    h.goto("/queries?size=1").await;
    let first = first_row_key(&h).await;
    assert_eq!(row_count(&h).await, 1, "?size=1 renders exactly one row");
    wait_range_prefix(&h, "1–1 of ").await;
    // The first page cannot go back: the control keeps its place, inert.
    h.wait_css("button[data-page=\"prev\"]").await;
    h.shot(1, "first-page").await;

    // Forward: the page lands in the URL, the window moves, and the size
    // parameter rides along.
    h.wait_css("a[data-page=\"next\"]").await.click().await;
    h.wait_url_contains("page=1").await;
    let second = wait_row_change(&h, &first).await;
    assert_eq!(row_count(&h).await, 1, "the second page renders one row");
    wait_range_prefix(&h, "2–2 of ").await;
    let url = h.current_url().await;
    assert!(
        url.contains("size=1"),
        "a page link must preserve the screen's other parameters (at {url})"
    );
    h.shot(2, "second-page").await;

    // Back: the first page is the plain path again (the default page is written
    // as its absence) and the original window returns.
    h.wait_css("a[data-page=\"prev\"]").await.click().await;
    h.wait_url_not_contains("page=1").await;
    let back = wait_row_change(&h, &second).await;
    assert_eq!(back, first, "paging back must restore the first window");
    wait_range_prefix(&h, "1–1 of ").await;
    h.shot(3, "back-on-the-first-page").await;

    // A page-size choice re-pages from the top at that window; the default size
    // clears the parameter entirely, so both fixtures are on one page.
    h.wait_css("a[data-page-size=\"25\"]").await.click().await;
    h.wait_url_not_contains("size=1").await;
    h.wait_css("[data-stored-query]").await;
    assert!(
        row_count(&h).await >= 2,
        "the default window must hold both seeded queries"
    );
    h.shot(4, "default-window").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    delete_fixtures(&cdr).await;
    h.finish().await;
}

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

/// Assert every `fragment` is in the current URL — the "my other parameters
/// survived the step" check.
///
/// # Panics
/// When one is missing, naming the URL that lost it.
async fn assert_url_keeps(h: &Harness, fragments: &[&str]) {
    let url = h.current_url().await;
    for fragment in fragments {
        assert!(
            url.contains(fragment),
            "a paging step must preserve `{fragment}` (at {url})"
        );
    }
}

/// The compositions tab's OFFSET paging: stepping forward and back moves the
/// row window, and BOTH the tab and the filter that produced the view survive
/// each step.
///
/// The tab is the surface where the loss was real: a step that rebuilt the
/// query string from scratch dropped `?tab=compositions` and landed the reader
/// back on the Status tab.
#[tokio::test]
async fn the_compositions_tab_pages_by_offset_and_keeps_its_tab_and_filter() {
    let Some(h) = Harness::start("compositions-offset-paging").await else {
        return;
    };
    let (Some(cdr), Some(user), Some(pass)) = (
        env("UI_E2E_CDR_URL"),
        env("UI_E2E_BASIC_USER"),
        env("UI_E2E_BASIC_PASS"),
    ) else {
        println!("SKIP compositions-offset-paging: seeding needs UI_E2E_CDR_URL/UI_E2E_BASIC_*");
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    ensure_paging_template(&http, &v1, &user, &pass).await;
    // This journey's OWN EHR: a full page and one more, each at its own start
    // time, plus one NEWER composition by a different composer that the filter
    // must exclude.
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
    let rows = "tr a[href*='/compositions/']";
    h.goto(&format!(
        "/ehrs/{ehr_id}?tab=compositions&composer={PAGED_COMPOSER_FILTER}"
    ))
    .await;
    // The filter narrows: the newer Beta composition is not on the page, even
    // though it would sort first without it.
    wait_link_count(&h, rows, PAGE_SIZE).await;
    assert!(
        !is_present_by(
            &h,
            By::XPath(format!("//tr[contains(., '{OTHER_COMPOSER}')]"))
        )
        .await,
        "the composer filter must exclude the other composer's composition"
    );
    let first = wait_attr(&h, rows, "href").await;
    h.shot(1, "compositions-first-page").await;

    // Forward: the offset lands in the URL, the window moves to the last row,
    // and BOTH the tab and the filter ride along.
    h.wait_css("a[data-page='next']").await.click().await;
    h.wait_url_contains("offset=25").await;
    let second = wait_attr_change(&h, rows, "href", &first).await;
    wait_link_count(&h, rows, 1).await;
    assert_url_keeps(
        &h,
        &[
            "tab=compositions",
            &format!("composer={PAGED_COMPOSER_FILTER}"),
            "offset=25",
        ],
    )
    .await;
    // The filter is not merely in the URL, it is still APPLIED: the second page
    // holds the 26th Alpha composition, never the newer Beta one.
    assert!(
        !is_present_by(
            &h,
            By::XPath(format!("//tr[contains(., '{OTHER_COMPOSER}')]"))
        )
        .await,
        "the composer filter must still narrow the second page"
    );
    h.shot(2, "compositions-second-page").await;

    // Back: the first page writes the offset as its ABSENCE, the original
    // window returns, and the tab + filter survive this direction too.
    h.wait_css("a[data-page='prev']").await.click().await;
    h.wait_url_not_contains("offset=").await;
    let back = wait_attr_change(&h, rows, "href", &second).await;
    assert_eq!(back, first, "paging back must restore the first window");
    wait_link_count(&h, rows, PAGE_SIZE).await;
    assert_url_keeps(
        &h,
        &[
            "tab=compositions",
            &format!("composer={PAGED_COMPOSER_FILTER}"),
        ],
    )
    .await;
    h.shot(3, "compositions-back-on-the-first-page").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}

/// The `/ehrs` finder's OFFSET paging: the second offset surface, whose window
/// moves the same way and whose first page is the plain path again.
#[tokio::test]
async fn the_ehr_finder_pages_by_offset() {
    let Some(h) = Harness::start("ehrs-offset-paging").await else {
        return;
    };
    let (Some(cdr), Some(user), Some(pass)) = (
        env("UI_E2E_CDR_URL"),
        env("UI_E2E_BASIC_USER"),
        env("UI_E2E_BASIC_PASS"),
    ) else {
        println!("SKIP ehrs-offset-paging: seeding needs UI_E2E_CDR_URL/UI_E2E_BASIC_*");
        h.finish().await;
        return;
    };
    let http = reqwest::Client::new();
    let v1 = format!("{cdr}/ferroehr/rest/openehr/v1");
    // Top the stack up to a second page rather than creating a fixed number:
    // every other journey creates EHRs too, so the count at this point is not
    // knowable in advance.
    let held = ehr_count(&http, &v1, &user, &pass).await;
    for _ in held..=PAGE_SIZE {
        create_ehr(&http, &v1, &user, &pass).await;
    }

    login_basic(&h).await;
    let rows = "tr a[href^='/ehrs/']";
    h.goto("/ehrs").await;
    wait_link_count(&h, rows, PAGE_SIZE).await;
    let first = wait_attr(&h, rows, "href").await;
    h.shot(1, "ehrs-first-page").await;

    h.wait_css("a[data-page='next']").await.click().await;
    h.wait_url_contains("offset=25").await;
    let second = wait_attr_change(&h, rows, "href", &first).await;
    h.shot(2, "ehrs-second-page").await;

    h.wait_css("a[data-page='prev']").await.click().await;
    h.wait_url_not_contains("offset=").await;
    let back = wait_attr_change(&h, rows, "href", &second).await;
    assert_eq!(back, first, "paging back must restore the first window");
    wait_link_count(&h, rows, PAGE_SIZE).await;
    h.shot(3, "ehrs-back-on-the-first-page").await;

    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
