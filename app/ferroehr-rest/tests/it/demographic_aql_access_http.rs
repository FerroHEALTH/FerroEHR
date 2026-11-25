// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A demographic AQL query through the access evaluator answers the parties
//! the Demographic API would serve the same caller (#6087): an administrator
//! and a caller with access groups each run a `PERSON` query over the
//! assembled router, and every party the administrator's query returns is in
//! the caller's rows exactly when the caller's `GET` of that party answers
//! `200`, and out of them exactly when it answers `403`.
//!
//! The access-control decision procedure decides "every access to an EHR"
//! (the FerroEHR access control draft, its decision procedure §Overview), and
//! a party belongs to no EHR, so a party is read under the Demographic API's
//! rules (`party_binding::protect::check_read`). No openEHR spec governs AQL
//! over a demographic repository: our own design.

use std::collections::BTreeSet;

use axum::Router;
use http::StatusCode;
use serde_json::{Value, json};

use crate::access_evaluator_http::{app_with, send};
use crate::common::BASE;
use crate::demographic_http::person_body;

/// The `PERSON` query of Cadasto/FerroHEALTH-backlog#51 (#6084), with its
/// relationship target as a parameter.
const REPORTED: &str = "SELECT DISTINCT p/uid/value FROM PERSON p \
     WHERE p/relationships/target/id/value = $target \
     AND p/details/items[at0001]/items[at0010]/value/value matches {'19500101', '1950-01-01'} \
     LIMIT 1001";

/// The versioned-object id of each `uid/value` the query `aql` answers
/// `user`, which must answer `200`.
async fn parties(app: &Router, user: &str, aql: &str, parameters: &Value) -> BTreeSet<String> {
    let body = json!({ "q": aql, "query_parameters": parameters }).to_string();
    let (status, result) = send(
        app,
        "POST",
        &format!("{BASE}/query/aql"),
        user,
        None,
        Some(("application/json", body)),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{user}: {aql}: {result}");
    let result: Value = serde_json::from_str(&result).expect("a RESULT_SET");
    result["rows"]
        .as_array()
        .expect("a rows array")
        .iter()
        .map(|row| {
            row[0]
                .as_str()
                .and_then(|uid| uid.split("::").next())
                .expect("a version uid")
                .to_owned()
        })
        .collect()
}

/// A `PERSON` created by the administrator from `body`, by its
/// versioned-object id.
async fn person(app: &Router, body: &Value) -> String {
    let (status, body) = send(
        app,
        "POST",
        &format!("{BASE}/demographic/person"),
        "root",
        None,
        Some(("application/json", body.to_string())),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the PERSON: {body}");
    let created: Value = serde_json::from_str(&body).expect("the PERSON");
    created["uid"]["value"]
        .as_str()
        .and_then(|uid| uid.split("::").next())
        .expect("a uid")
        .to_owned()
}

/// A `PERSON` that holds a client identifier, which makes it a policy party
/// only an administrator reads (`party_binding::policy_mark`).
fn policy_person() -> Value {
    let mut body = person_body();
    body["details"] = json!({
        "_type": "ITEM_TREE", "archetype_node_id": "at0010",
        "name": { "_type": "DV_TEXT", "value": "details" },
        "items": [{
            "_type": "ELEMENT", "archetype_node_id": "at0011",
            "name": { "_type": "DV_CODED_TEXT", "value": "client_id", "defining_code": { "_type": "CODE_PHRASE", "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr_access" }, "code_string": "client_id" } },
            "value": { "_type": "DV_IDENTIFIER", "issuer": "https://machines.example",
                       "id": "lab", "type": "client_id" }
        }]
    });
    body
}

#[tokio::test]
async fn a_person_query_answers_what_the_demographic_api_serves() {
    // A deployment's mapping recognises the policy parties
    // (`config::binding::person`), and `ADMIN` is the harness's administrator role.
    let mapping = crate::party_binding_http::mapping()
        .with_policy()
        .with_admin_role("ADMIN");
    let (_db, _pool, _svc, app) = Box::pin(app_with(mapping)).await;
    let created = [
        person(&app, &person_body()).await,
        person(&app, &person_body()).await,
    ];
    let policy = person(&app, &policy_person()).await;

    let every = parties(&app, "root", "SELECT p/uid/value FROM PERSON p", &json!({})).await;
    assert!(
        created
            .iter()
            .chain([&policy])
            .all(|party| every.contains(party)),
        "the administrator reads every PERSON: {every:?}"
    );
    let nurse = parties(
        &app,
        "nurse",
        "SELECT p/uid/value FROM PERSON p",
        &json!({}),
    )
    .await;
    let mut refused = 0;
    for party in &every {
        let (status, body) = send(
            &app,
            "GET",
            &format!("{BASE}/demographic/person/{party}"),
            "nurse",
            None,
            None,
        )
        .await;
        match status {
            StatusCode::OK => assert!(nurse.contains(party), "{party} is read, not queried"),
            StatusCode::FORBIDDEN => {
                refused += 1;
                assert!(!nurse.contains(party), "{party} is refused, yet queried");
            }
            other => panic!("the nurse's read of {party}: {other} {body}"),
        }
    }
    assert!(
        created.iter().all(|party| nurse.contains(party)),
        "an ordinary PERSON is anyone's to query: {nurse:?}"
    );
    assert!(
        refused > 0 && !nurse.contains(&policy),
        "the policy PERSON is the administrator's alone: {nurse:?}"
    );

    let target = json!({ "target": "01a1882d-70c6-77b5-930b-0f55a1b3af16" });
    for user in ["root", "nurse"] {
        assert!(
            parties(&app, user, REPORTED, &target).await.is_empty(),
            "{user}: no PERSON relates to the target"
        );
    }
}

/// With the access settings store unavailable, a demographic query still
/// answers from the party domain, which no `EHR_ACCESS` condition governs,
/// and a clinical query, whose condition cannot be judged, answers `503`
/// (#6110).
#[tokio::test]
async fn a_person_query_answers_while_the_settings_store_is_down() {
    let mapping = crate::party_binding_http::mapping()
        .with_policy()
        .with_admin_role("ADMIN");
    let (_db, pool, _svc, app) = Box::pin(app_with(mapping)).await;
    let created = person(&app, &person_body()).await;
    sqlx::query("ALTER TABLE clinical.access_class RENAME TO access_class_away")
        .execute(&pool)
        .await
        .expect("the settings store goes away");

    let nurse = parties(
        &app,
        "nurse",
        "SELECT p/uid/value FROM PERSON p",
        &json!({}),
    )
    .await;
    assert!(nurse.contains(&created), "the PERSON is served: {nurse:?}");

    let body = json!({ "q": "SELECT e/ehr_id/value FROM EHR e LIMIT 20" }).to_string();
    let (status, answer) = send(
        &app,
        "POST",
        &format!("{BASE}/query/aql"),
        "nurse",
        None,
        Some(("application/json", body)),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::SERVICE_UNAVAILABLE,
        "a clinical query is judged by the settings: {answer}"
    );
}
