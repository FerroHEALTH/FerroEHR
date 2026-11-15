// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The coded filters of the access log's ITS-REST resources (#5687): `action`,
//! `outcome` and `accessor_kind` take the FHIR token `terminology_id|code`
//! (FHIR R4 search.html §3.1.1.4.10) and select by the term's
//! `defining_code`, an HL7 code and an `openehr_access` code alike; a rubric,
//! a bare code, an unknown terminology or a code outside the group is a
//! `400`. Over the assembled router, the audit drain and the local store on a
//! real `PostgreSQL` 18. No openEHR spec governs the access log's filters:
//! our own design.

use axum::Router;
use http::StatusCode;
use serde_json::{Value, json};

use crate::access_log_reads_http::{Fixture, fixture, get, root, stored, token_with_role};
use crate::common::{self, BASE, EXT};

/// The page of `ehr`'s log under `query`, read by the administrator.
async fn page(app: &Router, ehr: &str, query: &str) -> Vec<Value> {
    let (status, page) = get(app, &format!("{EXT}/ehr/{ehr}/access_log?{query}"), &root()).await;
    assert_eq!(status, StatusCode::OK, "{query}: {page}");
    page["entries"].as_array().cloned().unwrap_or_default()
}

/// The `(terminology id, code)` of the coded value at `path` of `entry`.
fn code_at<'a>(entry: &'a Value, path: &[&str]) -> (&'a str, &'a str) {
    let coded = path.iter().fold(entry, |value, step| &value[step]);
    let code = &coded["defining_code"];
    (
        code["terminology_id"]["value"].as_str().unwrap_or_default(),
        code["code_string"].as_str().unwrap_or_default(),
    )
}

