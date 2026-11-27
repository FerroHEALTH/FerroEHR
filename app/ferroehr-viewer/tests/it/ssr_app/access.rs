// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Access settings tab through the viewer's real service over the
//! replayed CDR: the settings by directive, their versions, a commit over a
//! version no longer current, a repair of settings that parse, and the
//! answer of a `403`.

#![expect(
    clippy::disallowed_types,
    reason = "a stub is raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::json;

use super::{Viewer, id};

/// The settings a commit sends: open, with no directive.
const OPEN: &str = r#"{"_type":"ACCESS_DIRECTIVE_SETTINGS","default_access":{"_type":"DV_CODED_TEXT","value":"open","defining_code":{"_type":"CODE_PHRASE","terminology_id":{"_type":"TERMINOLOGY_ID","value":"openehr_access"},"code_string":"open"}}}"#;

/// The first version link of the tab.
fn first_version(main: &str) -> String {
    let marker = "?version=";
    let start = main.find(marker).map_or(0, |at| at + marker.len());
    let rest = main.get(start..).unwrap_or_default();
    let end = rest.find('"').unwrap_or(0);
    urlencoding::decode(rest.get(..end).unwrap_or_default())
        .map(std::borrow::Cow::into_owned)
        .unwrap_or_default()
}

#[tokio::test]
async fn the_settings_are_drawn_by_directive_with_their_versions() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let tab = format!("/ehrs/{}/access", id("ehr_access"));
    let page = viewer.page(&cookie, &tab).await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in [
        "These settings",
        "Consent for the cardiology ward",
        "consent",
        "permit",
        "Versions",
        "Commit new settings",
    ] {
        assert!(
            page.main().contains(words),
            "says {words:?}:\n{}",
            page.main()
        );
    }
    let older = first_version(page.main());
    assert!(
        !older.is_empty(),
        "an older version opens:\n{}",
        page.main()
    );
    let page = viewer
        .page(
            &cookie,
            &format!("{tab}?version={}", urlencoding::encode(&older)),
        )
        .await;
    assert!(
        page.main().contains("An older version of the settings"),
        "{}",
        page.main()
    );
}

#[tokio::test]
async fn a_commit_over_a_version_no_longer_current_is_refused_and_kept() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let tab = format!("/ehrs/{}/access", id("ehr_access"));
    let older = first_version(viewer.page(&cookie, &tab).await.main());
    let answer = viewer
        .send(
            http::Method::POST,
            &format!("{tab}/edit"),
            Some(&cookie),
            Some(&[
                ("version", older.as_str()),
                ("document", OPEN),
                ("intent", "save"),
            ]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK, "{}", answer.body);
    assert!(
        answer.main().contains("412 Precondition Failed"),
        "{}",
        answer.main()
    );
}

#[tokio::test]
async fn a_repair_of_settings_that_parse_is_refused_in_the_cdrs_words() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let tab = format!("/ehrs/{}/access", id("ehr_access"));
    let answer = viewer
        .send(
            http::Method::POST,
            &format!("{tab}/edit"),
            Some(&cookie),
            Some(&[("repair", "on"), ("document", OPEN), ("intent", "save")]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK, "{}", answer.body);
    assert!(answer.main().contains("409 Conflict"), "{}", answer.main());
}

#[tokio::test]
async fn a_refused_account_sees_the_refusal() {
    let refused = json!({
        "method": "GET",
        "path": format!("/ferroehr/rest/extensions/v1/ehr/{}/ehr_access", id("ehr_access")),
        "body": "",
        "status": 403,
        "headers": { "content-type": "application/json" },
        "response": json!({ "status": 403, "detail": "refused by the stub" }).to_string(),
    });
    let viewer = Viewer::start_with(vec![refused]).await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .page(&cookie, &format!("/ehrs/{}/access", id("ehr_access")))
        .await;
    assert!(
        page.main()
            .contains("Your account may not read these access settings."),
        "{}",
        page.main()
    );
}
