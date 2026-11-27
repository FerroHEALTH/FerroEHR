// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! EHR Extract export and import, and the repository's dump and load, over
//! stubbed routes (#6657).

#![expect(
    clippy::disallowed_types,
    reason = "the stubbed exchanges are raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::Value;

use crate::ssr_app::{Viewer, id};

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

/// An EHR's More menu offers its extract, which downloads what the CDR
/// serves.
#[tokio::test]
async fn an_ehr_exports_as_an_extract() {
    let ehr = id("ehr");
    let extract = r#"[{"_type":"EXTRACT"}]"#;
    let viewer = Viewer::start_with(vec![exchange(
        "GET",
        &format!("{EXT}/message/export/{ehr}"),
        200,
        extract,
    )])
    .await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, &format!("/ehrs/{ehr}")).await;
    assert!(
        page.body.contains(&format!("/download/extract/{ehr}")),
        "the EHR offers its extract"
    );
    let file = viewer
        .page(&cookie, &format!("/download/extract/{ehr}"))
        .await;
    assert_eq!(file.status, http::StatusCode::OK, "{}", file.body);
    assert_eq!(file.body, extract);
    let disposition = file
        .headers
        .get(http::header::CONTENT_DISPOSITION)
        .and_then(|value| value.to_str().ok())
        .unwrap_or_default();
    assert!(
        disposition.contains(&format!("ehr-extract-{ehr}.json")),
        "{disposition}"
    );
}

/// An import that the CDR stores opens the new EHR; one it refuses stays on
/// the page in the CDR's words.
#[tokio::test]
async fn an_import_opens_the_ehr_or_is_refused_in_the_cdrs_words() {
    let form = [
        ("source-text", r#"[{"_type":"EXTRACT"}]"#),
        ("ehr_id", ""),
        ("intent", "import"),
    ];
    let stored = Viewer::start_with(vec![exchange(
        "POST",
        &format!("{EXT}/message/import"),
        201,
        r#"{"uid":"7d44b88c-4199-4bad-97dc-d78268e01398"}"#,
    )])
    .await;
    let cookie = stored.sign_in().await;
    let list = stored.page(&cookie, "/ehrs").await;
    assert!(
        list.body.contains(r#"href="/ehrs/import""#),
        "EHRs offers Import"
    );
    let answer = stored
        .send(
            http::Method::POST,
            "/ehrs/import",
            Some(&cookie),
            Some(&form),
        )
        .await;
    assert_eq!(
        answer.location(),
        "/ehrs/7d44b88c-4199-4bad-97dc-d78268e01398"
    );
    let refused = Viewer::start_with(vec![exchange(
        "POST",
        &format!("{EXT}/message/import"),
        409,
        r#"{"title":"Conflict","detail":"an EHR with this id already exists"}"#,
    )])
    .await;
    let cookie = refused.sign_in().await;
    let page = refused
        .send(
            http::Method::POST,
            "/ehrs/import",
            Some(&cookie),
            Some(&form),
        )
        .await;
    assert!(
        page.main().contains("an EHR with this id already exists"),
        "{}",
        page.main()
    );
}

/// A dump reports that everything moved; a load lists what it skipped; an
/// admin group that is off hides the tab's actions.
#[tokio::test]
async fn the_repository_is_dumped_and_loaded() {
    let skipped = r#"[{"entity_type":"EHR","entity_id":"7d44b88c-4199-4bad-97dc-d78268e01398","dump_status":false,"error":"an EHR with this id already exists"}]"#;
    let viewer = Viewer::start_with(vec![
        exchange("POST", &format!("{EXT}/admin/dump"), 200, "[]"),
        exchange("POST", &format!("{EXT}/admin/load"), 200, skipped),
    ])
    .await;
    let cookie = viewer.sign_in().await;
    let tab = viewer.page(&cookie, "/server/repository").await;
    assert_eq!(tab.status, http::StatusCode::OK, "{}", tab.body);
    assert!(tab.body.contains(r#"commandfor="repository-load-confirm""#));
    let dumped = viewer
        .send(
            http::Method::POST,
            "/server/repository",
            Some(&cookie),
            Some(&[("dump_location", "/var/dumps/a"), ("intent", "dump")]),
        )
        .await;
    assert!(
        dumped.main().contains("Every record moved."),
        "{}",
        dumped.main()
    );
    let loaded = viewer
        .send(
            http::Method::POST,
            "/server/repository",
            Some(&cookie),
            Some(&[("load_location", "/var/dumps/a"), ("intent", "load")]),
        )
        .await;
    assert!(
        loaded.main().contains("an EHR with this id already exists"),
        "{}",
        loaded.main()
    );
    let off = Viewer::start_with(vec![exchange(
        "POST",
        &format!("{EXT}/admin/dump"),
        405,
        r#"{"title":"Method Not Allowed"}"#,
    )])
    .await;
    let cookie = off.sign_in().await;
    let page = off
        .send(
            http::Method::POST,
            "/server/repository",
            Some(&cookie),
            Some(&[("dump_location", "/var/dumps/a"), ("intent", "dump")]),
        )
        .await;
    assert!(
        page.main().contains("admin API turned off"),
        "{}",
        page.main()
    );
}
