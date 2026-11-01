// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `POST /query/cohort` on the wire.
//!
//! **OUR OWN EXTENSION: no openEHR spec governs this route** — ITS-REST 1.1.0
//! publishes an ad-hoc and a stored query and nothing else. What the suite pins
//! is the wire contract the platform behaviour hangs off: a bound deployment
//! serves the cohort with its `meta.cohort` block over parties and EHRs created
//! only over the wire, an unbound predicate is a
//! `400` in the openEHR error shape, and a deployment that binds nothing
//! answers `404` as if the surface were not there.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::Router;
use ferroehr::service::FerroEhrService;
use ferroehr::service::linkage::cohort::config::{CohortConfig, PredicateBinding, PredicateKind};
use http::StatusCode;
use serde_json::{Value, json};

use crate::common;

/// The address CLUSTER archetype the city predicate binds to — a CLUSTER under
/// the party's `details`, which the node codec decomposes into its own row.
const ADDRESS_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-CLUSTER.address.v1";
/// The person archetype the seeded parties carry.
const PERSON_ARCHETYPE: &str = "openEHR-DEMOGRAPHIC-PERSON.person.v1";

/// A deployment that binds the city predicate and suppresses nothing.
fn bound_config() -> CohortConfig {
    CohortConfig {
        small_cell_threshold: 0,
        max_cohort_size: 100_000,
        predicates: BTreeMap::from([(
            "city".to_owned(),
            PredicateBinding {
                archetype: ADDRESS_ARCHETYPE.to_owned(),
                node: "at0012".to_owned(),
                kind: PredicateKind::Text,
            },
        )]),
        subjects: false,
    }
}

/// A PERSON whose address CLUSTER carries `city`.
fn person(name: &str, city: &str) -> Value {
    json!({
        "_type": "PERSON",
        "archetype_node_id": PERSON_ARCHETYPE,
        "archetype_details": { "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": PERSON_ARCHETYPE },
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
            "archetype_node_id": "at0001",
            "name": { "_type": "DV_TEXT", "value": "structure" },
            "items": [{
                "_type": "CLUSTER",
                "archetype_node_id": ADDRESS_ARCHETYPE,
                "archetype_details": { "_type": "ARCHETYPED",
                    "archetype_id": { "_type": "ARCHETYPE_ID", "value": ADDRESS_ARCHETYPE },
                    "rm_version": "1.1.0" },
                "name": { "_type": "DV_TEXT", "value": "address" },
                "items": [{
                    "_type": "ELEMENT",
                    "archetype_node_id": "at0012",
                    "name": { "_type": "DV_TEXT", "value": "city" },
                    "value": { "_type": "DV_TEXT", "value": city }
                }]
            }]
        }
    })
}

/// A minimal valid RM COMPOSITION named `name`.
fn composition(name: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": "openEHR-EHR-COMPOSITION.encounter.v1",
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID",
                "value": "openEHR-EHR-COMPOSITION.encounter.v1" },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": name },
        "language": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "en" },
        "territory": { "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
            "code_string": "NL" },
        "category": { "_type": "DV_CODED_TEXT", "value": "event",
            "defining_code": { "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "433" } },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "cohort tester" }
    })
}

/// An `EHR_STATUS` whose subject is a `PARTY_SELF` with an `external_ref`
/// naming `id` in `namespace` as a `PERSON` (RM ehr master04 §EHR Status).
fn status_naming(id: &str, namespace: &str) -> Value {
    json!({
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
                "id": { "_type": "HIER_OBJECT_ID", "value": id },
                "namespace": namespace,
                "type": "PERSON"
            }
        },
        "is_queryable": true,
        "is_modifiable": true
    })
}

