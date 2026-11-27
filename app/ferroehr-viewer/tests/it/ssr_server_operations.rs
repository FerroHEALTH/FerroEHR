// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Server › Activity and Server › Integrity over stubbed admin routes: a
//! report, a refused interval, a verification with its rebuild, and the
//! admin group turned off (#6656).

#![expect(
    clippy::disallowed_types,
    reason = "the stubbed exchanges are raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::Value;

use crate::ssr_app::Viewer;

/// The extensions root the stubs answer under.
const EXT: &str = "/ferroehr/rest/extensions/v1";

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

/// The query the report routes are asked with for January 2026's
/// demographic service.
const JANUARY: &str =
    "a_service=DEMOGRAPHIC&time_interval=2026-01-01T00%3A00%3A00Z%2F2026-01-31T23%3A59%3A59.999Z";

/// The four report routes for January, answered `status` with `bodies`.
fn report(status: u16, bodies: [&str; 4]) -> Vec<Value> {
    [
        "contribution",
        "contribution/count",
        "versioned_composition/count",
        "composition_version/count",
    ]
    .into_iter()
    .zip(bodies)
    .map(|(route, body)| {
        exchange(
            "GET",
            &format!("{EXT}/admin/report/{route}?{JANUARY}"),
            status,
            body,
        )
    })
    .collect()
}

/// The address of January's demographic report.
const JANUARY_PAGE: &str = "/server/activity?a_service=DEMOGRAPHIC&from=2026-01-01&to=2026-01-31";

/// A report draws its four counts and each contribution id, a demographic
/// one linked to its page.
#[tokio::test]
async fn activity_counts_a_service_over_an_interval() {
    let id = "8849182c-82ad-4088-a07f-48ead4180515";
    let viewer = Viewer::start_with(report(200, [&format!("[\"{id}\"]"), "1", "0", "0"])).await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, JANUARY_PAGE).await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    let main = page.main();
    for words in [
        "Contributions",
        "Contribution ids listed",
        "Compositions changed",
        "Composition versions",
        &format!(r#"href="/parties/contributions/{id}""#),
    ] {
        assert!(main.contains(words), "Activity shows {words}:\n{main}");
    }
    assert!(
        page.body.contains(r#"href="/server/activity""#),
        "the tab is offered:\n{}",
        page.body
    );
}

/// A refused interval is drawn beside the fields in the CDR's words.
#[tokio::test]
async fn a_refused_interval_is_drawn_in_the_cdrs_words() {
    let refusal = r#"{"title":"Bad Request","detail":"the lower bound is after the upper bound"}"#;
    let viewer = Viewer::start_with(report(400, [refusal; 4])).await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, JANUARY_PAGE).await;
    let main = page.main();
    assert!(
        main.contains("the lower bound is after the upper bound"),
        "{main}"
    );
    assert!(
        main.contains(r#"name="a_service""#),
        "the form stays:\n{main}"
    );
}

/// A verification lists its findings and offers Rebuild behind a
/// confirmation; the rebuild draws what it did to each version.
#[tokio::test]
async fn integrity_verifies_then_rebuilds_behind_a_confirmation() {
    let vo = "1f0b7d64-6b2a-4a1f-9f0e-6b1d2a3c4d5e";
    let verified = serde_json::json!({
        "versions_checked": 12, "versions_with_body": 12, "versions_without_body": 0,
        "mismatch_count": 1,
        "mismatches": [{"domain": "clinical", "vo_id": vo, "sys_version": 2,
            "kind": "COMPOSITION", "defect": "nodes_missing"}],
        "truncated": false, "elapsed_ms": 9
    })
    .to_string();
    let rebuilt = serde_json::json!({
        "versions_checked": 12, "versions_damaged": 1, "versions_rebuilt": 1,
        "versions_refused": 0,
        "records": [{"vo_id": vo, "sys_version": 2, "kind": "COMPOSITION",
            "defect": "nodes_missing", "outcome": "rebuilt", "node_rows": 7}],
        "truncated": false, "elapsed_ms": 11
    })
    .to_string();
    let viewer = Viewer::start_with(vec![
        exchange(
            "POST",
            &format!("{EXT}/admin/integrity/verify?vo_id={vo}"),
            200,
            &verified,
        ),
        exchange(
            "POST",
            &format!("{EXT}/admin/integrity/rebuild-nodes?vo_id={vo}"),
            200,
            &rebuilt,
        ),
    ])
    .await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .send(
            http::Method::POST,
            "/server/integrity",
            Some(&cookie),
            Some(&[("vo_id", vo), ("intent", "verify")]),
        )
        .await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    let main = page.main();
    for words in ["nodes_missing", "clinical", r#"commandfor="rebuild-nodes""#] {
        assert!(main.contains(words), "the report shows {words}:\n{main}");
    }
    assert!(page.body.contains(r#"value="rebuild""#), "{}", page.body);
    let page = viewer
        .send(
            http::Method::POST,
            "/server/integrity",
            Some(&cookie),
            Some(&[("vo_id", vo), ("intent", "rebuild")]),
        )
        .await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert!(page.main().contains("rebuilt"), "{}", page.main());
}

/// An admin group that answers `405` hides the tab's actions behind a note.
#[tokio::test]
async fn an_admin_group_that_is_off_hides_the_sections() {
    let off = r#"{"title":"Method Not Allowed"}"#;
    let viewer = Viewer::start_with(vec![exchange(
        "POST",
        &format!("{EXT}/admin/integrity/verify"),
        405,
        off,
    )])
    .await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .send(
            http::Method::POST,
            "/server/integrity",
            Some(&cookie),
            Some(&[("intent", "verify")]),
        )
        .await;
    let main = page.main();
    assert!(main.contains("admin API turned off"), "{main}");
    assert!(!main.contains(r#"value="verify""#), "{main}");
}
