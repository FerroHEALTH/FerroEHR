// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `PERSON` query of Cadasto/FerroHEALTH-backlog#51 enters through an
//! index (#6090): an equality of a party's relationship target with a text
//! that names a party reads the parties whose trunk head names it from
//! `party_relationship_target`, and the answers stay those of the
//! comparison.
//!
//! The comparison keeps its exact text semantics, so the index is a superset
//! entry: it holds the party a reference names as the commit derived it (the
//! root before any `::`, case-folded, a hyphenated UUID), and a text that
//! names no party gets no entry. Its open rows name what each trunk head
//! names, so a query under `ALL_VERSIONS` gets no entry and still answers
//! every version. No openEHR spec governs access paths: our own design.

use std::collections::BTreeMap;

use ferroehr::aql::ir::Params;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use serde_json::{Value, json};
use sqlx::Executor as _;

use crate::aql_range_anchor::rows;
use crate::fixtures::{if_match, object};
use crate::storage_plans::{aql_sql_with, explain_aql, render, scans};
use crate::typed_body::typed;

/// The archetype of the inline relationships.
const RELATIONSHIP: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.relationship.v1";

/// The relationship target leaf of a `PERSON` `p`.
const TARGET: &str = "p/relationships/target/id/value";

/// The date of birth leaf of the PERSON of #6084.
const BIRTH: &str = "p/details/items[at0001]/items[at0010]/value/value";

/// A PERSON named `name` born on `birth`, holding one relationship to each
/// reference of `targets` with `source` as its source.
fn person(name: &str, birth: &str, source: Option<&str>, targets: &[&str]) -> Value {
    let mut body = json!({
        "_type": "PERSON",
        "archetype_node_id": "openEHR-DEMOGRAPHIC-PERSON.person.v1",
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                              "value": "openEHR-DEMOGRAPHIC-PERSON.person.v1" },
            "rm_version": "1.1.0" },
        "name": { "_type": "DV_TEXT", "value": name },
        "identities": [{
            "_type": "PARTY_IDENTITY",
            "archetype_node_id": "at0002",
            "name": { "_type": "DV_TEXT", "value": "legal name" },
            "details": {
                "_type": "ITEM_TREE",
                "archetype_node_id": "at0003",
                "name": { "_type": "DV_TEXT", "value": "structure" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0004",
                    "name": { "_type": "DV_TEXT", "value": "family" },
                    "value": { "_type": "DV_TEXT", "value": name }
                }]
            }
        }],
        "details": {
            "_type": "ITEM_TREE",
            "archetype_node_id": "at0000",
            "name": { "_type": "DV_TEXT", "value": "details" },
            "items": [{
                "_type": "CLUSTER",
                "archetype_node_id": "at0001",
                "name": { "_type": "DV_TEXT", "value": "birth" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0010",
                    "name": { "_type": "DV_TEXT", "value": "date of birth" },
                    "value": { "_type": "DV_DATE", "value": birth }
                }]
            }]
        }
    });
    if let Some(source) = source {
        let relationships: Vec<Value> = targets
            .iter()
            .map(|target| {
                json!({
                    "_type": "PARTY_RELATIONSHIP",
                    "archetype_node_id": RELATIONSHIP,
                    "name": { "_type": "DV_TEXT", "value": "related" },
                    "source": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                                "id": { "_type": "HIER_OBJECT_ID", "value": source } },
                    "target": { "_type": "PARTY_REF", "namespace": "demographic", "type": "PERSON",
                                "id": { "_type": "HIER_OBJECT_ID", "value": target } }
                })
            })
            .collect();
        body["relationships"] = json!(relationships);
    }
    body
}

/// Creates a PERSON and, when `targets` names any, relates it to them in a
/// second version; its versioned-object id and its latest version uid.
async fn related(
    svc: &FerroEhrService,
    name: &str,
    birth: &str,
    targets: &[&str],
) -> (String, String) {
    let created = svc
        .party_create(
            PartyKind::Person,
            WriteRequest::new(typed(&person(name, birth, None, &[]))),
        )
        .await
        .expect("create a person");
    let uid = created.body["uid"]["value"]
        .as_str()
        .expect("a version uid")
        .to_owned();
    let party = uid.split("::").next().expect("a party id").to_owned();
    if targets.is_empty() {
        return (party, uid);
    }
    let updated = svc
        .party_update(
            PartyKind::Person,
            object(&party),
            if_match(&uid),
            WriteRequest::new(typed(&person(name, birth, Some(&party), targets))),
        )
        .await
        .unwrap_or_else(|e| panic!("relate {name}: {e:?}"));
    let uid = updated.body["uid"]["value"]
        .as_str()
        .expect("a version uid")
        .to_owned();
    (party, uid)
}

/// How many relationship-target anchors `sql` carries.
fn target_anchors(sql: &str) -> usize {
    sql.matches("\"party_relationship_target\"").count()
}

