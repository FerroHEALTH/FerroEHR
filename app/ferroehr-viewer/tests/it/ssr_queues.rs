// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Audit delivery and the identity feed over stubbed admin routes: both
//! lists, an act on each, and a queue the CDR does not serve (#6658).

#![expect(
    clippy::disallowed_types,
    reason = "the stubbed exchanges are raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::Value;

use crate::ssr_app::Viewer;

/// The extensions root the stubs answer under.
const EXT: &str = "/ferroehr/rest/extensions/v1";

/// A refused message's id.
const MESSAGE: &str = "0196f1f4-8c3a-7d2e-9b1a-3c5d7e9f1a2b";

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

/// The delivery view: one configured cursor, one sink no longer configured
/// and one parked record.
fn delivery() -> Value {
    exchange(
        "GET",
        &format!("{EXT}/admin/audit-delivery"),
        200,
        &serde_json::json!({
            "cursors": [
                {"sink": "fhir_feed", "chain": "system", "through_id": 42, "behind": 3, "configured": true},
                {"sink": "syslog", "chain": "access_log", "through_id": 7, "behind": 120, "configured": false}
            ],
            "parked": [
                {"sink": "fhir_feed", "chain": "access_log", "record_id": 19, "reason": "refused", "parked_at": "2026-11-16T09:15:01Z", "retry": false}
            ]
        })
        .to_string(),
    )
}

/// Delivery lists the cursors and the parked records, confirms a dismissal
/// above them, and goes back to the list once the CDR dismissed it.
#[tokio::test]
async fn delivery_lists_the_cursors_and_dismisses_a_parked_record() {
    let viewer = Viewer::start_with(vec![
        delivery(),
        exchange(
            "POST",
            &format!("{EXT}/admin/audit-delivery/dismiss"),
            204,
            "",
        ),
    ])
    .await;
    let cookie = viewer.sign_in().await;
    let audit = viewer.page(&cookie, "/audit").await;
    assert!(
        audit.body.contains(r#"href="/audit/delivery""#),
        "Audit links Delivery"
    );
    let page = viewer.page(&cookie, "/audit/delivery").await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    let main = page.main();
    for words in [
        "fhir_feed",
        "120",
        "Not configured",
        "refused",
        "confirm=dismiss",
        "confirm=forget",
    ] {
        assert!(main.contains(words), "Delivery shows {words}:\n{main}");
    }
    let confirm = viewer
        .page(
            &cookie,
            "/audit/delivery?confirm=dismiss&sink=fhir_feed&chain=access_log&record_id=19",
        )
        .await;
    assert!(
        confirm.main().contains(r#"name="intent" value="dismiss""#),
        "{}",
        confirm.main()
    );
    let done = viewer
        .send(
            http::Method::POST,
            "/audit/delivery",
            Some(&cookie),
            Some(&[
                ("intent", "dismiss"),
                ("sink", "fhir_feed"),
                ("chain", "access_log"),
                ("record_id", "19"),
            ]),
        )
        .await;
    assert_eq!(done.location(), "/audit/delivery");
}

/// A CDR that serves no delivery view says so.
#[tokio::test]
async fn delivery_without_a_queue_says_so() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/audit/delivery").await;
    assert!(
        page.main().contains("serves no view of its audit delivery"),
        "{}",
        page.main()
    );
}

/// The identity feed lists its refused messages, retries one, and draws a
/// refused settlement in the CDR's words.
#[tokio::test]
async fn the_identity_feed_retries_and_settles_its_messages() {
    let listed = serde_json::json!({"messages": [{
        "message_id": MESSAGE, "reason": "unknown-domain-type",
        "received_at": "2026-11-16T09:15:00Z", "refused_at": "2026-11-16T09:15:01Z",
        "attempts": 1, "identities": ["x"],
        "splits": [{"person": "p", "identifiers": 1, "subjects": 0}]
    }]})
    .to_string();
    let viewer = Viewer::start_with(vec![
        exchange("GET", &format!("{EXT}/admin/pmir/messages"), 200, &listed),
        exchange(
            "POST",
            &format!("{EXT}/admin/pmir/messages/{MESSAGE}/retry"),
            202,
            r#"{"state":"received"}"#,
        ),
        exchange(
            "POST",
            &format!("{EXT}/admin/pmir/messages/{MESSAGE}/settle"),
            400,
            r#"{"title":"Bad Request","detail":"the reason is missing"}"#,
        ),
    ])
    .await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/server/identity").await;
    assert_eq!(page.status, http::StatusCode::OK, "{}", page.body);
    for words in [MESSAGE, "unknown-domain-type", "confirm=settle"] {
        assert!(
            page.main().contains(words),
            "the feed shows {words}:\n{}",
            page.main()
        );
    }
    let settle = viewer
        .page(
            &cookie,
            &format!("/server/identity?confirm=settle&message={MESSAGE}"),
        )
        .await;
    assert!(
        settle.main().contains(r#"name="decision""#),
        "{}",
        settle.main()
    );
    let retried = viewer
        .send(
            http::Method::POST,
            "/server/identity",
            Some(&cookie),
            Some(&[("intent", "retry"), ("message", MESSAGE)]),
        )
        .await;
    assert_eq!(retried.location(), "/server/identity");
    let refused = viewer
        .send(
            http::Method::POST,
            "/server/identity",
            Some(&cookie),
            Some(&[
                ("intent", "settle"),
                ("message", MESSAGE),
                ("decision", "discard"),
                ("reason", ""),
            ]),
        )
        .await;
    assert!(
        refused.main().contains("the reason is missing"),
        "{}",
        refused.main()
    );
}

/// A CDR that runs no identity feed says so.
#[tokio::test]
async fn an_absent_identity_feed_says_so() {
    let viewer = Viewer::start().await;
    let cookie = viewer.sign_in().await;
    let page = viewer.page(&cookie, "/server/identity").await;
    assert!(
        page.main().contains("runs no patient identity feed"),
        "{}",
        page.main()
    );
}
