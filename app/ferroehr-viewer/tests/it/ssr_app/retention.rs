// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Retention through the viewer's real service over the replayed CDR: the
//! declared periods, a declaration and a refused one, what is due, and the
//! answers of a `403` and a `405`.

#![expect(
    clippy::disallowed_types,
    reason = "a stub is raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::json;

use super::{Viewer, id};

/// The fields of the seeded period.
const SEEDED: [(&str, &str); 6] = [
    ("intent", "declare"),
    ("kind", "EHR"),
    ("jurisdiction", "NL"),
    ("period", "P20Y"),
    ("anchor", "last_commit"),
    ("source", "WGBO art. 7:454 lid 3 (synthetic demo period)"),
];

#[tokio::test]
async fn the_register_and_what_is_due_are_listed() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/retention").await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in [
        "Declared periods",
        "WGBO art. 7:454 lid 3 (synthetic demo period)",
        "last_commit",
        "What is due",
        "Declare or change a period",
    ] {
        assert!(
            page.main().contains(words),
            "says {words:?}:\n{}",
            page.main()
        );
    }
    assert!(
        page.main().contains(&format!("/ehrs/{}", id("ehr_due"))),
        "the anchored EHR is due:\n{}",
        page.main()
    );
}

#[tokio::test]
async fn a_declaration_returns_to_the_page_and_a_refused_one_is_kept() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let answer = viewer
        .send(
            http::Method::POST,
            "/retention",
            Some(&cookie),
            Some(&SEEDED),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::FOUND, "{}", answer.body);
    assert_eq!("/retention", answer.location());
    let mut refused = SEEDED;
    refused[3] = ("period", "forever");
    let answer = viewer
        .send(
            http::Method::POST,
            "/retention",
            Some(&cookie),
            Some(&refused),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK, "{}", answer.body);
    assert!(
        answer.main().contains("400 Bad Request"),
        "{}",
        answer.main()
    );
    assert!(
        answer.main().contains("forever"),
        "kept as typed:\n{}",
        answer.main()
    );
}

#[tokio::test]
async fn a_refused_account_and_an_admin_api_that_is_off_say_so() {
    for (status, words) in [
        (403, "Your account may not read the retention register."),
        (405, "admin API is off"),
    ] {
        let refused = json!({
            "method": "GET",
            "path": "/ferroehr/rest/extensions/v1/admin/retention/policy",
            "body": "",
            "status": status,
            "headers": { "content-type": "application/json" },
            "response": json!({ "status": status, "detail": "refused by the stub" }).to_string(),
        });
        let viewer = Viewer::start_with(vec![refused]).await;
        let cookie = viewer.sign_in().await;
        let page = viewer.page(&cookie, "/retention").await;
        assert!(page.main().contains(words), "{status}:\n{}", page.main());
        assert!(
            !page.main().contains("Declare or change a period"),
            "{status}"
        );
    }
}