/// A text the comparison keeps names the party the index holds, in any case
/// and with a `::` extension; a text naming no party, a query under
/// `ALL_VERSIONS` and a clinical path get no entry; and every anchored
/// query answers what the unanchored one does.
#[tokio::test]
async fn a_relationship_target_anchor_answers_what_the_comparison_keeps() {
    let (_db, svc) = crate::fixtures::service().await;
    let (target, _) = related(&svc, "target", "1960-01-01", &[]).await;
    let (other, _) = related(&svc, "other", "1960-01-02", &[]).await;
    let upper = target.to_uppercase();
    let extended = format!("{target}::ferroehr.example");
    related(&svc, "lower", "1950-01-01", &[&target]).await;
    related(&svc, "upper", "1950-01-01", &[&upper]).await;
    related(&svc, "extended", "1950-01-01", &[&extended]).await;
    related(&svc, "both", "1950-01-02", &[&other, &target]).await;
    related(&svc, "named", "1950-01-01", &["not-a-uuid"]).await;
    related(&svc, "elsewhere", "1950-01-01", &[&other]).await;
    // A party whose earlier version named the target and whose head does not.
    let (left, uid) = related(&svc, "left", "1950-01-01", &[&target]).await;
    svc.party_update(
        PartyKind::Person,
        object(&left),
        if_match(&uid),
        WriteRequest::new(typed(&person("left", "1950-01-01", None, &[]))),
    )
    .await
    .expect("the party stops naming the target");

    let absent = uuid::Uuid::now_v7().to_string();
    let mut answered = 0;
    for text in [
        target.clone(),
        upper.clone(),
        extended.clone(),
        "not-a-uuid".to_owned(),
        absent.clone(),
    ] {
        let names_party = text != "not-a-uuid";
        for test in [
            format!("{TARGET} = '{text}'"),
            format!("{TARGET} matches {{'{text}', '{absent}'}}"),
            format!("{TARGET} = '{text}' AND {BIRTH} matches {{'19500101', '1950-01-01'}}"),
        ] {
            let anchored = format!("SELECT p/uid/value, p/name/value FROM PERSON p WHERE {test}");
            let sql = aql_sql_with(&anchored, &Params::default(), Vec::new(), None)
                .sql
                .to_string();
            assert_eq!(
                target_anchors(&sql),
                usize::from(names_party),
                "{test}: the entry\n{sql}"
            );
            let plain = format!(
                "SELECT p/uid/value, p/name/value FROM PERSON p WHERE ({test}) OR ({test})"
            );
            let unanchored = aql_sql_with(&plain, &Params::default(), Vec::new(), None)
                .sql
                .to_string();
            assert_eq!(target_anchors(&unanchored), 0, "{plain}");
            let with = rows(&svc, &anchored, BTreeMap::new()).await;
            assert_eq!(
                with,
                rows(&svc, &plain, BTreeMap::new()).await,
                "{test}: the entry changes no row"
            );
            answered += with.len();
        }
    }
    assert!(answered > 0, "the texts keep some parties");

    // Under ALL_VERSIONS the party that left still answers its older version.
    let every = format!(
        "SELECT p/uid/value, p/name/value FROM VERSION v[ALL_VERSIONS] CONTAINS PERSON p \
         WHERE {TARGET} = '{target}'"
    );
    let sql = aql_sql_with(&every, &Params::default(), Vec::new(), None)
        .sql
        .to_string();
    assert_eq!(target_anchors(&sql), 0, "ALL_VERSIONS gets no entry\n{sql}");
    let versions = rows(&svc, &every, BTreeMap::new()).await;
    assert!(
        versions.iter().any(|row| row.contains("left")),
        "the older version that named the target answers: {versions:?}"
    );
}

/// The reported query reads its parties through the relationship index: no
/// node scan walks the persons, and the statement reads a fraction of the
/// buffers the same query without its entry reads, which reads the store.
#[tokio::test]
async fn the_reported_person_query_plans_through_an_index() {
    const PERSONS: u32 = 200;
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let pools = ferroehr::db::domain::DomainPools::from_shared(&pool);
    let svc = FerroEhrService::new(&pools);
    let mut targets = Vec::new();
    for index in 0..10 {
        targets.push(
            related(&svc, &format!("target {index}"), "1960-01-01", &[])
                .await
                .0,
        );
    }
    for index in 0..PERSONS {
        let birth = format!(
            "19{:02}-{:02}-{:02}",
            30 + index % 60,
            1 + index % 12,
            1 + index % 28
        );
        let target = &targets[usize::try_from(index % 10).expect("a small index")];
        related(&svc, &format!("person {index}"), &birth, &[target]).await;
    }
    pools
        .party
        .execute("ANALYZE")
        .await
        .expect("give the planner statistics over the seeded parties");

    let reported = format!(
        "SELECT DISTINCT p/uid/value FROM PERSON p WHERE {TARGET} = '{}' \
         AND {BIRTH} matches {{'19500101', '1950-01-01'}} LIMIT 1001",
        targets[3]
    );
    let plan = explain_aql(
        &pools.party,
        aql_sql_with(&reported, &Params::default(), Vec::new(), None),
    )
    .await;
    let scans = scans(&plan);
    assert!(
        scans
            .iter()
            .any(|scan| scan.relation == "party_relationship_target" && scan.index.is_some()),
        "the query enters through the relationship index:\n{}",
        render(&scans)
    );
    let population = f64::from(PERSONS);
    for scan in scans.iter().filter(|scan| scan.relation == "node_hot") {
        assert!(
            scan.rows * scan.loops < population,
            "no node scan walks the {PERSONS} persons:\n{}",
            render(&scans)
        );
    }
    let buffers = |plan: &Value| {
        plan.pointer("/0/Plan/Shared Hit Blocks")
            .and_then(Value::as_f64)
            .expect("the root plan node reports its buffers")
    };
    let unanchored = reported.replace(
        &format!("WHERE {TARGET}"),
        &format!("WHERE {TARGET} = 'x' OR {TARGET}"),
    );
    let without = explain_aql(
        &pools.party,
        aql_sql_with(&unanchored, &Params::default(), Vec::new(), None),
    )
    .await;
    assert!(
        buffers(&without) > population && buffers(&plan) * 4.0 < buffers(&without),
        "the entry reads a fraction of what the same query without it reads, which \
         reads the store: {} against {} buffers\n{}",
        buffers(&plan),
        buffers(&without),
        render(&scans)
    );
}
