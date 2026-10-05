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
//! End-to-end journey over the `RESULT_SET` **export** — the one viewer
//! surface whose outcome is a file rather than a screen.
//!
//! The export is a plain HTML `<form method="post">` to the viewer's own
//! `/export/aql` BFF route (`ferroehr_viewer::export`), so a browser click
//! hands the bytes to the download manager and `WebDriver` never sees them. The
//! journey therefore does what the button does: it runs the query on screen,
//! reads the THREE fields the form actually holds (`q`, `parameters_json`,
//! `format`) out of the DOM, lifts the browser's own session cookie off the
//! `WebDriver`, and re-posts exactly that request — so what it asserts is the
//! button's own target answering the button's own request, under the button's
//! own session.
//!
//! What it asserts: the CSV carries the column aliases as its header row and
//! then the very rows the table shows; the JSON carries the same `RESULT_SET`;
//! both arrive as downloads with their `Content-Disposition` filenames; and the
//! route refuses a caller with no session instead of exporting to anyone who
//! asks (the public-endpoint rule — `.claude/rules/leptos-ui.md` §0).
//!
//! The query carries its own `LIMIT`, which is what makes the comparison exact:
//! ITS-REST forbids combining `fetch`/`offset` with an AQL window
//! (`specifications/docs/query/Request.md` §Common Headers and Query
//! Parameters), so the screen's run and the export's run are the same bare
//! request and must return the same rows.

use crate::common;

use reqwest::StatusCode;

use common::{Element, Harness, env, find_all, login_basic};
use thirtyfour::prelude::*;

/// The query the journey exports: two scalar columns over the compositions
/// `scripts/ui-e2e.sh` seeds, ordered by a UNIQUE key and windowed by its own
/// `LIMIT` so the screen and the export see one identical, stable row set.
const EXPORT_AQL: &str = concat!(
    "SELECT c/uid/value AS uid, c/context/start_time/value AS observed ",
    "FROM EHR e CONTAINS COMPOSITION c ",
    "WHERE c/archetype_details/template_id/value = 'minimal_evaluation.en.v1' ",
    "ORDER BY c/uid/value ASC LIMIT 3"
);

/// The column aliases the query names, which are what the table heads with and
/// what the CSV's header row must spell.
const EXPORT_COLUMNS: [&str; 2] = ["uid", "observed"];

/// One export form's fields, read from the DOM exactly as the browser would
/// submit them.
struct PostedForm {
    /// The `q` field: the AQL the screen currently holds.
    q: String,
    /// The `parameters_json` field: the bindings object, empty when none.
    parameters_json: String,
    /// The `format` field: `csv` or `json`.
    format: String,
}

/// Read one hidden field's LIVE value off `form`.
///
/// `prop`, not `attr`: the viewer binds these fields with `prop:value`, so the
/// DOM property is where the current query lives (rules §5 — an attribute only
/// carries the initial value).
///
/// # Panics
/// When the field is absent or unreadable.
async fn field_value(form: &Element, name: &str) -> String {
    form.find(By::Css(format!("input[name='{name}']")))
        .await
        .prop("value")
        .await
}

/// The export form whose `format` field is `format`, read field by field.
///
/// Matched on the field rather than on document order: the two forms are
/// identical apart from that one value, and an order-based pick would silently
/// export the wrong representation if they were ever reordered.
///
/// # Panics
/// When no export form carries that format.
async fn posted_form(h: &Harness, format: &str) -> PostedForm {
    let forms = find_all(h, "form[action='/export/aql']").await;
    for form in forms {
        let held = field_value(&form, "format").await;
        if held == format {
            return PostedForm {
                q: field_value(&form, "q").await,
                parameters_json: field_value(&form, "parameters_json").await,
                format: held,
            };
        }
    }
    panic!("no `/export/aql` form on screen carries format `{format}`");
}

