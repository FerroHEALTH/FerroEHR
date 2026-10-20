// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! AQL rooted at a demographic class against a real PostgreSQL 18 (shared
//! testkit harness): parties are created through the demographic service, then
//! queried through the `QueryService` seam.
//!
//! QUERY master03 §FROM gives semantics to the clinical repository only, so a
//! `FROM PERSON`, `FROM ORGANISATION` or `FROM ROLE` root is the experimental
//! demographic extension; no openEHR spec governs it, and the behaviour pinned
//! here is our own design: the latest trunk version of each party, logically
//! deleted parties left out, paths inside the party (its `details` tree and
//! its `identities` containers), `CONTAINS CLUSTER` inside a party, `uid/value`
//! projection and ordering, REST paging, and typed refusals for the
//! constructs the subset does not serve.

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::typed_body::typed;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::status::{CallStatusType, SmError};

const IDENTIFIER_CLUSTER: &str = "openEHR-DEMOGRAPHIC-CLUSTER.person_identifier.v2";

/// A `DV_IDENTIFIER` value (RM `data_types` §Identifiers).
fn identifier(id: &str, issuer: &str) -> Value {
    json!({ "_type": "DV_IDENTIFIER", "issuer": issuer, "id": id })
}

/// The `details` tree every fixture party carries: one identifier element at
/// `details[at0001]/items[at0002]` and one archetyped identifier `CLUSTER`.
fn details(id: &str, system: &str) -> Value {
    json!({
        "_type": "ITEM_TREE",
        "archetype_node_id": "at0001",
        "name": { "_type": "DV_TEXT", "value": "details" },
        "items": [
            {
                "_type": "ELEMENT",
                "archetype_node_id": "at0002",
                "name": { "_type": "DV_TEXT", "value": "identifier" },
                "value": identifier(id, system)
            },
            {
                "_type": "CLUSTER",
                "archetype_node_id": IDENTIFIER_CLUSTER,
                "archetype_details": { "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": IDENTIFIER_CLUSTER },
                    "rm_version": "1.1.0" },
                "name": { "_type": "DV_TEXT", "value": "person identifier" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0001",
                    "name": { "_type": "DV_TEXT", "value": "identifier" },
                    "value": identifier(id, system)
                }]
            }
        ]
    })
}

/// One `PARTY_IDENTITY` carrying `name` at `identities[at0003]/details[at0004]
/// /items[at0005]/value/value`.
fn identity(name: &str) -> Value {
    json!([{
        "_type": "PARTY_IDENTITY",
        "archetype_node_id": "at0003",
        "name": { "_type": "DV_TEXT", "value": "legal name" },
        "details": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0004",
            "name": { "_type": "DV_TEXT", "value": "structure" },
            "items": [{
                "_type": "ELEMENT",
                "archetype_node_id": "at0005",
                "name": { "_type": "DV_TEXT", "value": "name" },
                "value": { "_type": "DV_TEXT", "value": name }
            }]
        }
    }])
}

fn archetyped(rm: &str, concept: &str) -> Value {
    let id = format!("openEHR-DEMOGRAPHIC-{rm}.{concept}.v1");
    json!({ "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": id },
        "rm_version": "1.1.0" })
}

fn person(name: &str, id: &str, system: &str) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": archetyped("PERSON", "person"),
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": identity(name),
        "details": details(id, system)
    })
}

fn organisation(name: &str, id: &str, system: &str) -> Value {
    json!({
        "_type": "ORGANISATION",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ORGANISATION.organisation.v1",
        "archetype_details": archetyped("ORGANISATION", "organisation"),
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": identity(name),
        "details": details(id, system)
    })
}

fn role(name: &str, id: &str, system: &str) -> Value {
    json!({
        "_type": "ROLE",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-ROLE.role.v1",
        "archetype_details": archetyped("ROLE", "role"),
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": identity(name),
        "details": details(id, system),
        "performer": {
            "_type": "PARTY_REF",
            "namespace": "demographic",
            "type": "PERSON",
            "id": { "_type": "HIER_OBJECT_ID", "value": "cccccccc-cccc-4ccc-8ccc-cccccccccccc" }
        }
    })
}

/// Creates a party and returns its `OBJECT_VERSION_ID`.
async fn create(svc: &FerroEhrService, kind: PartyKind, body: &Value) -> String {
    let created = svc
        .party_create(kind, typed(body), None)
        .await
        .unwrap_or_else(|e| panic!("create {kind:?}: {e:?}"));
    created.body["uid"]["value"]
        .as_str()
        .expect("uid.value")
        .to_owned()
}

