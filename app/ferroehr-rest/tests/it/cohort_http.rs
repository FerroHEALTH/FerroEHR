// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `POST /query/cohort` on the wire.
//!
//! **OUR OWN EXTENSION: no openEHR spec governs this route** — ITS-REST 1.1.0
//! publishes an ad-hoc and a stored query and nothing else. What the suite pins
//! is the wire contract the platform behaviour hangs off: a bound deployment
//! serves the cohort with its `meta.cohort` block, an unbound predicate is a
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

/// The SM `UPDATE_VERSION` commit envelope for a bare-RM create.
fn uv<T: serde::de::DeserializeOwned>(
    data: &Value,
) -> openehr_its::rest::generated::common::UpdateVersion<T> {
    use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};
    UpdateVersion {
        preceding_version_uid: None,
        lifecycle_state: ferroehr::service::version_update::lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the fixture commit body decodes as its RM type"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: ferroehr::service::version_update::change_type_coded("249"),
            description: None,
            committer: openehr_its::json::from_canonical_value(
                &json!({ "_type": "PARTY_IDENTIFIED", "name": "cohort tester" }),
            )
            .expect("committer"),
            additional_properties: BTreeMap::new(),
        }),
        signature: None,
        additional_properties: BTreeMap::new(),
    }
}

/// Seed three linked subjects in `city`, each with one composition.
async fn seed(service: &FerroEhrService, city: &str, names: &[&str]) {
    for name in names {
        let party = Box::pin(service.create_party(uv(&person(name, city))))
            .await
            .expect("create_party");
        let ehr = service.create_ehr(None).await.expect("create_ehr");
        service.link(party, ehr).await.expect("link");
        service
            .create_composition(ehr, uv(&composition(name)))
            .await
            .expect("create_composition");
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
    seed(&service, "Groningen", &["Ada", "Bram", "Cato"]).await;
    seed(&service, "Assen", &["Daan"]).await;
    let app = common::router_with(common::api_config(false), service);

    let (status, text) = common::send_body(
        &app,
        common::post_json(
            &format!("{}/query/cohort", common::BASE),
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
    let app = common::router_with(common::api_config(false), service);

    let (status, text) = common::send_body(
        &app,
        common::post_json(
            &format!("{}/query/cohort", common::BASE),
            &body("national_identifier", "x"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{text}");
    let error: Value = serde_json::from_str(&text).expect("an openEHR error body");
    assert!(
        error.get("message").is_some() && error.get("error").is_some(),
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
    let app = common::router_with(common::api_config(false), service);

    let (status, text) = common::send_body(
        &app,
        common::post_json(
            &format!("{}/query/cohort", common::BASE),
            &body("city", "Groningen"),
        ),
    )
    .await;
    assert_eq!(status, StatusCode::NOT_FOUND, "{text}");
    drop(db);
}
