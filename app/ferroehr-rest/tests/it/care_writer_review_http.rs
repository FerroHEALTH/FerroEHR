// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A care relationship written to a marked record, at the wire (#6141): a
//! writer `relationship_writers` names for the record's level writes it, and
//! the write's access-log record is flagged for review; one to a record at
//! `N` carries no flag, and a writer no entry names is refused `403` (draft
//! access control, The Decision Procedure, Writing Relationships, items 1
//! and 2).

use std::time::{Duration, Instant};

use axum::Router;
use http::StatusCode;
use serde_json::{Value, json};
use sqlx::PgPool;

use crate::access_confidentiality_http::{marked, put_policy};
use crate::access_evaluator_http::{app_with, send};
use crate::common;
use crate::common::{BASE, EXT};

/// A coded term of `terminology`.
fn coded(terminology: &str, code: &str) -> Value {
    json!({ "_type": "DV_CODED_TEXT", "value": code,
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology },
                "code_string": code } })
}

/// A term of the access control groups.
fn access(code: &str) -> Value {
    coded(ferroehr::party_binding::policy_mark::TERMINOLOGY, code)
}

/// A `PERSON` the administrator creates; its container.
async fn person(app: &Router) -> String {
    let req = common::req("POST", format!("{BASE}/demographic/person"))
        .auth(crate::access_evaluator_http::credential("root"))
        .prefer_representation()
        .json(crate::demographic_http::person_body().to_string())
        .build();
    let (status, headers, body) = common::send(app, req).await;
    assert_eq!(status, StatusCode::CREATED, "the person: {body}");
    common::etag_uid(&headers)
        .split("::")
        .next()
        .expect("a versioned-object id")
        .to_owned()
}

/// The EHR whose subject is the `PERSON` `patient`, created by the
/// administrator; its id.
async fn record_of(app: &Router, patient: &str) -> String {
    let namespace = ferroehr::config::binding::PartyBindingConfig::default().namespace;
    let status = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0" },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": { "_type": "PARTY_SELF", "external_ref": { "_type": "PARTY_REF",
            "namespace": namespace, "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": patient } } },
        "is_queryable": true,
        "is_modifiable": true
    });
    let (code, body) = send(
        app,
        "POST",
        &format!("{BASE}/ehr"),
        "root",
        None,
        Some(("application/json", status.to_string())),
    )
    .await;
    assert_eq!(code, StatusCode::CREATED, "the EHR: {body}");
    serde_json::from_str::<Value>(&body).expect("an EHR")["ehr_id"]["value"]
        .as_str()
        .expect("ehr_id")
        .to_owned()
}

/// A care relationship of its own from `patient` to `carer` from `from` for
/// a month.
fn care_of(patient: &str, carer: &str, from: &str) -> Value {
    let reference = |id: &str| {
        json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                "id": { "_type": "HIER_OBJECT_ID", "value": id } })
    };
    json!({ "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "org.example::openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.care.v1",
        "name": access("under_care_of"),
        "details": { "_type": "ITEM_TREE", "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "details" },
            "items": [
                { "_type": "ELEMENT", "archetype_node_id": "at0002", "name": access("care_role"),
                  "value": coded("HL7_ParticipationType", "ATND") },
                { "_type": "ELEMENT", "archetype_node_id": "at0003", "name": access("provider"),
                  "value": { "_type": "DV_IDENTIFIER", "issuer": "urn:example:providers",
                             "id": "hospital", "type": "ORGANISATION" } }] },
        "time_validity": { "_type": "DV_INTERVAL",
            "lower": { "_type": "DV_DATE", "value": from },
            "upper": { "_type": "DV_DATE", "value": "2027-12-31" },
            "lower_unbounded": false, "upper_unbounded": false },
        "source": reference(patient),
        "target": reference(carer) })
}

/// Writes `body` as the administrator; the status and the answer.
async fn relate(app: &Router, body: &Value) -> (StatusCode, String) {
    send(
        app,
        "POST",
        &format!("{EXT}/demographic/party_relationship"),
        "root",
        None,
        Some(("application/json", body.to_string())),
    )
    .await
}

/// The access-log records of the system chain and the access log that are
/// flagged for review, waiting for `expected` of them.
async fn flagged(pool: &PgPool, expected: i64) -> i64 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM audit.access_trail WHERE review = 'R'")
                .fetch_one(pool)
                .await
                .expect("the trail reads");
        if count >= expected || Instant::now() > deadline {
            return count;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

#[tokio::test]
async fn a_write_to_a_marked_record_is_flagged_for_review() {
    let mapping = crate::party_binding_http::mapping().with_admin_role("ADMIN");
    let (_db, pool, svc, app) = Box::pin(app_with(mapping)).await;
    crate::access_policy_care_http::provider(&app).await;
    let (patient, carer) = (person(&app).await, person(&app).await);
    let ehr = record_of(&app, &patient).await;
    let writers = json!({ "relationship_writers": [{
        "_type": "ACCESS_RELATIONSHIP_WRITER",
        "applies_to": [{ "_type": "ACCESS_ACTOR_SCOPE",
                         "caller_kind": access("professional") }],
        "confidentialities": [coded("HL7_Confidentiality", "R")] }] });
    put_policy(&app, &writers).await;

    let (status, body) = relate(&app, &care_of(&patient, &carer, "2026-11-01")).await;
    assert_eq!(status, StatusCode::CREATED, "at N: {body}");
    assert_eq!(flagged(&pool, 1).await, 0, "a write at N is no review");

    common::commit_access_settings(&svc, ehr.parse().expect("an EHR id"), marked("R")).await;
    let (status, body) = relate(&app, &care_of(&patient, &carer, "2026-11-02")).await;
    assert_eq!(status, StatusCode::CREATED, "a writer named for R: {body}");
    assert_eq!(
        flagged(&pool, 1).await,
        1,
        "the write at R is flagged for review"
    );

    common::commit_access_settings(&svc, ehr.parse().expect("an EHR id"), marked("V")).await;
    let (status, body) = relate(&app, &care_of(&patient, &carer, "2026-11-03")).await;
    assert_eq!(
        status,
        StatusCode::FORBIDDEN,
        "no writer named for V: {body}"
    );
    assert!(
        body.contains("marked V"),
        "the refusal names the level: {body}"
    );
    assert_eq!(flagged(&pool, 2).await, 1, "a refused write is no review");
}
