// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Query › Cohort over a stubbed cohort route: a run over the bound
//! predicates, a suppressed small cell, a refusal, and a deployment that
//! binds none (#6660).

#![expect(
    clippy::disallowed_types,
    reason = "the stubbed exchanges are raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::Value;

use crate::ssr_app::Viewer;

/// A stubbed exchange: `method` of `path` answered `status` with `response`.
fn exchange(method: &str, path: &str, status: u16, response: &str) -> Value {
    serde_json::json!({
        "method": method,
        "path": path,
        "body": "",
        "status": status,
        "headers": {"content-type": "application/json"},
        "response": response,
    })
}

/// The redacted configuration of a deployment binding `city` and `sex`.
fn bound() -> Value {
    let binding = serde_json::json!({"archetype": "openEHR-DEMOGRAPHIC-ADDRESS.address.v1", "at_code": "at0001", "match": "exact"});
    exchange(
        "GET",
        "/ferroehr/rest/extensions/v1/admin/config",
        200,
        &serde_json::json!({"privacy": {"cohort": {"predicates": {"city": binding, "sex": binding}, "subjects": false}}})
            .to_string(),
    )
}

/// A cohort answer of one row, its small cell `suppressed` or not.
fn answered(suppressed: bool) -> Value {
    let rows = if suppressed {
        serde_json::json!([])
    } else {
        serde_json::json!([["8849fd1e-0000-4000-8000-000000000001::local::1"]])
    };
    exchange(
        "POST",
        "/ferroehr/rest/extensions/v1/query/cohort",
        200,
        &serde_json::json!({
            "meta": {"_type": "RESULTSET", "cohort": {"size": 412, "served_ehrs": if suppressed { 0 } else { 388 }, "suppressed": suppressed, "definition": "9f2c4e"}},
            "columns": [{"name": "uid", "path": "c/uid/value"}],
            "rows": rows
        })
        .to_string(),
    )
}

/// The form of a run over Groningen.
const RUN: [(&str, &str); 3] = [
    (
        "aql",
        "SELECT c/uid/value AS uid FROM EHR e CONTAINS COMPOSITION c",
    ),
    ("cohort-city", "Groningen"),
    ("intent", "run"),
];

/// A run offers the bound predicates only, draws the rows as a result set
/// with the cohort's facts, and its exports carry the cohort.
#[tokio::test]
async fn a_cohort_run_draws_its_rows_and_its_facts() {
    let viewer = Viewer::start_with(vec![bound(), answered(false)]).await;
    let cookie = viewer.sign_in().await;
    let form = viewer.page(&cookie, "/query/cohort").await;
    assert_eq!(form.status, http::StatusCode::OK, "{}", form.body);
    assert!(
        form.main().contains(r#"name="cohort-city""#),
        "{}",
        form.main()
    );
    assert!(
        form.main().contains(r#"name="cohort-sex""#),
        "{}",
        form.main()
    );
    assert!(
        !form.main().contains(r#"name="cohort-postcode_area""#),
        "{}",
        form.main()
    );
    let page = viewer
        .send(
            http::Method::POST,
            "/query/cohort",
            Some(&cookie),
            Some(&RUN),
        )
        .await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    let main = page.main();
    for words in [
        "8849fd1e-0000-4000-8000-000000000001",
        "412",
        "388",
        "9f2c4e",
    ] {
        assert!(main.contains(words), "the run shows {words}:\n{main}");
    }
    assert!(
        main.contains(r#"<input type="hidden" name="cohort" value="1">"#),
        "the exports run the same cohort:\n{main}"
    );
}

/// A suppressed small cell says so in place of rows.
#[tokio::test]
async fn a_suppressed_cell_says_so() {
    let viewer = Viewer::start_with(vec![bound(), answered(true)]).await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .send(
            http::Method::POST,
            "/query/cohort",
            Some(&cookie),
            Some(&RUN),
        )
        .await;
    assert!(
        page.main().contains("The rows are withheld"),
        "{}",
        page.main()
    );
}

/// A refusal is drawn in the CDR's words.
#[tokio::test]
async fn a_refused_cohort_is_drawn_in_the_cdrs_words() {
    let viewer = Viewer::start_with(vec![
        bound(),
        exchange(
            "POST",
            "/ferroehr/rest/extensions/v1/query/cohort",
            400,
            r#"{"title":"Bad Request","detail":"`city` is not a bound cohort predicate"}"#,
        ),
    ])
    .await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .send(
            http::Method::POST,
            "/query/cohort",
            Some(&cookie),
            Some(&RUN),
        )
        .await;
    assert!(
        page.main().contains("is not a bound cohort predicate"),
        "{}",
        page.main()
    );
}

/// A deployment that binds no predicate offers no cohort: by its
/// configuration, or by the route's `404` when the session may not read it.
#[tokio::test]
async fn a_deployment_binding_nothing_offers_no_cohort() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/query/cohort").await;
    assert!(
        page.main().contains("binds no cohort predicate"),
        "{}",
        page.main()
    );
    let hidden = Viewer::start_with(vec![
        exchange(
            "GET",
            "/ferroehr/rest/extensions/v1/admin/config",
            403,
            r#"{"title":"Forbidden"}"#,
        ),
        exchange(
            "POST",
            "/ferroehr/rest/extensions/v1/query/cohort",
            404,
            r#"{"title":"Not Found"}"#,
        ),
    ])
    .await;
    let cookie = hidden.sign_in().await;
    let page = hidden
        .send(
            http::Method::POST,
            "/query/cohort",
            Some(&cookie),
            Some(&RUN),
        )
        .await;
    assert!(
        page.main().contains("binds no cohort predicate"),
        "{}",
        page.main()
    );
    assert!(
        !page.main().contains(r#"name="cohort-city""#),
        "{}",
        page.main()
    );
}
