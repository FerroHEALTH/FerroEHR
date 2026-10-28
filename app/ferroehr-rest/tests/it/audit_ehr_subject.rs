// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every access to one EHR is recorded against that EHR and the user, end to
//! end over the assembled router, the audit sender and the local Audit Record
//! Repository.
//!
//! A person asking who accessed their record (GDPR Art. 15(1)(c),
//! `docs/law/eu/gdpr/text.html`; EHDS Art. 9(2), `docs/law/eu/ehds/text.html`)
//! is answered by the subject-scoped ITI-81 retrieval, which finds a record
//! only when it names the EHR. An export names its EHR after `/message/export/`
//! rather than `/ehr/`, and its Extract record is written by the service layer,
//! where the user came from no principal; both now name the EHR and the user. A
//! read of the restriction register names the EHR in a query parameter and is
//! recorded by the service with the EHR, as the access-restriction register is.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::time::Duration;

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode, header};
use serde_json::Value;
use sqlx::PgPool;

use crate::common;

/// The synthetic patient subject every EHR of the test resolves to.
const SUBJECT: &str = "patient-subject-1";
/// The synthetic user every request of the test authenticates as.
const USER: &str = "archivist";

/// A `GET` of `path` under the base path, as the test's user.
fn get(path: &str) -> Request<Body> {
    Request::get(common::full(path))
        .header(header::AUTHORIZATION, common::basic_credential(USER))
        .body(Body::empty())
        .unwrap()
}

/// An audited app with the local store on and one EHR; returns the EHR id.
async fn app() -> (testkit::TestDb, PgPool, Router, String) {
    common::audited_app(SUBJECT, USER).await
}

/// One stored record: its operation, resource class, resource id, principal
/// and patient.
type Row = (
    Option<String>,
    String,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The stored records `matches` selects, polled until `expected` have landed.
async fn stored(pool: &PgPool, expected: usize, matches: impl Fn(&Row) -> bool) -> Vec<Row> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let rows: Vec<Row> = sqlx::query_as(
            "SELECT operation, resource_class, resource_id, principal, patient_id \
             FROM audit.audit_event ORDER BY chain_seq",
        )
        .fetch_all(pool)
        .await
        .expect("read the trail");
        let rows: Vec<Row> = rows.into_iter().filter(|row| matches(row)).collect();
        if rows.len() >= expected {
            return rows;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {} of {expected} records landed",
            rows.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// The resources the subject-scoped ITI-81 retrieval answers that `matches`
/// selects, polled until `expected` have landed.
async fn retrieved(app: &Router, expected: usize, matches: impl Fn(&Value) -> bool) -> Vec<Value> {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let (status, body) = common::send_body(
            app,
            get(&format!(
                "/fhir/r4/AuditEvent?date=ge2026-01-01&patient={SUBJECT}"
            )),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "subject-scoped retrieval: {body}");
        let bundle: Value = serde_json::from_str(&body).expect("bundle json");
        let found: Vec<Value> = bundle["entry"]
            .as_array()
            .map(|entries| {
                entries
                    .iter()
                    .map(|entry| entry["resource"].clone())
                    .filter(|resource| matches(resource))
                    .collect()
            })
            .unwrap_or_default();
        if found.len() >= expected {
            return found;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "only {} of {expected} records answered: {bundle}",
            found.len()
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// Whether `resource` carries the REST operation `op` among its subtypes.
fn has_operation(resource: &Value, op: &str) -> bool {
    resource["subtype"]
        .as_array()
        .is_some_and(|subtypes| subtypes.iter().any(|coding| coding["code"] == op))
}

/// The requesting agent's user identifier of `resource`.
fn requestor(resource: &Value) -> Value {
    resource["agent"]
        .as_array()
        .expect("agents")
        .iter()
        .find(|agent| agent["requestor"] == true)
        .expect("an access record names its requestor")["who"]["identifier"]["value"]
        .clone()
}

/// The REST record of an export names the EHR and the user, the Extract
/// record names the user, and the subject-scoped ITI-81 retrieval answers both.
#[tokio::test]
async fn an_export_is_recorded_against_its_ehr_and_its_user() {
    let (_db, pool, app, ehr) = app().await;
    let (status, body) = common::send_body(&app, get(&format!("/message/export/{ehr}"))).await;
    assert_eq!(status, StatusCode::OK, "export: {body}");

    let operation = stored(&pool, 1, |row| {
        row.0.as_deref() == Some("message_export_ehrs")
    })
    .await;
    assert_eq!(operation[0].3.as_deref(), Some(USER), "{operation:?}");
    assert_eq!(operation[0].4.as_deref(), Some(SUBJECT), "{operation:?}");

    let extract = stored(&pool, 1, |row| row.1 == "extract").await;
    assert_eq!(extract[0].2.as_deref(), Some(ehr.as_str()), "{extract:?}");
    assert_eq!(extract[0].3.as_deref(), Some(USER), "{extract:?}");
    assert_eq!(extract[0].4.as_deref(), Some(SUBJECT), "{extract:?}");

    let answered = retrieved(&app, 1, |r| has_operation(r, "message_export_ehrs")).await;
    assert_eq!(requestor(&answered[0]), Value::from(USER));
    let answered = retrieved(&app, 1, |r| {
        r["entity"]
            .as_array()
            .is_some_and(|e| e.iter().any(|entity| entity["name"] == "Export"))
            && !has_operation(r, "message_export_ehrs")
    })
    .await;
    assert_eq!(requestor(&answered[0]), Value::from(USER));
}

/// A read of the restriction register is recorded against the EHR it names
/// in its query, with the user, and the subject-scoped retrieval answers it.
#[tokio::test]
async fn a_restriction_register_read_is_recorded_against_its_ehr() {
    let (_db, pool, app, ehr) = app().await;
    let (status, body) =
        common::send_body(&app, get(&format!("/admin/restriction?ehr_id={ehr}"))).await;
    assert_eq!(status, StatusCode::OK, "register: {body}");

    let read = stored(&pool, 1, |row| {
        row.2.as_deref() == Some("restriction:ehr-wide")
    })
    .await;
    assert_eq!(read[0].3.as_deref(), Some(USER), "{read:?}");
    assert_eq!(read[0].4.as_deref(), Some(SUBJECT), "{read:?}");

    let answered = retrieved(&app, 1, |r| {
        r["entity"].as_array().is_some_and(|e| {
            e.iter()
                .any(|entity| entity["what"]["identifier"]["value"] == "restriction:ehr-wide")
        })
    })
    .await;
    assert_eq!(requestor(&answered[0]), Value::from(USER));
}
