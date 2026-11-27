// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The admin registers through the viewer's real service over the replayed
//! CDR: a register drawn by its members, its history back to version 0, an
//! older version, a save refused with `412`, and the refusals of a `403`
//! and a `405`.

#![expect(
    clippy::disallowed_types,
    reason = "a stub is raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::json;

use super::Viewer;

#[tokio::test]
async fn the_access_delay_is_drawn_by_its_members_with_its_history() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .page(&cookie, "/access-policy/registers/access-delay")
        .await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in [
        "Access delay",
        "Enabled",
        "Laboratory-Reports",
        "3d",
        "The default, which no one committed",
        "Edit the register",
    ] {
        assert!(
            page.main().contains(words),
            "says {words:?}:\n{}",
            page.main()
        );
    }
    let older = viewer
        .page(&cookie, "/access-policy/registers/access-delay?version=0")
        .await;
    assert!(
        older.main().contains("An older version of the register"),
        "{}",
        older.main()
    );
    assert!(
        !older.main().contains("Edit the register"),
        "{}",
        older.main()
    );
}

#[tokio::test]
async fn the_audit_category_map_reads_its_default() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .page(&cookie, "/access-policy/registers/audit-categories")
        .await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert!(page.main().contains("Audit categories"), "{}", page.main());
}

#[tokio::test]
async fn a_save_over_a_version_no_longer_newest_is_refused_with_412() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let answer = viewer
        .send(
            http::Method::POST,
            "/access-policy/registers/access-delay/edit",
            Some(&cookie),
            Some(&[
                ("version", "0"),
                (
                    "document",
                    r#"{"enabled":true,"categories":["Laboratory-Reports"],"max_delay":"48h"}"#,
                ),
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
    assert!(
        answer.main().contains("48h"),
        "kept as typed:\n{}",
        answer.main()
    );
}

#[tokio::test]
async fn a_refused_account_and_an_admin_api_that_is_off_say_so() {
    for (status, words) in [
        (403, "Your account may not read this register."),
        (405, "admin API is off"),
    ] {
        let refused = json!({
            "method": "GET",
            "path": "/ferroehr/rest/extensions/v1/admin/registers/access-delay",
            "body": "",
            "status": status,
            "headers": { "content-type": "application/json" },
            "response": json!({ "status": status, "detail": "refused by the stub" }).to_string(),
        });
        let viewer = Viewer::start_with(vec![refused]).await;
        let cookie = viewer.sign_in().await;
        let page = viewer
            .page(&cookie, "/access-policy/registers/access-delay")
            .await;
        assert!(page.main().contains(words), "{status}:\n{}", page.main());
    }
}
