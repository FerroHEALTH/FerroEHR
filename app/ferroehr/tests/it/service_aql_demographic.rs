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

use ferroehr::service::version_update::WriteRequest;
use std::collections::BTreeMap;

use serde_json::{Value, json};

use crate::typed_body::typed;
use ferroehr::error::ServiceError;
use ferroehr::error::refusal::Malformed;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::query::request::AqlQueryRequest;

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

fn group(name: &str, id: &str, system: &str) -> Value {
    json!({
        "_type": "GROUP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-GROUP.group.v1",
        "archetype_details": archetyped("GROUP", "group"),
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": identity(name),
        "details": details(id, system)
    })
}

fn agent(name: &str, id: &str, system: &str) -> Value {
    json!({
        "_type": "AGENT",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-AGENT.device.v1",
        "archetype_details": archetyped("AGENT", "device"),
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": identity(name),
        "details": details(id, system)
    })
}

/// A `PARTY_REF` naming the container of the version `uid` as `class`.
fn party_ref(class: &str, uid: &str) -> Value {
    json!({ "_type": "PARTY_REF", "namespace": "demographic", "type": class,
        "id": { "_type": "HIER_OBJECT_ID", "value": party_id(uid) } })
}

/// A `PARTY_RELATIONSHIP` from `source` to `target`, each a `(class, uid)`,
/// named `name` and carrying the `details` tree of `id`.
fn relationship(name: &str, id: &str, source: (&str, &str), target: (&str, &str)) -> Value {
    json!({
        "_type": "PARTY_RELATIONSHIP",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1",
        "archetype_details": archetyped("PARTY_RELATIONSHIP", "relationship"),
        "name": { "_type": "DV_TEXT", "value": name },
        "details": details(id, "urn:relation"),
        "source": party_ref(source.0, source.1),
        "target": party_ref(target.0, target.1)
    })
}

/// Creates a standalone `PARTY_RELATIONSHIP` and returns its
/// `OBJECT_VERSION_ID`.
async fn relate(svc: &FerroEhrService, body: &Value) -> String {
    let created = svc
        .party_relationship_create(WriteRequest::new(typed(body)))
        .await
        .unwrap_or_else(|e| panic!("create PARTY_RELATIONSHIP: {e:?}"));
    created.body["uid"]["value"]
        .as_str()
        .expect("uid.value")
        .to_owned()
}

/// Creates a party and returns its `OBJECT_VERSION_ID`.
async fn create(svc: &FerroEhrService, kind: PartyKind, body: &Value) -> String {
    let created = svc
        .party_create(kind, WriteRequest::new(typed(body)))
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
    svc.execute_aql(aql, None, &request)
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"))
        .result_set["rows"]
        .as_array()
        .expect("rows array")
        .clone()
}