/// Runs an AQL query scoped to `ehr` as the administrator, so its log holds
/// a `query` action.
async fn query(fx: &Fixture) {
    let (status, body) = common::send_body(
        &fx.app,
        common::req("POST", format!("{BASE}/query/aql"))
            .auth(root())
            .header("openehr-ehr-id", fx.ehr.as_str())
            .json(
                json!({ "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c" }).to_string(),
            )
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    stored(&fx.pool, 1, |entry| {
        entry["ehr_id"]["value"] == json!(fx.ehr)
            && code_at(entry, &["action"]) == ("openehr_access", "query")
    })
    .await;
}

/// `action` selects an HL7 `audit-event-action` code and an `openehr_access`
/// code by their coded content, and a code no entry carries selects none.
#[tokio::test]
async fn the_action_filter_selects_by_the_coded_term() {
    let fx = fixture().await;
    query(&fx).await;

    let read = page(&fx.app, &fx.ehr, "action=audit-event-action%7CR").await;
    assert!(!read.is_empty(), "the administrator's read is selected");
    assert!(
        read.iter()
            .all(|entry| code_at(entry, &["action"]) == ("audit-event-action", "R")),
        "{read:#?}"
    );

    let queried = page(&fx.app, &fx.ehr, "action=openehr_access%7Cquery").await;
    assert!(!queried.is_empty(), "the query is selected");
    assert!(
        queried
            .iter()
            .all(|entry| code_at(entry, &["action"]) == ("openehr_access", "query")),
        "{queried:#?}"
    );

    let deleted = page(&fx.app, &fx.ehr, "action=audit-event-action%7CD").await;
    assert!(deleted.is_empty(), "no entry deleted: {deleted:#?}");
}

/// `outcome` selects a granted and a refused access on an EHR's log and on
/// the review, and an outcome no entry carries selects none.
#[tokio::test]
async fn the_outcome_filter_selects_by_the_coded_term() {
    let fx = fixture().await;
    let dr = common::basic_credential("dr");
    let (status, body) = get(&fx.app, &format!("{EXT}/ehr/{}/access_log", fx.ehr), &dr).await;
    assert_eq!(status, StatusCode::FORBIDDEN, "{body}");
    stored(&fx.pool, 1, |entry| {
        entry["ehr_id"]["value"] == json!(fx.ehr)
            && code_at(entry, &["outcome", "result"]) == ("openehr_access", "refused")
    })
    .await;

    for (value, code) in [
        ("openehr_access%7Cgranted", "granted"),
        ("openehr_access%7Crefused", "refused"),
    ] {
        let selected = page(&fx.app, &fx.ehr, &format!("outcome={value}")).await;
        assert!(!selected.is_empty(), "{value}");
        assert!(
            selected
                .iter()
                .all(|entry| code_at(entry, &["outcome", "result"]) == ("openehr_access", code)),
            "{value}: {selected:#?}"
        );
    }
    let failed = page(&fx.app, &fx.ehr, "outcome=openehr_access%7Cfailed").await;
    assert!(failed.is_empty(), "no access failed: {failed:#?}");

    let review =
        |outcome: &str| format!("{EXT}/admin/access-log?ehr_id={}&outcome={outcome}", fx.ehr);
    let (status, refused) = get(&fx.app, &review("openehr_access%7Crefused"), &root()).await;
    assert_eq!(status, StatusCode::OK, "{refused}");
    let refused = refused["entries"].as_array().cloned().unwrap_or_default();
    assert!(!refused.is_empty(), "the review selects the refusal");
    assert!(
        refused
            .iter()
            .all(|entry| code_at(entry, &["outcome", "result"]) == ("openehr_access", "refused")),
        "{refused:#?}"
    );
    let (status, failed) = get(&fx.app, &review("openehr_access%7Cfailed"), &root()).await;
    assert_eq!(status, StatusCode::OK, "{failed}");
    assert_eq!(failed["entries"], json!([]), "{failed}");
}

/// `accessor_kind` selects the HL7 `extra-security-role-type` code of the
/// subject of care, whose own reads of their log `include_self` alone shows,
/// and an `openehr_access` kind; a kind no entry carries selects none.
#[tokio::test]
async fn the_accessor_kind_filter_selects_by_the_coded_term() {
    let fx = fixture().await;
    let patient = token_with_role(None, &fx.ehr);
    let (status, body) = get(
        &fx.app,
        &format!("{EXT}/ehr/{}/access_log", fx.ehr),
        &patient,
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    stored(&fx.pool, 1, |entry| {
        entry["ehr_id"]["value"] == json!(fx.ehr)
            && code_at(entry, &["accessor", "kind"]) == ("extra-security-role-type", "datasubject")
    })
    .await;

    let subject = "accessor_kind=extra-security-role-type%7Cdatasubject";
    let hidden = page(&fx.app, &fx.ehr, subject).await;
    assert!(
        hidden.is_empty(),
        "the subject's own log reads: {hidden:#?}"
    );
    let shown = page(&fx.app, &fx.ehr, &format!("{subject}&include_self=true")).await;
    assert!(!shown.is_empty(), "include_self shows the subject's read");
    assert!(
        shown
            .iter()
            .all(|entry| code_at(entry, &["accessor", "kind"])
                == ("extra-security-role-type", "datasubject")),
        "{shown:#?}"
    );

    let person = page(&fx.app, &fx.ehr, "accessor_kind=openehr_access%7Cperson").await;
    assert!(!person.is_empty(), "the administrator is a person");
    assert!(
        person
            .iter()
            .all(|entry| code_at(entry, &["accessor", "kind"]) == ("openehr_access", "person")),
        "{person:#?}"
    );
    let system = page(&fx.app, &fx.ehr, "accessor_kind=openehr_access%7Csystem").await;
    assert!(system.is_empty(), "no system accessed: {system:#?}");
}

/// A rubric, a bare code, a terminology the group holds no term of, a code
/// outside the group and a code-less token are each a `400` naming the
/// parameter, on an EHR's log and on the review.
#[tokio::test]
async fn a_coded_filter_outside_its_form_or_group_is_refused() {
    let fx = fixture().await;
    let log = format!("{EXT}/ehr/{}/access_log", fx.ehr);
    let review = format!("{EXT}/admin/access-log");
    let cases: &[(&str, &str, &[&str])] = &[
        (
            &log,
            "action",
            &[
                "read",
                "R",
                "%7CR",
                "audit-event-action%7C",
                "urn%3Aexample%7CR",
                "http%3A%2F%2Fhl7.org%2Ffhir%2Faudit-event-action%7CR",
                "audit-event-action%7CE",
                "openehr_access%7Cread",
                "openehr_access%7Cgranted",
            ],
        ),
        (
            &log,
            "outcome",
            &[
                "granted",
                "refused",
                "urn%3Aexample%7Cgranted",
                "openehr_access%7Cdenied",
                "openehr_access%7Cquery",
            ],
        ),
        (
            &log,
            "accessor_kind",
            &[
                "subject%20of%20care",
                "datasubject",
                "urn%3Aexample%7Cperson",
                "extra-security-role-type%7Cdatasubjects",
                "openehr_access%7Cdatasubject",
            ],
        ),
        (
            &review,
            "outcome",
            &["granted", "openehr_access%7Cread", "audit-event-action%7CR"],
        ),
    ];
    for (resource, name, values) in cases {
        for value in *values {
            let uri = format!("{resource}?{name}={value}");
            let (status, body) = get(&fx.app, &uri, &root()).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{uri}: {body}");
            assert!(body.to_string().contains(name), "{uri}: {body}");
        }
    }
}

/// A coded filter pages: the pages under it partition the entries it
/// selects, newest first, and leave out the entries it does not select.
#[tokio::test]
async fn a_coded_filter_pages() {
    let fx = fixture().await;
    for _ in 0..3 {
        let (status, body) = get(&fx.app, &format!("{BASE}/ehr/{}", fx.ehr), &root()).await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }
    stored(&fx.pool, 4, |entry| {
        entry["ehr_id"]["value"] == json!(fx.ehr)
            && code_at(entry, &["action"]) == ("audit-event-action", "R")
    })
    .await;
    query(&fx).await;
    let until = jiff::Timestamp::now();
    let bounded = format!("to={until}&action=audit-event-action%7CR");

    let every = page(&fx.app, &fx.ehr, &bounded).await;
    assert!(every.len() >= 4, "{every:#?}");
    assert!(
        every
            .iter()
            .all(|entry| code_at(entry, &["action"]) == ("audit-event-action", "R")),
        "the query is left out: {every:#?}"
    );
    let first = page(&fx.app, &fx.ehr, &format!("{bounded}&fetch=2")).await;
    let rest = page(&fx.app, &fx.ehr, &format!("{bounded}&offset=2&fetch=1000")).await;
    assert_eq!(first.len(), 2, "{first:#?}");
    let paged: Vec<&Value> = first
        .iter()
        .chain(&rest)
        .map(|entry| &entry["uid"])
        .collect();
    let whole: Vec<&Value> = every.iter().map(|entry| &entry["uid"]).collect();
    assert_eq!(paged, whole, "the pages partition the selection in order");
}
