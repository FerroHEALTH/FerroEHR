// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An EHR's access log through the viewer's real service over the replayed
//! CDR: the note on the person's own view, drawn from the view the CDR names.

#![expect(
    clippy::disallowed_types,
    reason = "a stub is raw JSON by the testing rule (.claude/rules/testing.md \
              §Test-fixture construction)"
)]

use serde_json::{Value, json};

use super::{Viewer, id};

/// The words of the note on the person's own view.
const OWN_VIEW: &str = "own view of their access log, as the CDR serves it";

/// The uid of the stubbed entry.
const UID: &str = "01a21788-af74-70a3-8d1b-0283473df148";

/// One entry in the subject view: no credential, channel or binding.
fn subject_entry() -> Value {
    json!({
        "_type": "ACCESS_LOG_ENTRY",
        "uid": {"value": UID},
        "time_recorded": {"value": "2026-11-26T11:06:03.504176Z"},
        "action": {"value": "read", "defining_code": {
            "terminology_id": {"value": "HL7_AuditEventAction"}, "code_string": "R"}},
        "outcome": {"result": {"value": "granted", "defining_code": {
            "terminology_id": {"value": "openehr_access"}, "code_string": "granted"}}},
        "accessor": {"kind": {"value": "person", "defining_code": {
            "terminology_id": {"value": "openehr_access"}, "code_string": "person"}}}
    })
}

/// A `GET` of `path` answered `200` with `response`, naming the subject view.
fn subject_view(path: &str, response: &Value) -> Value {
    json!({
        "method": "GET",
        "path": path,
        "body": "",
        "status": 200,
        "headers": {
            "content-type": "application/json",
            "ferroehr-access-log-view": "subject",
        },
        "response": response.to_string(),
    })
}

#[tokio::test]
async fn the_subject_view_says_it_is_the_persons_own_log() {
    let ehr = id("ehr");
    let log = format!("/ferroehr/rest/extensions/v1/ehr/{ehr}/access_log");
    let list = format!("{log}?offset=0&fetch=25");

    let full = Viewer::start().await;
    let cookie = full.sign_in().await;
    let tab = full.page(&cookie, &format!("/ehrs/{ehr}/access-log")).await;
    assert_eq!(tab.status, http::StatusCode::OK, "{}", tab.body);
    assert!(
        !tab.main().contains(OWN_VIEW),
        "the full view: {}",
        tab.main()
    );

    let page = json!({"offset": 0, "fetch": 25, "entries": [subject_entry()]});
    let subject = Viewer::start_with(vec![
        subject_view(&list, &page),
        subject_view(&format!("{log}/{UID}"), &subject_entry()),
    ])
    .await;
    let cookie = subject.sign_in().await;
    let tab = subject
        .page(&cookie, &format!("/ehrs/{ehr}/access-log"))
        .await;
    assert_eq!(tab.status, http::StatusCode::OK, "{}", tab.body);
    assert!(tab.main().contains(OWN_VIEW), "{}", tab.main());
    let one = subject
        .page(&cookie, &format!("/ehrs/{ehr}/access-log/{UID}"))
        .await;
    assert_eq!(one.status, http::StatusCode::OK, "{}", one.body);
    assert!(one.main().contains(OWN_VIEW), "{}", one.main());
}