async fn refuse(svc: &FerroEhrService, aql: &str, request: AqlQueryRequest) -> ServiceError {
    svc.execute_aql(aql, None, &request)
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
    let ward = create(&svc, PartyKind::Group, &group("Ward 3", "G-1", "urn:ward")).await;
    let analyser = create(
        &svc,
        PartyKind::Agent,
        &agent("Analyser", "A-1", "urn:device"),
    )
    .await;
    let employed = relate(
        &svc,
        &relationship(
            "employment",
            "PR-1",
            ("PERSON", &jane),
            ("ORGANISATION", &acme),
        ),
    )
    .await;

    for (class, id, system, expected) in [
        ("PERSON", "P-1", "urn:bsn", &jane),
        ("ORGANISATION", "O-1", "urn:agb", &acme),
        ("ROLE", "R-1", "urn:big", &nurse),
        ("GROUP", "G-1", "urn:ward", &ward),
        ("AGENT", "A-1", "urn:device", &analyser),
        ("PARTY_RELATIONSHIP", "PR-1", "urn:relation", &employed),
        // The abstract roots cover every concrete class below them.
        ("PARTY", "P-1", "urn:bsn", &jane),
        ("PARTY", "R-1", "urn:big", &nurse),
        ("PARTY", "A-1", "urn:device", &analyser),
        ("ACTOR", "G-1", "urn:ward", &ward),
        ("ACTOR", "O-1", "urn:agb", &acme),
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

    // Each root counts the classes it covers, and no other: a `ROLE` is no
    // `ACTOR`, and a relationship is no `PARTY`.
    for (class, count) in [
        ("PERSON", 2),
        ("ORGANISATION", 1),
        ("ROLE", 1),
        ("GROUP", 1),
        ("AGENT", 1),
        ("ACTOR", 5),
        ("PARTY", 6),
        ("PARTY_RELATIONSHIP", 1),
    ] {
        let rows = run(
            &svc,
            &format!("SELECT COUNT(x/uid/value) FROM {class} x"),
            AqlQueryRequest::default(),
        )
        .await;
        assert_eq!(rows, vec![json!([count])], "the {class} count");
    }
    let rows = run(
        &svc,
        "SELECT r/uid/value FROM ACTOR r WHERE r/details[at0001]/items[at0002]/value/id = 'R-1'",
        AqlQueryRequest::default(),
    )
    .await;
    assert!(rows.is_empty(), "a ROLE is no ACTOR: {rows:?}");
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

/// One `CONTACT` holding one `ADDRESS` whose `details` carry `street`
/// (RM demographic §CONTACT, §ADDRESS).
fn contacts(street: &str) -> Value {
    json!([{
        "_type": "CONTACT",
        "archetype_node_id": "at0010",
        "name": { "_type": "DV_TEXT", "value": "home" },
        "addresses": [{
            "_type": "ADDRESS",
            "archetype_node_id": "at0011",
            "name": { "_type": "DV_TEXT", "value": "postal" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0012",
                "name": { "_type": "DV_TEXT", "value": "address" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0013",
                    "name": { "_type": "DV_TEXT", "value": "street" },
                    "value": { "_type": "DV_TEXT", "value": street }
                }]
            }
        }]
    }])
}

/// One `CAPABILITY` whose `credentials` carry `licence` (RM demographic
/// §CAPABILITY).
fn capabilities(licence: &str) -> Value {
    json!([{
        "_type": "CAPABILITY",
        "archetype_node_id": "at0020",
        "name": { "_type": "DV_TEXT", "value": "registration" },
        "credentials": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0021",
            "name": { "_type": "DV_TEXT", "value": "credentials" },
            "items": [{
                "_type": "ELEMENT",
                "archetype_node_id": "at0022",
                "name": { "_type": "DV_TEXT", "value": "licence" },
                "value": identifier(licence, "urn:big")
            }]
        }
    }])
}

