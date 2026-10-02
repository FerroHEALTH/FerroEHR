// SPDX-FileCopyrightText: Vernum Projecten B.V.
// SPDX-License-Identifier: BUSL-1.1

//! End-to-end AQL `TERMINOLOGY('expand', …)` tests (B4 stage (a)) against a real
//! PostgreSQL 18 (shared testkit harness): seed COMPOSITIONs with a coded ELEMENT leaf,
//! then run `matches TERMINOLOGY('expand', …)` / `matches {…, TERMINOLOGY(…)}`
//! through the `QueryService` seam and assert the expansion is merged into the
//! generated `IN (…)` predicate.
//!
//! Two providers are exercised with no network:
//! * the in-process `openehr-term` **bundle** (`service_api = "openehr"`,
//!   `params_uri` = a group id) — group codes as the value set;
//! * a **FHIR R4B** terminology server faked by `wiremock`
//!   (`service_api = "hl7.org/fhir/4.0"`, `ValueSet/$expand`).
//!
//! Spec: master03 §TERMINOLOGY (lines 748–767) + the matches-merge note (lines
//! 756–759).

#![expect(
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;

use serde_json::{Value, json};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

use openehr_rm::prelude::PartyProxy;

use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::service::status::CallStatusType;
use ferroehr::service::terminology::config::{FhirOperation, FhirProviderConfig, ProviderKind};
use ferroehr::service::terminology::fhir::FhirTerminologyProvider;

use ferroehr::service::version_update::{change_type_coded, lifecycle_state_coded};
use openehr_its::rest::generated::common::{UpdateAudit, UpdateAuditData, UpdateVersion};

const OBS_ARCHETYPE: &str = "openEHR-EHR-OBSERVATION.minimal.v1";
/// The coded leaf path (mirrors the `DV_QUANTITY` leaf in `service_aql.rs`, but at
/// `.../value/defining_code/code_string` — the master03 example path).
const CODE_PATH: &str =
    "data[at0001]/events[at0002]/data[at0003]/items[at0004]/value/defining_code/code_string";

/// The SM `UPDATE_VERSION` commit envelope for a bare-RM write.
fn uv<T: serde::de::DeserializeOwned>(data: &Value) -> UpdateVersion<T> {
    UpdateVersion {
        preceding_version_uid: None,
        lifecycle_state: lifecycle_state_coded("532"),
        attestations: None,
        data: openehr_its::json::from_canonical_value(data)
            .expect("the fixture commit body decodes as its RM type"),
        commit_audit: UpdateAudit::UpdateAudit(UpdateAuditData {
            _type: None,
            system_id: None,
            change_type: change_type_coded("249"),
            description: None,
            committer: openehr_its::json::from_canonical_value::<PartyProxy>(
                &json!({ "_type": "PARTY_IDENTIFIED", "name": "conformance tester" }),
            )
            .expect("committer"),
            additional_properties: std::collections::BTreeMap::new(),
        }),
        signature: None,
        additional_properties: std::collections::BTreeMap::new(),
    }
}

/// The base composition, template stripped, uid dropped.
fn base_composition() -> Value {
    let path = format!(
        "{}/../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/minimal_observation.json",
        env!("CARGO_MANIFEST_DIR")
    );
    serde_json::from_str(&std::fs::read_to_string(&path).expect("read fixture"))
        .expect("parse fixture")
}

/// A composition whose OBSERVATION leaf is a `DV_CODED_TEXT` with the given
/// defining code, so `.../value/defining_code/code_string` is queryable.
///
/// The leaf terminology is caller-controlled: a non-openEHR `terminology_id`
/// (e.g. the FHIR test's SNOMED) keeps the code out of the RM-mandated openEHR
/// terminology validation, which only checks RM-fixed coded positions (not
/// archetype-defined ELEMENT values).
fn composition_coded(name: &str, terminology_id: &str, code: &str) -> Value {
    let mut c = base_composition();
    if let Some(details) = c
        .get_mut("archetype_details")
        .and_then(Value::as_object_mut)
    {
        details.remove("template_id");
    }
    if let Some(obj) = c.as_object_mut() {
        obj.remove("uid");
    }
    c["name"] = json!({ "_type": "DV_TEXT", "value": name });
    c["content"][0]["archetype_details"] = json!({
        "_type": "ARCHETYPED",
        "archetype_id": { "_type": "ARCHETYPE_ID", "value": OBS_ARCHETYPE },
        "rm_version": "1.1.0",
    });
    c["content"][0]["data"]["events"][0]["data"]["items"][0]["value"] = json!({
        "_type": "DV_CODED_TEXT",
        "value": name,
        "defining_code": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": terminology_id },
            "code_string": code,
        }
    });
    c
}

