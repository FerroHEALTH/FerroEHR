// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Archive and Restore on an EHR's and a party's page over stubbed archive
//! routes, and with the admin group off (#6655).

#![expect(
    clippy::disallowed_types,
    reason = "the stubbed exchanges are raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::Value;

use crate::ssr_app::{Viewer, id};

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

/// The archive route of `segment`, or its restore.
fn route(segment: &str, restore: bool) -> String {
    let tail = if restore { "/restore" } else { "" };
    format!("/ferroehr/rest/extensions/v1/admin/archive/{segment}{tail}")
}

/// An EHR's More menu offers Archive and Restore behind confirmations that
/// say the EHR leaves query results; an archive goes back to the EHR with a
/// toast, and a refusal is drawn in the CDR's words.
#[tokio::test]
async fn an_ehr_is_archived_and_restored() {
    let ehr = id("ehr");
    let page_path = format!("/ehrs/{ehr}");
    let viewer = Viewer::start_with(vec![
        exchange("POST", &route("ehrs", false), 204, ""),
        exchange(
            "POST",
            &route("ehrs", true),
            404,
            r#"{"title":"Not Found","detail":"no EHR with that id"}"#,
        ),
    ])
    .await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, &page_path).await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in [
        r#"commandfor="archive-record""#,
        r#"commandfor="restore-record""#,
        "leaves every query result",
        r#"value="archive""#,
    ] {
        assert!(page.body.contains(words), "the EHR page offers {words}");
    }
    let archived = viewer
        .send(
            http::Method::POST,
            &page_path,
            Some(&cookie),
            Some(&[("intent", "archive")]),
        )
        .await;
    assert_eq!(archived.location(), page_path);
    let refused = viewer
        .send(
            http::Method::POST,
            &page_path,
            Some(&cookie),
            Some(&[("intent", "restore")]),
        )
        .await;
    assert!(
        refused.main().contains("no EHR with that id"),
        "{}",
        refused.main()
    );
}

/// A party's More menu offers the same, and a restore goes back to it.
#[tokio::test]
async fn a_party_is_archived_and_restored() {
    let party = id("party");
    let page_path = format!("/parties/person/{party}");
    let viewer = Viewer::start_with(vec![exchange("POST", &route("parties", true), 204, "")]).await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, &page_path).await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains(r#"commandfor="archive-record""#),
        "{}",
        page.body
    );
    assert!(page.body.contains("Its relationships stay where they are"));
    let restored = viewer
        .send(
            http::Method::POST,
            &page_path,
            Some(&cookie),
            Some(&[("intent", "restore")]),
        )
        .await;
    assert_eq!(restored.location(), page_path);
}

/// With the admin group off, neither page draws the actions.
#[tokio::test]
async fn the_actions_wait_for_the_admin_group() {
    let manifest = serde_json::json!({
        "solution": "FerroEHR", "solution_version": "6.0.0", "vendor": "Cadasto B.V.",
        "restapi_specs_version": "1.1.0", "conformance_profile": "BASE+OPTIONS",
        "endpoints": ["/ehr", "/definition", "/query", "/demographic"]
    })
    .to_string();
    let viewer = Viewer::start_with(vec![exchange(
        "OPTIONS",
        "/ferroehr/rest/openehr/v1",
        200,
        &manifest,
    )])
    .await;
    let cookie = viewer.sign_in().await;
    for path in [
        format!("/ehrs/{}", id("ehr")),
        format!("/parties/person/{}", id("party")),
    ] {
        let page = viewer.page(&cookie, &path).await;
        assert_eq!(page.status, http::StatusCode::OK, "{path}");
        assert!(!page.body.contains("archive-record"), "{path}");
    }
}