/// The paths into a party's `relationships`, `contacts` and `roles` and a
/// `ROLE`'s `capabilities` answer their values, in `SELECT` and in `WHERE`
/// (RM demographic §PARTY, §ACTOR, §ROLE; BASE `master11-paths.adoc`
/// §Predicate Expressions). `contacts` and `capabilities` hold containers
/// with rows of their own, so a path through them answers one row per
/// match; `relationships` and `roles` are held in the party's own data, so a
/// projected path through them answers the array of its matches in one cell.
#[tokio::test]
async fn paths_into_relationships_contacts_roles_and_capabilities() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);

    let acme = create(
        &svc,
        PartyKind::Organisation,
        &organisation("Acme", "O-1", "urn:agb"),
    )
    .await;
    let mut nurse = role("Nurse", "R-1", "urn:big");
    nurse["capabilities"] = capabilities("BIG-42");
    let nurse = create(&svc, PartyKind::Role, &nurse).await;

    let mut jane = person("Jane", "P-1", "urn:bsn");
    let draft = create(&svc, PartyKind::Person, &jane).await;
    jane["contacts"] = contacts("Main street 1");
    jane["roles"] = json!([party_ref("ROLE", &nurse)]);
    let mut employment = relationship(
        "employment",
        "PR-1",
        ("PERSON", &draft),
        ("ORGANISATION", &acme),
    );
    // NOTE: an inline relationship is no archetype root, so it carries an at-code (RM common LOCATABLE.Archetyped_valid).
    employment["archetype_node_id"] = json!("at0030");
    let held = employment.as_object_mut().expect("a relationship object");
    held.remove("archetype_details");
    held.remove("details");
    jane["relationships"] = json!([employment]);
    let jane = svc
        .party_update(
            PartyKind::Person,
            crate::fixtures::object(party_id(&draft).to_owned()),
            crate::fixtures::if_match(draft.clone()),
            WriteRequest::new(typed(&jane)),
        )
        .await
        .unwrap_or_else(|e| panic!("update: {e:?}"))
        .body["uid"]["value"]
        .as_str()
        .expect("uid")
        .to_owned();

    for (aql, expected) in [
        (
            "SELECT p/relationships[at0030]/target/id/value FROM PERSON p",
            json!([[party_id(&acme)]]),
        ),
        (
            "SELECT p/relationships[at0030 and name/value='employment']/target/type FROM PERSON p",
            json!([["ORGANISATION"]]),
        ),
        (
            "SELECT p/contacts[at0010]/addresses[at0011]/details[at0012]/items[at0013]/value/value \
             FROM PERSON p",
            json!(["Main street 1"]),
        ),
        (
            "SELECT p/roles/id/value FROM PERSON p",
            json!([[party_id(&nurse)]]),
        ),
        (
            "SELECT r/capabilities[at0020]/credentials[at0021]/items[at0022]/value/id FROM ROLE r",
            json!(["BIG-42"]),
        ),
        (
            "SELECT a/details[at0012]/items[at0013]/value/value \
             FROM PARTY p CONTAINS ADDRESS a",
            json!(["Main street 1"]),
        ),
    ] {
        let rows = run(&svc, aql, AqlQueryRequest::default()).await;
        assert_eq!(rows, vec![expected], "{aql}");
    }

    for (aql, expected) in [
        (
            "SELECT p/uid/value FROM PARTY p \
             WHERE p/relationships[at0030]/target/id/value = $target",
            &jane,
        ),
        (
            "SELECT p/uid/value FROM ACTOR p WHERE p/roles/id/value = $target",
            &jane,
        ),
    ] {
        let target = if aql.contains("roles") {
            party_id(&nurse)
        } else {
            party_id(&acme)
        };
        let rows = run(&svc, aql, params(&[("target", target)])).await;
        assert_eq!(rows, vec![json!([expected])], "{aql}");
    }
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
            crate::fixtures::object(vo.clone()),
            crate::fixtures::if_match(v1.clone()),
            WriteRequest::new(typed(&person("Jane Roe", "P-9", "urn:bsn"))),
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
    svc.party_delete(
        PartyKind::Person,
        crate::fixtures::object(&doomed),
        crate::fixtures::delete_precondition(&doomed, Some(&doomed)),
        WriteRequest::new(()),
    )
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

/// The constructs the extension does not serve answer `400` naming them: a
/// demographic class below the root (an inline `PARTY_RELATIONSHIP` is held
/// by value in its source party and has no row of its own), a relationship
/// joined to an `EHR`, and a demographic root under any other class.
#[tokio::test]
async fn unserved_demographic_constructs_are_refused() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);
    let jane = create(&svc, PartyKind::Person, &person("Jane", "P-1", "urn:bsn")).await;

    for (aql, names) in [
        ("SELECT p/uid/value FROM EHR e CONTAINS PERSON p", "EHR"),
        (
            "SELECT r/uid/value FROM PARTY_RELATIONSHIP r CONTAINS EHR e",
            "EHR",
        ),
        (
            "SELECT p/uid/value FROM PARTY p CONTAINS PARTY_RELATIONSHIP r",
            "PARTY_RELATIONSHIP",
        ),
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
        assert!(
            matches!(&err, ServiceError::BadRequest(r) if r.kind == Malformed::Precondition),
            "{aql}"
        );
        assert!(
            err.message().contains(names),
            "the refusal of {aql} names {names}: {}",
            err.message()
        );
    }

    // An EHR scope cannot apply to a party query.
    let ehr_id = crate::fixtures::create_ehr(&svc).await.to_string();
    let err = refuse(
        &svc,
        "SELECT p/uid/value FROM PERSON p",
        AqlQueryRequest {
            ehr_ids: vec![crate::fixtures::ehr(&ehr_id)],
            ..AqlQueryRequest::default()
        },
    )
    .await;
    assert!(matches!(&err, ServiceError::BadRequest(r) if r.kind == Malformed::Precondition));
    assert!(err.message().contains("ehr_id"), "{}", err.message());

    // A clinical query does not read the party domain.
    let rows = run(
        &svc,
        "SELECT c/uid/value FROM EHR e CONTAINS CLUSTER c",
        AqlQueryRequest::default(),
    )
    .await;
    assert!(rows.is_empty(), "no clinical row for {jane}, got {rows:?}");
}