fn params(pairs: &[(&str, &str)]) -> AqlQueryRequest {
    AqlQueryRequest {
        parameters: pairs
            .iter()
            .map(|(k, v)| ((*k).to_owned(), json!(v)))
            .collect::<BTreeMap<_, _>>(),
        ..AqlQueryRequest::default()
    }
}

async fn run(svc: &FerroEhrService, aql: &str, request: AqlQueryRequest) -> Vec<Value> {
    svc.execute_ad_hoc_query(aql.to_owned(), request)
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"))
        .result_set["rows"]
        .as_array()
        .expect("rows array")
        .clone()
}

async fn refuse(svc: &FerroEhrService, aql: &str, request: AqlQueryRequest) -> SmError {
    svc.execute_ad_hoc_query(aql.to_owned(), request)
        .await
        .expect_err("the query must be refused")
}

fn new_service(db: &testkit::TestDb) -> FerroEhrService {
    FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
}

/// Each served root answers rows over the latest version of each party, with
/// a `WHERE` on a parameterised path through the `details` tree.
#[tokio::test]
async fn each_root_finds_its_party_by_identifier() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);

    let jane = create(&svc, PartyKind::Person, &person("Jane", "P-1", "urn:bsn")).await;
    create(&svc, PartyKind::Person, &person("John", "P-2", "urn:bsn")).await;
    let acme = create(
        &svc,
        PartyKind::Organisation,
        &organisation("Acme", "O-1", "urn:agb"),
    )
    .await;
    let nurse = create(&svc, PartyKind::Role, &role("Nurse", "R-1", "urn:big")).await;

    for (class, id, system, expected) in [
        ("PERSON", "P-1", "urn:bsn", &jane),
        ("ORGANISATION", "O-1", "urn:agb", &acme),
        ("ROLE", "R-1", "urn:big", &nurse),
    ] {
        let aql = format!(
            "SELECT x/uid/value FROM {class} x \
             WHERE x/details[at0001]/items[at0002]/value/id = $identifier \
             AND x/details[at0001]/items[at0002]/value/issuer = $system"
        );
        let rows = run(
            &svc,
            &aql,
            params(&[("identifier", id), ("system", system)]),
        )
        .await;
        assert_eq!(rows, vec![json!([expected])], "{class} by identifier");
    }

    // A person query never answers an organisation, and the reverse.
    let people = run(
        &svc,
        "SELECT p/uid/value FROM PERSON p",
        AqlQueryRequest::default(),
    )
    .await;
    assert_eq!(people.len(), 2, "two persons, got {people:?}");
}

/// `CONTAINS CLUSTER` inside a party, and a path through the `identities`
/// containers the node codec stores as their own rows.
#[tokio::test]
async fn contained_cluster_and_identity_paths() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);

    let jane = create(&svc, PartyKind::Person, &person("Jane", "P-1", "urn:bsn")).await;
    create(&svc, PartyKind::Person, &person("John", "P-2", "urn:bsn")).await;

    let aql = format!(
        "SELECT p/uid/value, c/items[at0001]/value/id \
         FROM PERSON p CONTAINS CLUSTER c[{IDENTIFIER_CLUSTER}] \
         WHERE c/items[at0001]/value/id = $identifier"
    );
    let rows = run(&svc, &aql, params(&[("identifier", "P-1")])).await;
    assert_eq!(rows, vec![json!([jane, "P-1"])]);

    let rows = run(
        &svc,
        "SELECT p/identities[at0003]/details[at0004]/items[at0005]/value/value \
         FROM PERSON p WHERE p/uid/value = $uid",
        params(&[("uid", &jane)]),
    )
    .await;
    assert_eq!(rows, vec![json!(["Jane"])]);

    // The whole party reassembles from its rows.
    let rows = run(
        &svc,
        "SELECT p FROM PERSON p WHERE p/uid/value = $uid",
        params(&[("uid", &jane)]),
    )
    .await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0][0]["_type"], "PERSON");
    assert_eq!(rows[0][0]["name"]["value"], "Jane");
    assert_eq!(
        rows[0][0]["details"]["items"][0]["value"]["id"], "P-1",
        "the details tree reassembles"
    );
}

/// `ORDER BY p/uid/value` with the REST `offset` and `fetch` pages through
/// the parties in a stable order.
#[tokio::test]
async fn ordered_paging_over_parties() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);

    let mut uids = Vec::new();
    for i in 0..5 {
        let id = format!("P-{i}");
        uids.push(create(&svc, PartyKind::Person, &person("Someone", &id, "urn:bsn")).await);
    }
    uids.sort();

    let aql = "SELECT p/uid/value FROM PERSON p ORDER BY p/uid/value";
    let mut paged = Vec::new();
    for offset in [0, 2, 4] {
        let request = AqlQueryRequest {
            offset: Some(offset),
            fetch: Some(2),
            ..AqlQueryRequest::default()
        };
        for row in run(&svc, aql, request).await {
            paged.push(row[0].as_str().expect("uid").to_owned());
        }
    }
    assert_eq!(paged, uids, "three pages cover the five parties in order");

    let first = run(
        &svc,
        "SELECT p/uid/value FROM PERSON p LIMIT 2",
        AqlQueryRequest::default(),
    )
    .await;
    assert_eq!(first.len(), 2, "the streaming shape honours LIMIT");
}

