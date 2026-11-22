// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The admin routes over the delivery of the audit records (#6242): the
//! view lists each sink's cursors and the records it parked, a retry and a
//! dismissal act on a parked record, a sink is forgotten, and every act is
//! recorded in the system log naming its operator. No openEHR spec governs
//! audit delivery: our own design.

use http::StatusCode;
use serde_json::{Value, json};

use crate::access_log_reads_http::{fixture, get, root};
use crate::common::{self, EXT};

/// The status and JSON body of `POST uri` with `body` as the administrator.
async fn post(app: &axum::Router, uri: &str, body: &Value) -> (StatusCode, Value) {
    let (status, text) = common::send_body(
        app,
        common::req("POST", uri)
            .auth(root())
            .accept("application/json")
            .json(body.to_string())
            .build(),
    )
    .await;
    (status, serde_json::from_str(&text).unwrap_or(Value::Null))
}

#[tokio::test]
async fn an_operator_reads_retries_dismisses_and_forgets_and_each_act_is_recorded() {
    let fx = fixture().await;
    sqlx::query(
        "INSERT INTO audit.delivery_cursor (sink, chain, through_id) \
         VALUES ('syslog', 'system', 0), ('syslog', 'access_log', 0)",
    )
    .execute(&fx.pool)
    .await
    .expect("a syslog cursor the configuration does not name");
    sqlx::query(
        "INSERT INTO audit.delivery_parked (sink, chain, record_id, reason) \
         VALUES ('syslog', 'system', 7, 'refused'), ('syslog', 'system', 8, 'unrenderable')",
    )
    .execute(&fx.pool)
    .await
    .expect("two parked records");

    let (status, view) = get(&fx.app, &format!("{EXT}/admin/audit-delivery"), &root()).await;
    assert_eq!(status, StatusCode::OK, "{view}");
    assert_eq!(view["parked"].as_array().map(Vec::len), Some(2), "{view}");
    assert!(
        view["cursors"].as_array().is_some_and(|cursors| cursors
            .iter()
            .any(|c| c["sink"] == "syslog" && c["configured"] == json!(false))),
        "the unconfigured sink's cursor is shown: {view}"
    );

    let retry = format!("{EXT}/admin/audit-delivery/retry");
    let (status, body) = post(
        &fx.app,
        &retry,
        &json!({ "sink": "syslog", "chain": "system", "record_id": 7 }),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "{body}");
    let retried: bool = sqlx::query_scalar(
        "SELECT retry FROM audit.delivery_parked WHERE sink = 'syslog' AND record_id = 7",
    )
    .fetch_one(&fx.pool)
    .await
    .expect("the parked record");
    assert!(retried, "the retry is asked");

    let dismiss = format!("{EXT}/admin/audit-delivery/dismiss");
    let parked = json!({ "sink": "syslog", "chain": "system", "record_id": 8 });
    assert_eq!(
        post(&fx.app, &dismiss, &parked).await.0,
        StatusCode::NO_CONTENT
    );
    assert_eq!(
        post(&fx.app, &dismiss, &parked).await.0,
        StatusCode::NOT_FOUND,
        "a dismissed record is parked no more"
    );
    assert_eq!(
        post(
            &fx.app,
            &retry,
            &json!({ "sink": "pager", "chain": "system", "record_id": 7 })
        )
        .await
        .0,
        StatusCode::BAD_REQUEST,
        "a name of no sink is refused"
    );

    let (status, forgotten) = post(
        &fx.app,
        &format!("{EXT}/admin/audit-delivery/forget"),
        &json!({ "sink": "syslog" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{forgotten}");
    assert_eq!(forgotten, json!({ "cursors": 2, "parked": 1 }));

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let wanted = [
        "admin_audit_delivery",
        "admin_retry_parked_audit_record",
        "admin_dismiss_parked_audit_record",
        "admin_forget_audit_sink",
    ];
    loop {
        let recorded: Vec<String> = sqlx::query_scalar(
            "SELECT DISTINCT operation FROM audit.audit_event \
             WHERE principal = 'root' AND operation LIKE 'admin_%audit%'",
        )
        .fetch_all(&fx.pool)
        .await
        .expect("the system log");
        if wanted.iter().all(|op| recorded.iter().any(|r| r == op)) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "every act is recorded naming its operator: {recorded:?}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
}