/// A party, the EHR whose `EHR_STATUS` names it as its subject, and one
/// composition per name in it.
async fn linked(svc: &FerroEhrService, name: &str, id: &str, compositions: &[&str]) -> String {
    let uid = create(svc, PartyKind::Person, &person(name, id, "urn:bsn")).await;
    let party = ferroehr::ids::VoId(
        uid.split("::")
            .next()
            .and_then(|s| s.parse().ok())
            .expect("party id"),
    );
    let ehr = crate::fixtures::create_ehr_for_party(svc, party).await;
    for composition in compositions {
        svc.create_composition(
            ehr,
            crate::fixtures::uv(&crate::fixtures::composition(composition), "249", None),
        )
        .await
        .unwrap_or_else(|e| panic!("create_composition ({composition}): {e:?}"));
    }
    uid
}

/// `PERSON p CONTAINS EHR e CONTAINS COMPOSITION c` joins each party to the
/// compositions of the EHR it is the subject of, across the three pools, with
/// the WHERE conjuncts of each side, ORDER BY over keys of both sides, and
/// REST paging over the joined rows. A deleted party and an EHR whose subject
/// has no party answer nothing.
#[tokio::test]
async fn a_party_joins_to_the_content_of_its_ehr() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);

    let ada = linked(&svc, "Ada", "P-1", &["visit b", "visit a"]).await;
    let bram = linked(&svc, "Bram", "P-2", &["visit c"]).await;
    let gone = linked(&svc, "Gone", "P-3", &["visit d"]).await;
    svc.party_delete(
        PartyKind::Person,
        crate::fixtures::object(&gone),
        crate::fixtures::delete_precondition(&gone, Some(&gone)),
        WriteRequest::new(()),
    )
    .await
    .expect("delete");
    // An EHR with content and no party.
    let orphan = crate::fixtures::create_ehr(&svc).await;
    svc.create_composition(
        orphan,
        crate::fixtures::uv(&crate::fixtures::composition("visit e"), "249", None),
    )
    .await
    .expect("orphan composition");

    let aql = "SELECT p/uid/value, c/name/value \
               FROM PERSON p CONTAINS EHR e CONTAINS COMPOSITION c \
               ORDER BY p/details[at0001]/items[at0002]/value/id DESC, c/name/value";
    let rows = run(&svc, aql, AqlQueryRequest::default()).await;
    assert_eq!(
        rows,
        vec![
            json!([bram, "visit c"]),
            json!([ada, "visit a"]),
            json!([ada, "visit b"]),
        ],
        "joined, ordered by a party key then a clinical key"
    );

    let page = run(
        &svc,
        aql,
        AqlQueryRequest {
            offset: Some(1),
            fetch: Some(1),
            ..AqlQueryRequest::default()
        },
    )
    .await;
    assert_eq!(
        page,
        vec![json!([ada, "visit a"])],
        "the page is cut from the join"
    );

    let rows = run(
        &svc,
        "SELECT c/name/value FROM PERSON p CONTAINS EHR e CONTAINS COMPOSITION c \
         WHERE p/details[at0001]/items[at0002]/value/id = $identifier \
         AND c/name/value = 'visit b'",
        params(&[("identifier", "P-1")]),
    )
    .await;
    assert_eq!(rows, vec![json!(["visit b"])], "each side's WHERE applies");

    for (aql, names) in [
        (
            "SELECT COUNT(*) FROM PERSON p CONTAINS EHR e CONTAINS COMPOSITION c",
            "aggregate",
        ),
        (
            "SELECT DISTINCT c/name/value FROM PERSON p CONTAINS EHR e CONTAINS COMPOSITION c",
            "DISTINCT",
        ),
        (
            "SELECT c/name/value FROM PERSON p CONTAINS EHR e CONTAINS COMPOSITION c \
             WHERE p/name/value = c/name/value",
            "both",
        ),
    ] {
        let err = refuse(&svc, aql, AqlQueryRequest::default()).await;
        assert!(
            matches!(&err, ServiceError::BadRequest(r) if r.kind == Malformed::Precondition),
            "{aql}"
        );
        assert!(err.message().contains(names), "{aql}: {}", err.message());
    }
}

