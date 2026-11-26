// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Roles and grants through the viewer's real service over the replayed CDR:
//! the roles holding a grant and the system agents, and the refusal of an
//! account the CDR does not let read them.

#![expect(
    clippy::disallowed_types,
    reason = "a stub is raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::json;

use super::{Viewer, id};

/// The link to a page starting `/parties/{kind}/` nearest before `words` in
/// `main`: the row link of the row `words` names.
fn party_link_before(main: &str, kind: &str, words: &str) -> String {
    let marker = format!("href=\"/parties/{kind}/");
    let end = main.find(words).unwrap_or(main.len());
    let before = main.get(..end).unwrap_or_default();
    let start = before.rfind(&marker).map_or(end, |at| at + "href=\"".len());
    let rest = main.get(start..).unwrap_or_default();
    rest.get(..rest.find('"').unwrap_or(0))
        .unwrap_or_default()
        .to_owned()
}

#[tokio::test]
async fn the_roles_holding_a_grant_and_the_system_agents_are_listed() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/grants").await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in [
        "Roles holding a grant",
        "write_access_policy",
        "read_change_feed",
        "hold_scopes",
        "In force",
        "System agents",
        "lab-system",
        "https://login.example.org/machines",
        "ORGANISATION",
        "/parties/role/",
        "/parties/agent/",
    ] {
        assert!(
            page.main().contains(words),
            "says {words:?}:\n{}",
            page.main()
        );
    }
}

#[tokio::test]
async fn an_account_the_cdr_refuses_reads_no_grant() {
    let mut refused = super::recording()
        .iter()
        .find(|exchange| {
            exchange["body"]
                .as_str()
                .is_some_and(|body| body.contains("FROM ROLE r CONTAINS CAPABILITY"))
        })
        .cloned()
        .expect("the recording holds the roles query");
    refused["status"] = json!(403);
    refused["response"] =
        json!(json!({ "status": 403, "detail": "refused by the stub" }).to_string());
    let viewer = Viewer::start_with(vec![refused]).await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/grants").await;
    assert!(
        page.main().contains("Your account may not read grants."),
        "{}",
        page.main()
    );
}

#[tokio::test]
async fn a_policy_party_is_marked_with_its_grants_and_an_ordinary_one_is_not() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let grants = viewer.page(&cookie, "/grants").await;
    let agent = party_link_before(grants.main(), "agent", ">lab-system<");
    assert!(
        !agent.is_empty(),
        "the agent links its party:\n{}",
        grants.main()
    );
    let page = viewer.page(&cookie, &agent).await;
    assert_eq!(page.status, http::StatusCode::OK, "{agent}: {}", page.body);
    for words in ["Policy party", "lab-system", "read_change_feed"] {
        assert!(
            page.main().contains(words),
            "{agent} says {words:?}:\n{}",
            page.main()
        );
    }
    let role = party_link_before(grants.main(), "role", ">write_access_policy<");
    let page = viewer.page(&cookie, &role).await;
    assert!(
        page.main().contains("Policy party"),
        "{role}:\n{}",
        page.main()
    );
    let ordinary = viewer
        .page(&cookie, &format!("/parties/person/{}", id("party")))
        .await;
    assert!(
        !ordinary.main().contains("Policy party"),
        "an ordinary party carries no mark:\n{}",
        ordinary.main()
    );
}