/// `POST /ehr` with the `EHR_STATUS` naming `id` in `namespace`, then one
/// `POST /ehr/{ehr_id}/composition` named `name`; answers the EHR id.
async fn ehr_naming(app: &Router, id: &str, namespace: &str, name: &str) -> String {
    let (status, headers, body) = common::send(
        app,
        common::post_json(
            &format!("{}/ehr", common::BASE),
            &status_naming(id, namespace).to_string(),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "the EHR is created: {body}");
    let ehr = common::etag_uid(&headers);
    let (status, body) = common::send_body(
        app,
        common::post_json(
            &format!("{}/ehr/{ehr}/composition", common::BASE),
            &composition(name).to_string(),
        ),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::CREATED,
        "the composition is committed: {body}"
    );
    ehr
}

/// Seed one PERSON in `city` per name over `POST /demographic/person`, the EHR
/// whose subject names it over `POST /ehr`, and one composition in it.
async fn seed(app: &Router, city: &str, names: &[&str]) {
    for name in names {
        let (status, headers, body) = common::send(
            app,
            common::post_json(
                &format!("{}/demographic/person", common::BASE),
                &person(name, city).to_string(),
            ),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "the PERSON is created: {body}");
        let uid = common::etag_uid(&headers);
        let party = uid.split("::").next().expect("a versioned-object id");
        ehr_naming(app, party, "local", name).await;
    }
}

/// The request body every case posts.
fn body(predicate: &str, value: &str) -> String {
    json!({
        "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
        "cohort": [{ "name": predicate, "value": value }],
        "purpose": "secondary-use"
    })
    .to_string()
}

/// A bound deployment serves the cohort, with the cohort facts in `meta`.
#[tokio::test]
async fn a_bound_deployment_serves_a_cohort() {
    let (db, pool) = common::migrated_pool().await;
    let service = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_cohort(bound_config()),
    );
    let app = common::router_with(common::api_config(false), service).await;
    seed(&app, "Groningen", &["Ada", "Bram", "Cato"]).await;
    seed(&app, "Assen", &["Daan"]).await;

    let (status, text) = common::send_body(
        &app,
        common::post_json(
            &format!("{}/query/cohort", common::EXT),
            &body("city", "Groningen"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let served: Value = serde_json::from_str(&text).expect("a RESULT_SET");
    assert_eq!(
        served["rows"].as_array().map(Vec::len),
        Some(3),
        "one row per Groningen composition: {served}"
    );
    let cohort = &served["meta"]["cohort"];
    assert_eq!(cohort["size"], json!(3));
    assert_eq!(cohort["served_ehrs"], json!(3));
    assert_eq!(cohort["suppressed"], json!(false));
    assert_eq!(
        cohort["definition"].as_str().map(str::len),
        Some(64),
        "the cohort is named by a digest, never by its values: {cohort}"
    );
    assert!(
        !text.contains("Groningen"),
        "no predicate value reaches the wire: {text}"
    );
    drop(db);
}

/// A predicate the deployment did not bind is a `400` in the openEHR error
/// shape, naming the predicate so a client can fix its request.
#[tokio::test]
async fn an_unbound_predicate_is_refused() {
    let (db, pool) = common::migrated_pool().await;
    let service = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_cohort(bound_config()),
    );
    let app = common::router_with(common::api_config(false), service).await;

    let (status, text) = common::send_body(
        &app,
        common::post_json(
            &format!("{}/query/cohort", common::EXT),
            &body("national_identifier", "x"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let error: Value = serde_json::from_str(&text).expect("problem details");
    assert!(
        error.get("type").is_some() && error.get("detail").is_some(),
        "the refusal uses the server's one error shape: {text}"
    );
    assert!(
        text.contains("national_identifier"),
        "the refusal names the predicate: {text}"
    );
    drop(db);
}

/// A deployment that binds no predicate answers `404`: the surface is off, and
/// says so the way an unmounted route would.
#[tokio::test]
async fn an_unconfigured_deployment_answers_not_found() {
    let (db, service) = common::test_service().await;
    assert!(!service.cohort_enabled(), "nothing is bound by default");
    let app = common::router_with(common::api_config(false), service).await;

    let (status, text) = common::send_body(
        &app,
        common::post_json(
            &format!("{}/query/cohort", common::EXT),
            &body("city", "Groningen"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    drop(db);
}

/// A body that names `subjects` instead of predicates is served under
/// `[cohort] subjects = true` and the local provider, each subject resolved
/// through the `EHR_STATUS.subject` the EHR was created with over the wire,
/// and no identifier value reaches the wire.
#[tokio::test]
async fn a_cohort_names_its_subjects_by_identifier() {
    use ferroehr::service::ehr_index::provider::EhrIndexProvider;

    const DOMAIN: &str = "urn:oid:2.999.1.4114";
    let (db, pool) = common::migrated_pool().await;
    let service = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_ehr_index(EhrIndexProvider::Local)
            .with_cohort(CohortConfig {
                small_cell_threshold: 0,
                subjects: true,
                ..CohortConfig::default()
            }),
    );
    let app = common::router_with(common::api_config(false), service).await;
    let named = ehr_naming(&app, "wire-a", DOMAIN, "wire-a").await;
    ehr_naming(&app, "wire-b", DOMAIN, "wire-b").await;
    let request = json!({
        "q": "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c",
        "subjects": [{ "id": "wire-a", "namespace": DOMAIN }],
        "purpose": "secondary-use"
    })
    .to_string();

    let (status, text) = common::send_body(
        &app,
        common::post_json(&format!("{}/query/cohort", common::EXT), &request),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{text}");
    let served: Value = serde_json::from_str(&text).expect("a RESULT_SET");
    assert_eq!(served["meta"]["cohort"]["size"], json!(1), "{served}");
    assert_eq!(served["rows"].as_array().map(Vec::len), Some(1), "{served}");
    let (status, body) = common::send_body(
        &app,
        common::get(&format!(
            "{}/ehr/{named}/composition/{}",
            common::BASE,
            served["rows"][0][0].as_str().expect("a composition uid")
        )),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "the served row is the named subject's composition: {body}"
    );
    assert!(
        !text.contains("wire-a"),
        "no identifier reaches the wire: {text}"
    );
    drop(db);
}