/// A query rooted at a demographic class joins over the population, so its
/// clinical half runs over the linked EHRs as a population subset (#4247): an
/// EHR whose `EHR_STATUS.is_queryable` is false answers no row (RM
/// `org.openehr.rm.ehr.ehr_status.adoc`: "True if this EHR should be included
/// in population queries").
#[tokio::test]
async fn a_join_leaves_out_a_non_queryable_ehr() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);

    let ada = linked(&svc, "Ada", "P-1", &["visit a"]).await;
    let bram = create(&svc, PartyKind::Person, &person("Bram", "P-2", "urn:bsn")).await;
    let bram_party = ferroehr::ids::VoId(
        bram.split("::")
            .next()
            .and_then(|s| s.parse().ok())
            .expect("party id"),
    );
    let hidden = crate::fixtures::create_ehr_for_party(&svc, bram_party).await;
    svc.create_composition(
        hidden,
        crate::fixtures::uv(&crate::fixtures::composition("visit b"), "249", None),
    )
    .await
    .expect("create_composition");
    crate::fixtures::set_not_queryable(&svc, hidden).await;

    let rows = run(
        &svc,
        "SELECT p/uid/value, c/name/value FROM PERSON p CONTAINS EHR e CONTAINS COMPOSITION c",
        AqlQueryRequest::default(),
    )
    .await;
    assert_eq!(
        rows,
        vec![json!([ada, "visit a"])],
        "the non-queryable EHR of {bram} answers no joined row"
    );
}

/// The party's versioned-object id: its `uid/value` before the first `::`
/// (RM common master06 §Version Identification).
fn party_id(uid: &str) -> &str {
    uid.split("::").next().expect("a versioned-object id")
}

