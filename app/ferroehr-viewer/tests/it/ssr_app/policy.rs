// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Access policy through the viewer's real service over the replayed CDR:
//! the policy in force by section, its versions, a save the CDR refuses, and
//! the answers that show no policy.

#![expect(
    clippy::disallowed_types,
    reason = "a stub is raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::{Value, json};

use super::{Viewer, id};

/// A refusal of the policy read with `status`, in the CDR's error shape.
fn refused_read(status: u16) -> Value {
    json!({
        "method": "GET",
        "path": "/ferroehr/rest/extensions/v1/admin/access_policy",
        "body": "",
        "status": status,
        "headers": { "content-type": "application/json" },
        "response": json!({ "status": status, "detail": "refused by the stub" }).to_string(),
    })
}

/// The hidden version an editor's form carries.
fn opened_version(body: &str) -> String {
    let marker = r#"name="version" value=""#;
    let start = body.find(marker).map_or(0, |at| at + marker.len());
    let rest = body.get(start..).unwrap_or_default();
    rest.get(..rest.find('"').unwrap_or(0))
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn the_policy_in_force_is_drawn_by_section_with_its_coded_values() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/access-policy").await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in [
        "Demo access policy",
        "Care relationship facts",
        "Care relationship required",
        "P7D",
        "Concealed confidentialities",
        "very restricted",
        "HL7_Confidentiality",
        "Controller rules",
        "ACCESS_POLICY, version 2",
        "Edit the policy",
    ] {
        assert!(
            page.body.contains(words),
            "says {words:?}:\n{}",
            page.main()
        );
    }
}

#[tokio::test]
async fn the_versions_list_every_version_and_open_an_older_one_read_only() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let versions = viewer.page(&cookie, "/access-policy/versions").await;
    assert_eq!(versions.status, http::StatusCode::OK, "{}", versions.body);
    assert!(
        versions.main().contains("/access-policy?version=1"),
        "the older version opens from its row:\n{}",
        versions.main()
    );
    let older = viewer.page(&cookie, "/access-policy?version=1").await;
    assert_eq!(older.status, http::StatusCode::OK, "{}", older.body);
    assert!(
        older.main().contains("ACCESS_POLICY, version 1"),
        "{}",
        older.main()
    );
    assert!(
        !older.main().contains("Edit the policy"),
        "an older version offers no edit:\n{}",
        older.main()
    );
}

#[tokio::test]
async fn a_save_over_a_version_no_longer_newest_is_refused_with_412_and_kept() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let editor = viewer.page(&cookie, "/access-policy/edit").await;
    assert_eq!(editor.status, http::StatusCode::OK, "{}", editor.body);
    let current = opened_version(&editor.body);
    assert!(
        current.ends_with("::2"),
        "the editor opens the newest: {current}"
    );
    let stale = current.replace("::2", "::1");
    let archetype = "openEHR-EHR-ACCESS_POLICY.controller.v1";
    let document = json!({
        "_type": "ACCESS_POLICY",
        "name": { "_type": "DV_TEXT", "value": "Demo access policy, stale edit" },
        "archetype_node_id": archetype,
        "archetype_details": { "_type": "ARCHETYPED", "rm_version": "1.2.0",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype } },
        "controller": { "_type": "PARTY_REF", "namespace": "demographic", "type": "ORGANISATION",
            "id": { "_type": "HIER_OBJECT_ID", "value": id("organisation") } },
        "care_relationship_required": false,
        "purpose_required": false
    })
    .to_string();
    let answer = viewer
        .send(
            http::Method::POST,
            "/access-policy/edit",
            Some(&cookie),
            Some(&[
                ("version", stale.as_str()),
                ("document", document.as_str()),
                ("intent", "save"),
            ]),
        )
        .await;
    assert_eq!(answer.status, http::StatusCode::OK, "{}", answer.body);
    assert!(
        answer.main().contains("412 Precondition Failed"),
        "the CDR's refusal is drawn:\n{}",
        answer.main()
    );
    assert!(
        answer.main().contains("stale edit"),
        "the document is kept as typed:\n{}",
        answer.main()
    );
}

#[tokio::test]
async fn no_committed_policy_says_nothing_is_tightened_and_offers_the_first() {
    let viewer = Viewer::start_with(vec![refused_read(404)]).await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/access-policy").await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    assert!(
        page.main().contains("No access policy is committed"),
        "{}",
        page.main()
    );
    assert!(
        page.main().contains("Nothing is tightened"),
        "{}",
        page.main()
    );
    assert!(
        page.main().contains("Write the first version"),
        "{}",
        page.main()
    );
}

#[tokio::test]
async fn a_refused_account_and_an_admin_api_that_is_off_say_so() {
    for (status, words) in [
        (403, "Your account may not read the access policy."),
        (405, "admin API is off"),
    ] {
        let viewer = Viewer::start_with(vec![refused_read(status)]).await;
        let cookie = viewer.sign_in().await;
        let page = viewer.page(&cookie, "/access-policy").await;
        assert!(page.main().contains(words), "{status}:\n{}", page.main());
        assert!(
            !page.main().contains("Edit the policy"),
            "{status} offers no edit:\n{}",
            page.main()
        );
    }
}
