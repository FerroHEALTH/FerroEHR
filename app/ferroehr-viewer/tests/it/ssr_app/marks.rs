// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's Legal marks tab through the viewer's real service over the
//! replayed CDR: each register, a write of each kind the CDR answers, a
//! refusal in the CDR's words, and the answers of a `403` and a `405`.

#![expect(
    clippy::disallowed_types,
    reason = "a stub is raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::json;

use super::{Viewer, id};

#[tokio::test]
async fn the_tab_shows_each_register() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer
        .page(&cookie, &format!("/ehrs/{}/marks", id("ehr")))
        .await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in [
        "Restriction of processing",
        "Objection to research processing",
        "Access delay",
        "Retention",
        "Record the objection",
        "Hold from disposal",
        "CH",
    ] {
        assert!(
            page.main().contains(words),
            "says {words:?}:\n{}",
            page.main()
        );
    }
}

#[tokio::test]
async fn a_write_of_each_kind_returns_to_the_tab() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let held = id("composition_flat");
    let tab = format!("/ehrs/{ehr}/marks");
    for fields in [
        vec![("intent", "lift")],
        vec![("intent", "withdraw")],
        vec![("intent", "unhold"), ("vo_id", held.as_str())],
    ] {
        let answer = viewer
            .send(http::Method::POST, &tab, Some(&cookie), Some(&fields))
            .await;
        assert_eq!(
            answer.status,
            http::StatusCode::FOUND,
            "{fields:?}: {}",
            answer.body
        );
        assert_eq!(tab, answer.location(), "{fields:?}");
    }
}

#[tokio::test]
async fn a_refused_mark_is_drawn_in_the_cdrs_words() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let tab = format!("/ehrs/{}/marks", id("ehr"));
    let answer = viewer
        .send(
            http::Method::POST,
            &tab,
            Some(&cookie),
            Some(&[("intent", "restrict"), ("ground", "no-such-ground")]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK, "{}", answer.body);
    assert!(
        answer.main().contains("400 Bad Request"),
        "{}",
        answer.main()
    );
}

#[tokio::test]
async fn a_refused_account_and_an_admin_api_that_is_off_say_so() {
    for (status, words) in [
        (403, "Your account may not read the legal marks."),
        (405, "admin API is off"),
    ] {
        let refused = json!({
            "method": "GET",
            "path": format!("/ferroehr/rest/extensions/v1/admin/restriction?ehr_id={}", id("ehr")),
            "body": "",
            "status": status,
            "headers": { "content-type": "application/json" },
            "response": json!({ "status": status, "detail": "refused by the stub" }).to_string(),
        });
        let viewer = Viewer::start_with(vec![refused]).await;
        let cookie = viewer.sign_in().await;
        let page = viewer
            .page(&cookie, &format!("/ehrs/{}/marks", id("ehr")))
            .await;
        assert!(page.main().contains(words), "{status}:\n{}", page.main());
    }
}

#[tokio::test]
async fn a_held_composition_shows_its_hold_and_the_professional_releases_it() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let ehr = id("ehr");
    let held = id("composition_flat");
    let page = viewer
        .page(&cookie, &format!("/ehrs/{ehr}/compositions/{held}"))
        .await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert!(
        page.body.contains("Held from the person until"),
        "{}",
        page.body
    );
    let other = id("composition");
    let answer = viewer
        .send(
            http::Method::POST,
            &format!("/ehrs/{ehr}/compositions/{other}"),
            Some(&cookie),
            Some(&[("intent", "release-access-delay")]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::FOUND, "{}", answer.body);
}