/// The browser's cookies for the viewer origin, as one `Cookie` header.
///
/// This is what makes the re-post honest: the request travels on the SAME
/// session the signed-in browser holds, not on a credential the test invented.
///
/// # Panics
/// When the signed-in browser holds no cookie at all.
async fn session_cookie_header(h: &Harness) -> String {
    let cookies = h.cookies().await;
    assert!(
        !cookies.is_empty(),
        "a signed-in viewer browser must hold a session cookie"
    );
    cookies
        .iter()
        .map(|c| format!("{}={}", c.name, c.value))
        .collect::<Vec<_>>()
        .join("; ")
}

/// POST the form to `/export/aql`, optionally carrying a session cookie.
///
/// Redirects are NOT followed: the route answers an unauthenticated caller with
/// a redirect to `/login`, and following it would turn a refusal into a `200`
/// carrying a login page.
///
/// # Panics
/// When the client cannot be built or the request cannot be sent.
async fn post_export(base: &str, cookie: Option<&str>, form: &PostedForm) -> reqwest::Response {
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .expect("an HTTP client that does not follow redirects");
    let mut request = http.post(format!("{base}/export/aql"));
    if let Some(cookie) = cookie {
        request = request.header("Cookie", cookie);
    }
    request
        .form(&[
            ("q", form.q.as_str()),
            ("parameters_json", form.parameters_json.as_str()),
            ("format", form.format.as_str()),
        ])
        .send()
        .await
        .expect("post the export form")
}

/// One response header as text (empty when absent/non-ASCII).
fn header(response: &reqwest::Response, name: reqwest::header::HeaderName) -> String {
    response
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_owned()
}

/// The rendered results table: its header cells, then one `Vec` of cell texts
/// per body row.
///
/// # Panics
/// When the table has not rendered.
async fn table_on_screen(h: &Harness) -> (Vec<String>, Vec<Vec<String>>) {
    h.wait_css("table tbody tr").await;
    let mut headers = Vec::new();
    for cell in find_all(h, "table thead th").await {
        headers.push(cell.text().await);
    }
    let mut rows = Vec::new();
    for row in find_all(h, "table tbody tr").await {
        let mut cells = Vec::new();
        for cell in row.find_all(By::Tag("td")).await {
            cells.push(cell.text().await);
        }
        rows.push(cells);
    }
    (headers, rows)
}

/// Run a query on the raw AQL screen and leave the results table on screen.
///
/// # Panics
/// When the query never runs or renders no row.
async fn run_export_query(h: &Harness) {
    h.goto("/queries/aql").await;
    h.wait_css("#aql-editor").await.send_keys(EXPORT_AQL).await;
    // Run stays DISABLED until the typed AQL reaches the signal, so the wait
    // has to carry that condition — a click on the disabled button is
    // intercepted by the toolbar above it, not merely lost.
    h.wait_clickable_xpath("//button[normalize-space(.)='Run']")
        .await
        .click()
        .await;
    h.wait_css("table tbody tr").await;
}