async fn create_ehr(svc: &FerroEhrService) -> String {
    svc.create_ehr(None).await.expect("create_ehr").to_string()
}

async fn create_coded(
    svc: &FerroEhrService,
    ehr_id: &str,
    name: &str,
    terminology_id: &str,
    code: &str,
) -> String {
    svc.create_composition(
        ehr_id.parse().expect("ehr_id uuid"),
        uv(&composition_coded(name, terminology_id, code)),
    )
    .await
    .unwrap_or_else(|e| panic!("create_composition ({name}, {code}): {e:?}"))
    .version_uid()
}

async fn run_aql(svc: &FerroEhrService, aql: &str) -> Value {
    svc.execute_ad_hoc_query(aql.to_owned(), AqlQueryRequest::default())
        .await
        .unwrap_or_else(|e| panic!("query {aql:?}: {e:?}"))
        .result_set
}

/// The set of `c/name/value` strings returned by a query (order-independent).
fn names(result: &Value) -> Vec<String> {
    result["rows"]
        .as_array()
        .expect("rows array")
        .iter()
        .map(|row| {
            row.as_array().expect("row array")[0]
                .as_str()
                .expect("name value")
                .to_owned()
        })
        .collect()
}

// ── the bundle provider (no network) ─────────────────────────────────────────

#[tokio::test]
async fn terminology_expand_bundle_merges_group_codes() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = create_ehr(&svc).await;

    // `249` (creation) is in the `audit_change_type` group; `433` (event) is a
    // valid openEHR code outside it.
    create_coded(&svc, &ehr, "in-group", "openehr", "249").await;
    create_coded(&svc, &ehr, "out-of-group", "openehr", "433").await;

    // Standalone expand operand → merged into the IN (…) predicate.
    let aql = format!(
        "SELECT c/name/value \
         FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         WHERE o/{CODE_PATH} matches TERMINOLOGY('expand', 'openehr', 'audit_change_type')"
    );
    assert_eq!(
        names(&run_aql(&svc, &aql).await),
        vec!["in-group".to_owned()]
    );

    // Mixed list: an explicit code merged with the expansion (master03 line 759)
    // — `433` (explicit) and the `audit_change_type` codes both match.
    let mixed = format!(
        "SELECT c/name/value \
         FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         WHERE o/{CODE_PATH} matches {{'433', TERMINOLOGY('expand', 'openehr', 'audit_change_type')}}"
    );
    let mut got = names(&run_aql(&svc, &mixed).await);
    got.sort();
    assert_eq!(got, vec!["in-group".to_owned(), "out-of-group".to_owned()]);
}

#[tokio::test]
async fn terminology_expand_unknown_service_is_bad_request() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = create_ehr(&svc).await;
    create_coded(&svc, &ehr, "x", "openehr", "249").await;

    // An unrecognised service_api (no FHIR provider configured either) → a typed
    // 400, never a 500.
    let aql = format!(
        "SELECT c/name/value \
         FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         WHERE o/{CODE_PATH} matches TERMINOLOGY('expand', 'bogus.terminology.api', 'x')"
    );
    let err = svc
        .execute_ad_hoc_query(aql, AqlQueryRequest::default())
        .await
        .expect_err("unknown service_api must be rejected");
    assert_eq!(err.status, CallStatusType::PreconditionViolation, "{err:?}");
}

#[tokio::test]
async fn terminology_expand_unknown_value_set_is_bad_request() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = create_ehr(&svc).await;
    create_coded(&svc, &ehr, "x", "openehr", "249").await;

    // A value set the bundle does not know → a typed 400.
    let aql = format!(
        "SELECT c/name/value \
         FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         WHERE o/{CODE_PATH} matches TERMINOLOGY('expand', 'openehr', 'no_such_group')"
    );
    let err = svc
        .execute_ad_hoc_query(aql, AqlQueryRequest::default())
        .await
        .expect_err("unknown value set must be rejected");
    assert_eq!(err.status, CallStatusType::PreconditionViolation, "{err:?}");
}

