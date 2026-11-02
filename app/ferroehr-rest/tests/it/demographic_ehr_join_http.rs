// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A PERSON and the EHR whose subject names it, both created over the wire,
//! join in `FROM PERSON p CONTAINS EHR e` with no other call (#4411).
//!
//! `EHR_STATUS.subject` is a `PARTY_SELF` whose `external_ref` is a
//! `PARTY_REF` to the demographic party (RM ehr master04 §EHR Status; BASE
//! `base_types` `OBJECT_REF`, `PARTY_REF`), and ITS-REST 1.1.0
//! `ehr_get_by_subject` matches `external_ref.id.value` and
//! `external_ref.namespace`. The demographic root and its join are no openEHR
//! spec's (QUERY master03 §FROM gives semantics to the clinical repository
//! only): our own design. The six requests are the ones the report made.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's async bodies; panicking assertions and direct JSON indexing \
              are the intended shape here"
)]

use std::sync::Arc;

use axum::Router;
use http::StatusCode;
use serde_json::{Value, json};

use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr_index::provider::EhrIndexProvider;
use ferroehr_rest::config::AppConfig;

use crate::common::{self, BASE};
use crate::demographic_http::person_body;

/// The router over a service on the local EHR index posture with no pseudonym
/// namespace declared, the posture the report ran.
async fn local_app() -> (testkit::TestDb, Router) {
    let (db, pool) = common::migrated_pool().await;
    ferroehr::db::stamp_subject_posture(&pool, false)
        .await
        .expect("stamp the subject posture");
    let policy = ferroehr::privacy::PrivacyPolicy::compile(
        &ferroehr::privacy::config::PrivacyConfig::default(),
    )
    .expect("the policy compiles");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_privacy(Arc::new(policy))
        .with_ehr_index(EhrIndexProvider::Local);
    let mut config = AppConfig {
        server: common::server(),
        ..AppConfig::default()
    };
    config.auth.enabled = false;
    (db, common::router_with(config, Arc::new(service)).await)
}

/// `POST /query/aql` with `q` and `query_parameters`, answering the rows.
async fn query(app: &Router, aql: &str, parameters: &Value) -> Vec<Value> {
    let req = common::req("POST", format!("{BASE}/query/aql"))
        .json(json!({ "q": aql, "query_parameters": parameters }).to_string())
        .build();
    let (status, result) = common::send_json(app, req).await;
    assert_eq!(status, StatusCode::OK, "query {aql}: {result}");
    result["rows"].as_array().expect("rows array").clone()
}

#[tokio::test]
async fn a_person_joins_to_the_ehr_its_subject_names_over_the_wire() {
    let (_db, app) = local_app().await;

    // 1. The PERSON.
    let (status, headers, body) = common::send(
        &app,
        common::post_json(
            &format!("{BASE}/demographic/person"),
            &person_body().to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the PERSON is created: {body}");
    let uid = common::etag_uid(&headers);
    let party = uid
        .split("::")
        .next()
        .expect("a versioned-object id")
        .to_owned();

    // 2. The EHR whose subject names the PERSON.
    let external_ref = json!({
        "_type": "PARTY_REF",
        "id": { "_type": "HIER_OBJECT_ID", "value": party },
        "namespace": "local",
        "type": "PERSON"
    });
    let status_body = json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": { "_type": "PARTY_SELF", "external_ref": external_ref },
        "is_queryable": true,
        "is_modifiable": true
    });
    let (status, headers, body) = common::send(
        &app,
        common::post_json(&format!("{BASE}/ehr"), &status_body.to_string()),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the EHR is created: {body}");
    let ehr_id = common::etag_uid(&headers);

    // 3. The EHR by its subject.
    let (status, body) = common::send_body(
        &app,
        common::get(&format!(
            "{BASE}/ehr?subject_id={party}&subject_namespace=local"
        )),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the subject's EHR is found: {body}");
    let found: Value = serde_json::from_str(&body).expect("the EHR body");
    assert_eq!(found["ehr_id"]["value"], json!(ehr_id), "{body}");

    // 4. The stored subject reference.
    let (status, body) = common::send_body(
        &app,
        common::get(&format!("{BASE}/ehr/{ehr_id}/ehr_status")),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the EHR_STATUS is read: {body}");
    let stored: Value = serde_json::from_str(&body).expect("the EHR_STATUS body");
    assert_eq!(
        stored["subject"]["external_ref"], external_ref,
        "the subject is stored as sent"
    );

    // 5. The EHR by its subject reference in AQL.
    let rows = query(
        &app,
        "SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_status/subject/external_ref/id/value = $u",
        &json!({ "u": party }),
    )
    .await;
    assert_eq!(
        rows,
        vec![json!([ehr_id])],
        "the EHR root finds its subject"
    );

    // 6. The PERSON joined to its EHR, unfiltered and by the party's uid.
    let rows = query(
        &app,
        "SELECT p/uid/value, e/ehr_id/value FROM PERSON p CONTAINS EHR e",
        &json!({}),
    )
    .await;
    assert_eq!(
        rows,
        vec![json!([uid, ehr_id])],
        "the PERSON joins to its EHR"
    );
    let rows = query(
        &app,
        "SELECT e/ehr_id/value FROM PERSON p CONTAINS EHR e WHERE p/uid/value LIKE $u",
        &json!({ "u": format!("{party}::*") }),
    )
    .await;
    assert_eq!(rows, vec![json!([ehr_id])], "a LIKE on the uid joins");
    let rows = query(
        &app,
        "SELECT e/ehr_id/value FROM PERSON p CONTAINS EHR e WHERE p/uid/value = $u",
        &json!({ "u": uid }),
    )
    .await;
    assert_eq!(rows, vec![json!([ehr_id])], "the full versioned uid joins");
}