/// The export buttons serve the rows on screen: CSV as a header row plus those
/// rows, JSON as the same `RESULT_SET`, both as named downloads — and neither
/// to a caller with no session.
#[tokio::test]
#[expect(
    clippy::too_many_lines,
    reason = "one linear journey: run, read the form, re-post it in both formats, assert the bytes"
)]
async fn the_result_export_serves_the_rows_on_screen() {
    let Some(h) = Harness::start("aql-export").await else {
        return;
    };
    if env("UI_E2E_SEEDED_EHR_ID").is_none() {
        println!("SKIP aql-export: UI_E2E_SEEDED_EHR_ID unset (run scripts/ui-e2e.sh)");
        h.finish().await;
        return;
    }
    login_basic(&h).await;
    run_export_query(&h).await;
    h.shot(1, "results-before-export").await;

    let (headers, rows) = table_on_screen(&h).await;
    // The header cells carry Tailwind's `uppercase` and `text()` returns the
    // RENDERED text, so the screen is matched case-insensitively; the alias
    // itself is what the CSV header row must spell, asserted verbatim below.
    let headed: Vec<String> = headers.iter().map(|h| h.to_lowercase()).collect();
    assert_eq!(
        headed, EXPORT_COLUMNS,
        "the table heads with the query's column aliases"
    );
    assert_eq!(
        rows.len(),
        3,
        "the query's own LIMIT windows the screen to three rows"
    );

    let base = h.base.clone();
    let cookie = session_cookie_header(&h).await;

    // ── CSV: the button's own request, answered as a download ───────────────
    let csv_form = posted_form(&h, "csv").await;
    assert_eq!(
        csv_form.q, EXPORT_AQL,
        "the CSV form posts the AQL the screen ran"
    );
    let response = post_export(&base, Some(&cookie), &csv_form).await;
    assert_eq!(response.status(), StatusCode::OK, "the CSV export answers");
    assert_eq!(
        header(&response, reqwest::header::CONTENT_TYPE),
        "text/csv; charset=utf-8"
    );
    assert_eq!(
        header(&response, reqwest::header::CONTENT_DISPOSITION),
        "attachment; filename=\"aql-export.csv\"",
        "the CSV arrives as a named download, not as a page"
    );
    let csv = response.text().await.expect("the CSV body");
    let lines: Vec<&str> = csv.lines().collect();
    // Plain joins are the right comparison here: every projected value is a uid
    // or an ISO-8601 instant, so no cell carries the comma, quote or newline
    // that would make the writer quote it (RFC 4180 quoting is pinned by
    // `ferroehr_viewer::export`'s own unit tests).
    let mut expected = vec![EXPORT_COLUMNS.join(",")];
    expected.extend(rows.iter().map(|row| row.join(",")));
    assert_eq!(
        lines, expected,
        "the CSV is the column aliases followed by exactly the rows on screen"
    );

    // ── JSON: the same rows, as the RESULT_SET the CDR served ───────────────
    let json_form = posted_form(&h, "json").await;
    assert_eq!(
        json_form.q, csv_form.q,
        "both export buttons post the same query"
    );
    let response = post_export(&base, Some(&cookie), &json_form).await;
    assert_eq!(response.status(), StatusCode::OK, "the JSON export answers");
    assert_eq!(
        header(&response, reqwest::header::CONTENT_TYPE),
        "application/json"
    );
    assert_eq!(
        header(&response, reqwest::header::CONTENT_DISPOSITION),
        "attachment; filename=\"aql-export.json\"",
        "the JSON arrives as a named download, not as a page"
    );
    let body = response.text().await.expect("the JSON body");
    let result_set: serde_json::Value = serde_json::from_str(&body).expect("the exported JSON");
    let columns: Vec<String> = result_set
        .get("columns")
        .and_then(serde_json::Value::as_array)
        .expect("the RESULT_SET columns")
        .iter()
        .map(|column| {
            column
                .get("name")
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default()
                .to_owned()
        })
        .collect();
    assert_eq!(
        columns, EXPORT_COLUMNS,
        "the exported RESULT_SET names the same columns the table heads with"
    );
    let exported: Vec<Vec<String>> = result_set
        .get("rows")
        .and_then(serde_json::Value::as_array)
        .expect("the RESULT_SET rows")
        .iter()
        .map(|row| {
            row.as_array()
                .map(|cells| {
                    cells
                        .iter()
                        .map(|cell| match cell {
                            serde_json::Value::String(s) => s.clone(),
                            serde_json::Value::Null => String::new(),
                            other => other.to_string(),
                        })
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        exported, rows,
        "the exported RESULT_SET carries exactly the rows on screen"
    );

    // ── The route is a public endpoint, and it enforces the session ─────────
    let refused = post_export(&base, None, &csv_form).await;
    assert_eq!(
        refused.status(),
        StatusCode::SEE_OTHER,
        "an export request with no viewer session is redirected, never served"
    );
    assert_eq!(
        header(&refused, reqwest::header::LOCATION),
        "/login",
        "the refusal sends the caller to sign in"
    );

    h.shot(2, "results-after-export").await;
    h.assert_console_clean(&["401", "Failed to load resource"])
        .await;
    h.finish().await;
}