/// An `EHR_STATUS` whose subject is a `PARTY_SELF` with an `external_ref`
/// naming `id` in `namespace` as an object of class `class` (RM ehr master04
/// §EHR Status; BASE `OBJECT_REF`), written as `POST /ehr` receives it.
fn status_naming(id: &str, namespace: &str, class: &str) -> openehr_rm::prelude::EhrStatus {
    openehr_its::json::from_canonical_value(&json!({
        "_type": "EHR_STATUS",
        "archetype_node_id": "openEHR-EHR-EHR_STATUS.generic.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-EHR-EHR_STATUS.generic.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "EHR Status" },
        "subject": {
            "_type": "PARTY_SELF",
            "external_ref": {
                "_type": "PARTY_REF",
                "namespace": namespace,
                "type": class,
                "id": { "_type": "HIER_OBJECT_ID", "value": id }
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    }))
    .expect("the subject-bearing EHR_STATUS decodes")
}

/// An EHR whose subject names `id` as `class` in `namespace`, holding one
/// composition named `composition`.
async fn subject_ehr(
    svc: &FerroEhrService,
    id: &str,
    namespace: &str,
    class: &str,
    composition: &str,
) -> ferroehr::ids::EhrId {
    let ehr =
        Box::pin(svc.create_ehr_meta(WriteRequest::new(Some(status_naming(id, namespace, class)))))
            .await
            .unwrap_or_else(|e| panic!("create_ehr naming {class} {id}: {e:?}"))
            .0;
    svc.create_composition(
        ehr,
        crate::fixtures::uv(&crate::fixtures::composition(composition), "249", None),
    )
    .await
    .unwrap_or_else(|e| panic!("create_composition ({composition}): {e:?}"));
    ehr
}

/// An EHR created with an `EHR_STATUS.subject.external_ref` that names a party
/// joins to that party, for each root, the abstract `PARTY` and `ACTOR`
/// included, in any
/// namespace, and when the reference names the root's abstract RM ancestor
/// (BASE `OBJECT_REF.type`: "the class (concrete or abstract)"; `PARTY_REF`
/// admits `PARTY` and `ACTOR`). A subject naming no party, a subject naming
/// the party under another class, and a deleted party answer nothing
/// (#4411).
#[tokio::test]
async fn an_ehr_whose_subject_names_the_party_joins() {
    let db = testkit::db().await.expect("testkit database");
    let svc = new_service(&db);

    let ada = create(&svc, PartyKind::Person, &person("Ada", "P-1", "urn:bsn")).await;
    let ada_ehr = subject_ehr(&svc, party_id(&ada), "local", "PERSON", "visit a").await;
    let pia = create(&svc, PartyKind::Person, &person("Pia", "P-2", "urn:bsn")).await;
    let pia_ehr = subject_ehr(&svc, party_id(&pia), "demographic", "PARTY", "visit p").await;
    let acme = create(
        &svc,
        PartyKind::Organisation,
        &organisation("Acme", "O-1", "urn:agb"),
    )
    .await;
    let acme_ehr = subject_ehr(&svc, party_id(&acme), "local", "ORGANISATION", "visit o").await;
    let nurse = create(&svc, PartyKind::Role, &role("Nurse", "R-1", "urn:big")).await;
    let nurse_ehr = subject_ehr(&svc, party_id(&nurse), "local", "ROLE", "visit r").await;
    let ward = create(&svc, PartyKind::Group, &group("Ward 3", "G-1", "urn:ward")).await;
    let ward_ehr = subject_ehr(&svc, party_id(&ward), "local", "GROUP", "visit w").await;
    let analyser = create(
        &svc,
        PartyKind::Agent,
        &agent("Analyser", "A-1", "urn:device"),
    )
    .await;
    let analyser_ehr = subject_ehr(&svc, party_id(&analyser), "local", "ACTOR", "visit d").await;

    // A second PERSON named in the `local` namespace: one EHR, one row.
    let both = create(&svc, PartyKind::Person, &person("Both", "P-3", "urn:bsn")).await;
    let both_ehr = subject_ehr(&svc, party_id(&both), "local", "PERSON", "visit b").await;

    // The subject names the PERSON as an ORGANISATION: no PERSON row.
    let otto = create(&svc, PartyKind::Person, &person("Otto", "P-4", "urn:bsn")).await;
    subject_ehr(&svc, party_id(&otto), "local", "ORGANISATION", "visit x").await;
    // The subject names no party.
    subject_ehr(
        &svc,
        "dddddddd-dddd-4ddd-8ddd-dddddddddddd",
        "local",
        "PERSON",
        "visit n",
    )
    .await;
    // The party is deleted after its EHR was created.
    let gone = create(&svc, PartyKind::Person, &person("Gone", "P-5", "urn:bsn")).await;
    subject_ehr(&svc, party_id(&gone), "local", "PERSON", "visit g").await;
    svc.party_delete(
        PartyKind::Person,
        crate::fixtures::object(&gone),
        crate::fixtures::delete_precondition(&gone, Some(&gone)),
        WriteRequest::new(()),
    )
    .await
    .expect("delete");

    let rows = run(
        &svc,
        "SELECT p/uid/value, e/ehr_id/value FROM PERSON p CONTAINS EHR e ORDER BY p/name/value",
        AqlQueryRequest::default(),
    )
    .await;
    assert_eq!(
        rows,
        vec![
            json!([ada, ada_ehr.to_string()]),
            json!([both, both_ehr.to_string()]),
            json!([pia, pia_ehr.to_string()]),
        ],
        "each PERSON joins to the EHR its subject names, once"
    );

    let rows = run(
        &svc,
        "SELECT p/uid/value, c/name/value \
         FROM PERSON p CONTAINS EHR e CONTAINS COMPOSITION c ORDER BY p/name/value",
        AqlQueryRequest::default(),
    )
    .await;
    assert_eq!(
        rows,
        vec![
            json!([ada, "visit a"]),
            json!([both, "visit b"]),
            json!([pia, "visit p"]),
        ],
        "the content below the EHR joins too"
    );

    for (root, uid, ehr, composition) in [
        ("ORGANISATION", &acme, acme_ehr, "visit o"),
        ("ROLE", &nurse, nurse_ehr, "visit r"),
        ("GROUP", &ward, ward_ehr, "visit w"),
        ("AGENT", &analyser, analyser_ehr, "visit d"),
    ] {
        let rows = run(
            &svc,
            &format!("SELECT p/uid/value, e/ehr_id/value FROM {root} p CONTAINS EHR e"),
            AqlQueryRequest::default(),
        )
        .await;
        assert_eq!(rows, vec![json!([uid, ehr.to_string()])], "{root}");
        let rows = run(
            &svc,
            &format!(
                "SELECT p/uid/value, c/name/value FROM {root} p CONTAINS EHR e CONTAINS COMPOSITION c"
            ),
            AqlQueryRequest::default(),
        )
        .await;
        assert_eq!(rows, vec![json!([uid, composition])], "{root} content");
    }

    // The abstract roots join each party under its own class: the PERSON
    // named as an ORGANISATION joins under neither, and a ROLE is no ACTOR.
    for (root, expected) in [
        (
            "PARTY",
            vec![
                (&acme, acme_ehr),
                (&ada, ada_ehr),
                (&analyser, analyser_ehr),
                (&both, both_ehr),
                (&nurse, nurse_ehr),
                (&pia, pia_ehr),
                (&ward, ward_ehr),
            ],
        ),
        (
            "ACTOR",
            vec![
                (&acme, acme_ehr),
                (&ada, ada_ehr),
                (&analyser, analyser_ehr),
                (&both, both_ehr),
                (&pia, pia_ehr),
                (&ward, ward_ehr),
            ],
        ),
    ] {
        let rows = run(
            &svc,
            &format!(
                "SELECT p/uid/value, e/ehr_id/value FROM {root} p CONTAINS EHR e ORDER BY p/name/value"
            ),
            AqlQueryRequest::default(),
        )
        .await;
        let expected: Vec<Value> = expected
            .iter()
            .map(|(uid, ehr)| json!([uid, ehr.to_string()]))
            .collect();
        assert_eq!(rows, expected, "{root} joins each party under its class");
    }

    for root in ["PERSON", "PARTY", "ACTOR"] {
        let rows = run(
            &svc,
            &format!("SELECT e/ehr_id/value FROM {root} p CONTAINS EHR e WHERE p/uid/value = $uid"),
            params(&[("uid", &otto)]),
        )
        .await;
        assert!(
            rows.is_empty(),
            "a subject naming the PERSON as an ORGANISATION answers nothing under {root}: {rows:?}"
        );
    }
}

/// A join that reaches its EHR through the subject reference alone still
/// leaves the party read on the party-domain record and the crossing on the
/// linkage-domain record, each counting what it served (#4411).
#[tokio::test]
async fn a_subject_join_is_recorded_in_the_party_and_linkage_domains() {
    use ferroehr::system_log::config::{AuditConfig, StoreConfig};
    use ferroehr::system_log::sender::{AuditHandle, start};

    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let audit_config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention: None,
            sgb_v_309_controller: false,
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            ..StoreConfig::default()
        },
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit_config, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender);

    let ada = create(&svc, PartyKind::Person, &person("Ada", "P-1", "urn:bsn")).await;
    subject_ehr(&svc, party_id(&ada), "local", "PERSON", "visit a").await;
    let rows = run(
        &svc,
        "SELECT p/uid/value, e/ehr_id/value FROM PERSON p CONTAINS EHR e",
        AqlQueryRequest::default(),
    )
    .await;
    assert_eq!(rows.len(), 1, "the subject join answers its row: {rows:?}");

    for (domain, resource) in [("linkage", "party-ehr:query"), ("party", "party-query")] {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let count = loop {
            let found: Option<Option<i64>> = sqlx::query_scalar(
                "SELECT result_count FROM audit.audit_event \
                 WHERE domain = $1 AND resource_id = $2 ORDER BY recorded_at DESC LIMIT 1",
            )
            .bind(domain)
            .bind(resource)
            .fetch_optional(&pool)
            .await
            .expect("read the access log");
            if let Some(count) = found {
                break count;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "the {domain} read was not recorded within the drain window"
            );
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
        };
        assert_eq!(count, Some(1), "the {domain} record counts one {resource}");
    }
}
