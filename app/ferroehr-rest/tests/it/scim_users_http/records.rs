// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binding fields of the access records of a bound person and of a SCIM
//! client (#5002, #5007), through the assembled router against PostgreSQL.
//!
//! EHDS Annex II 3.2 asks the record to identify "(a) ... the healthcare
//! provider" and "(b) ... the specific natural person or persons having
//! accessed" (`docs/law/eu/ehds/text.html`). A person's read, AQL query and
//! write each carry the outcome `bound`, the `PERSON`, the groups its token
//! names and the declared provider, and a SCIM client's requests the outcome
//! `system`. Which fields a record carries is our own design.

use std::sync::Arc;

use ferroehr::config::binding::{ProviderId, Providers};
use http::StatusCode;
use serde_json::{Value, json};

use super::{AUDIENCE, CLIENT, PERSONS, app_of, call, config, created};
use crate::common::{self, BASE};

/// The issuer of the declared provider's identifier (the Dutch URA).
const PROVIDER_ISSUER: &str = "urn:oid:2.16.528.1.1007.3.3";
/// The declared provider's identifier.
const PROVIDER_ID: &str = "90000014";
/// The binding key of the person.
const KEY: &str = "5d1e2f3a-7f76-453a-919d-413861904646";

/// One stored record: its operation, binding outcome, party, groups and
/// provider identifier.
type Stored = (
    String,
    Option<String>,
    Option<String>,
    Option<String>,
    Option<String>,
);

/// The records naming the `PERSON` `person`, once the trail holds each of
/// `operations`.
async fn records_of(pool: &sqlx::PgPool, person: &str, operations: &[&str]) -> Vec<Stored> {
    for _ in 0..100 {
        let rows: Vec<Stored> = sqlx::query_as(
            "SELECT operation, binding, party ->> 'id', party_groups::text, provider ->> 'id' \
             FROM audit.audit_event WHERE party ->> 'id' = $1 ORDER BY recorded_at",
        )
        .bind(person)
        .fetch_all(pool)
        .await
        .expect("read the trail");
        if operations
            .iter()
            .all(|operation| rows.iter().any(|row| row.0 == *operation))
        {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let all: Vec<(String, Option<String>, Option<String>)> =
        sqlx::query_as("SELECT operation, binding, party::text FROM audit.audit_event")
            .fetch_all(pool)
            .await
            .expect("read the trail");
    panic!("the trail lacks one of {operations:?} for {person}: {all:?}");
}

#[tokio::test]
async fn a_bound_persons_read_query_and_write_record_its_party_groups_and_provider() {
    let mut config = config();
    config.party_binding.provider = Some(Providers::One(ProviderId {
        issuer: PROVIDER_ISSUER.to_owned(),
        id: PROVIDER_ID.to_owned(),
    }));
    let (_db, pool, service, _) = Box::pin(app_of(config.clone(), true)).await;
    // NOTE: `ferroehr_rest::build_full` hands the authenticator the local party source, as here.
    let source = Arc::new(service.party_source("demographic"));
    let app = crate::party_binding_http::router_with(Arc::clone(&service), config, source).await;
    let (person, _) = created(&app, CLIENT, "jdoe", KEY).await;
    let (status, _, body) = call(
        &app,
        "POST",
        "/Groups",
        CLIENT,
        Some(&json!({
            "schemas": ["urn:ietf:params:scim:schemas:core:2.0:Group"],
            "displayName": "Ward",
            "externalId": "ward",
            "members": [{ "value": person }],
        })),
        &[],
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "{body}");
    let ward = body["id"].as_str().expect("a Group id").to_owned();

    let exp = jiff::Timestamp::now().as_second() + 3600;
    let login = common::bearer(&json!({ "iss": PERSONS, "aud": AUDIENCE, "exp": exp,
                                        "sub": KEY, "groups": ["ward"] }));
    let (status, headers, body) = common::send(
        &app,
        common::req("POST", format!("{BASE}/ehr"))
            .auth(&login)
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the write: {body}");
    let ehr = common::etag_uid(&headers);
    let (status, body) = common::send_body(
        &app,
        common::req("GET", format!("{BASE}/ehr/{ehr}"))
            .auth(&login)
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the read: {body}");
    let aql = urlencoding::encode("SELECT e/ehr_id/value FROM EHR e");
    let (status, body) = common::send_body(
        &app,
        common::req("GET", format!("{BASE}/query/aql?q={aql}"))
            .auth(&login)
            .accept("application/json")
            .build(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the query: {body}");

    let operations = ["ehr_create", "ehr_get_by_id", "query_execute_adhoc_query"];
    let rows = records_of(&pool, &person, &operations).await;
    for operation in operations {
        let row = rows
            .iter()
            .find(|row| row.0 == operation)
            .expect("the operation's record");
        let groups: Value =
            serde_json::from_str(row.3.as_deref().unwrap_or("null")).expect("the groups are JSON");
        assert_eq!(
            (row.1.as_deref(), row.2.as_deref(), groups, row.4.as_deref()),
            (
                Some("bound"),
                Some(person.as_str()),
                json!([ward]),
                Some(PROVIDER_ID)
            ),
            "{operation}: {rows:?}"
        );
    }

    // The SCIM client's own requests are a system caller's records.
    let outcomes: Vec<Option<String>> =
        sqlx::query_scalar("SELECT binding FROM audit.audit_event WHERE principal = $1")
            .bind(CLIENT)
            .fetch_all(&pool)
            .await
            .expect("read the trail");
    assert!(!outcomes.is_empty(), "the SCIM requests are recorded");
    assert!(
        outcomes
            .iter()
            .all(|outcome| outcome.as_deref() == Some("system")),
        "{outcomes:?}"
    );
}