/// Only the latest version answers, and a logically deleted party none.
#[tokio::test]
async fn latest_version_and_deleted_parties() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);

    let v1 = create(&svc, PartyKind::Person, &person("Jane", "P-1", "urn:bsn")).await;
    let vo = v1.split("::").next().expect("vo id").to_owned();
    let updated = svc
        .party_update(
            PartyKind::Person,
            vo.clone(),
            v1.clone(),
            typed(&person("Jane Roe", "P-9", "urn:bsn")),
            None,
        )
        .await
        .expect("update");
    let v2 = updated.body["uid"]["value"]
        .as_str()
        .expect("uid")
        .to_owned();

    let by_id = "SELECT p/uid/value FROM PERSON p \
                 WHERE p/details[at0001]/items[at0002]/value/id = $identifier";
    assert!(
        run(&svc, by_id, params(&[("identifier", "P-1")]))
            .await
            .is_empty(),
        "the superseded identifier answers nothing"
    );
    assert_eq!(
        run(&svc, by_id, params(&[("identifier", "P-9")])).await,
        vec![json!([v2])]
    );

    let doomed = create(&svc, PartyKind::Person, &person("Gone", "P-5", "urn:bsn")).await;
    svc.party_delete(PartyKind::Person, doomed.clone(), Some(doomed), None)
        .await
        .expect("delete");
    for aql in [
        by_id,
        // The non-streaming shape (ordered) and the streaming one (limited).
        "SELECT p/uid/value FROM PERSON p \
         WHERE p/details[at0001]/items[at0002]/value/id = $identifier ORDER BY p/uid/value",
    ] {
        assert!(
            run(&svc, aql, params(&[("identifier", "P-5")]))
                .await
                .is_empty(),
            "a deleted party answers nothing: {aql}"
        );
    }
    let all = run(
        &svc,
        "SELECT p/uid/value FROM PERSON p LIMIT 10",
        AqlQueryRequest::default(),
    )
    .await;
    assert_eq!(
        all,
        vec![json!([v2])],
        "only the live party, at its latest version"
    );
}

/// The constructs the subset does not serve answer `400` naming them.
#[tokio::test]
async fn unserved_demographic_constructs_are_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);
    let jane = create(&svc, PartyKind::Person, &person("Jane", "P-1", "urn:bsn")).await;

    for (aql, names) in [
        ("SELECT g/uid/value FROM GROUP g", "GROUP"),
        ("SELECT a/uid/value FROM AGENT a", "AGENT"),
        ("SELECT p/uid/value FROM PARTY p", "PARTY"),
        (
            "SELECT r/uid/value FROM PARTY_RELATIONSHIP r",
            "PARTY_RELATIONSHIP",
        ),
        ("SELECT p/uid/value FROM EHR e CONTAINS PERSON p", "EHR"),
        (
            "SELECT p/uid/value FROM PERSON p CONTAINS COMPOSITION c",
            "COMPOSITION",
        ),
        (
            "SELECT p/uid/value FROM PERSON p CONTAINS ORGANISATION o",
            "ORGANISATION",
        ),
    ] {
        let err = refuse(&svc, aql, AqlQueryRequest::default()).await;
        assert_eq!(err.status, CallStatusType::PreconditionViolation, "{aql}");
        assert!(
            err.message.contains(names),
            "the refusal of {aql} names {names}: {}",
            err.message
        );
    }

    // An EHR scope cannot apply to a party query.
    let ehr_id = svc.create_ehr(None).await.expect("create_ehr").to_string();
    let err = refuse(
        &svc,
        "SELECT p/uid/value FROM PERSON p",
        AqlQueryRequest {
            ehr_ids: vec![ehr_id],
            ..AqlQueryRequest::default()
        },
    )
    .await;
    assert_eq!(err.status, CallStatusType::PreconditionViolation);
    assert!(err.message.contains("ehr_id"), "{}", err.message);

    // A clinical query does not read the party domain.
    let rows = run(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS CLUSTER c",
        AqlQueryRequest::default(),
    )
    .await;
    assert!(rows.is_empty(), "no clinical row for {jane}, got {rows:?}");
}