// ── the FHIR provider (wiremock, no network) ─────────────────────────────────

/// The value-set URL and a hierarchical `$expand` response with two SNOMED
/// members (one nested, exercising the recursive `contains` flatten).
const VS_URL: &str = "http://snomed.info/sct?fhir_vs=isa/50697003";

async fn fhir_expand_server() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("GET"))
        .and(path("/ValueSet/$expand"))
        .and(query_param("url", VS_URL))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "resourceType": "ValueSet",
            "status": "active",
            "expansion": {
                "timestamp": "2026-08-04T00:00:00Z",
                "contains": [
                    {"code": "442031002", "display": "Screening for X",
                     "contains": [{"code": "11713004", "display": "child code"}]}
                ]
            }
        })))
        .mount(&server)
        .await;
    server
}

fn fhir_provider(base: &str) -> FhirTerminologyProvider {
    let cfg = FhirProviderConfig {
        kind: ProviderKind::Fhir,
        url: base.to_owned(),
        operation: FhirOperation::Expand,
        connect_timeout_ms: 800,
        request_timeout_ms: 1_500,
        oauth2_client: None,
        client_cert_path: None,
        client_key_path: None,
        ca_bundle_path: None,
        cache_ttl_secs: 0,
        cache_capacity: 1024,
    };
    FhirTerminologyProvider::new("test-fhir", &cfg).expect("build provider")
}

#[tokio::test]
async fn terminology_expand_fhir_merges_expansion_codes() {
    let server = fhir_expand_server().await;
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_external_terminology(Arc::new(fhir_provider(&server.uri())));
    let ehr = create_ehr(&svc).await;

    // SNOMED-coded leaves (non-openEHR terminology → not RM-terminology-checked).
    // The terminology is spelled the way BASE `base_types` master05 spells it,
    // not as the FHIR system URI: a `TERMINOLOGY_ID` is a `name-str`, and the
    // URI belongs in the value-set argument below. The match is on
    // `code_string`, so the spelling is not what this test measures.
    create_coded(&svc, &ehr, "member", "SNOMED-CT", "442031002").await;
    create_coded(&svc, &ehr, "child", "SNOMED-CT", "11713004").await;
    create_coded(&svc, &ehr, "non-member", "SNOMED-CT", "999999").await;

    let aql = format!(
        "SELECT c/name/value \
         FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         WHERE o/{CODE_PATH} matches TERMINOLOGY('expand', 'hl7.org/fhir/4.0', '{VS_URL}')"
    );
    let mut got = names(&run_aql(&svc, &aql).await);
    got.sort();
    // Both the parent and the nested child code are in the flattened expansion.
    assert_eq!(got, vec!["child".to_owned(), "member".to_owned()]);
}

/// A query whose `matches` operand resolves through the terminology service
/// (master03 §TERMINOLOGY) is **never** served from the plan cache: the
/// expansion may change between executions, so the plan is not a pure function
/// of the query text. Running the same terminology query twice therefore never
/// produces a cache hit — it re-parses and re-expands each time. No openEHR spec
/// governs the cache; this asserts the semantics-preserving bypass.
#[tokio::test]
async fn terminology_query_is_never_cached() {
    let db = testkit::db().await.expect("testkit database");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()));
    let ehr = create_ehr(&svc).await;
    create_coded(&svc, &ehr, "in-group", "openehr", "249").await;

    let aql = format!(
        "SELECT c/name/value \
         FROM EHR e CONTAINS COMPOSITION c CONTAINS OBSERVATION o[{OBS_ARCHETYPE}] \
         WHERE o/{CODE_PATH} matches TERMINOLOGY('expand', 'openehr', 'audit_change_type')"
    );

    let before = svc.plan_cache().stats();
    let first = names(&run_aql(&svc, &aql).await);
    let second = names(&run_aql(&svc, &aql).await);
    let after = svc.plan_cache().stats();

    assert_eq!(
        first,
        vec!["in-group".to_owned()],
        "expansion resolved once"
    );
    assert_eq!(
        first, second,
        "and identically on the re-expanded second run"
    );
    assert_eq!(
        after.hits, before.hits,
        "a terminology query is re-expanded every execution — never a cache hit"
    );
    assert_eq!(
        after.misses,
        before.misses + 2,
        "both terminology executions miss the plan cache"
    );
}
