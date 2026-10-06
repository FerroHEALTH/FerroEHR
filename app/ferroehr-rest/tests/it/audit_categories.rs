// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS priority category of every access record, end to end over the
//! assembled router and the local Audit Record Repository (#3621).
//!
//! Regulation (EU) 2025/327 Annex II 3.2(c) asks the logging component for
//! "the categories of data accessed", and Art. 14(1) names the priority
//! categories (`docs/law/eu/ehds/text.html`). The deployment declares which
//! templates and archetypes hold which category (`[audit.categories]`); the
//! record is classified at access from what was served, template first, then
//! root archetype, and from the query's positive constraints where the answer
//! carries no object. No openEHR spec governs the classification — our own
//! design/extension.

#![expect(
    clippy::expect_used,
    clippy::unwrap_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::path::PathBuf;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use axum::Router;
use axum::body::Body;
use ferroehr::config::auth::AuthConfig;
use ferroehr::config::management::{AccessLevel, EndpointLevels, ManagementConfig};
use ferroehr::config::server::ServerConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::categories::CategoryMapConfig;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;
use tracing::field::{Field, Visit};
use tracing::span::{Attributes, Id, Record};
use tracing_subscriber::layer::{Context, Layer};
use tracing_subscriber::prelude::*;
use tracing_subscriber::registry::LookupSpan;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";
const IPS: &str = "International Patient Summary";
const ENCOUNTER: &str = "openEHR-EHR-COMPOSITION.encounter.v1";
const REPORT: &str = "openEHR-EHR-COMPOSITION.report.v1";

/// The archetype map every case runs under, beside any template entries.
fn map(templates: &Value) -> Value {
    json!({
        "templates": templates,
        "archetypes": {
            "openEHR-EHR-COMPOSITION.encounter.v1": ["test-results"],
            "openEHR-EHR-COMPOSITION.report.v1": ["imaging"],
            "openEHR-EHR-COMPOSITION.discharge.v1": ["discharge-report"],
            "openEHR-EHR-OBSERVATION.imaging_exam_result.v0": ["imaging"]
        }
    })
}

/// A minimal valid templateless COMPOSITION rooted at `archetype`.
fn composition(archetype: &str) -> Value {
    json!({
        "_type": "COMPOSITION",
        "archetype_node_id": archetype,
        "archetype_details": {
            "_type": "ARCHETYPED",
            "archetype_id": { "_type": "ARCHETYPE_ID", "value": archetype },
            "rm_version": "1.2.0"
        },
        "name": { "_type": "DV_TEXT", "value": "category test" },
        "language": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_639-1" },
            "code_string": "en"
        },
        "territory": {
            "_type": "CODE_PHRASE",
            "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "ISO_3166-1" },
            "code_string": "NL"
        },
        "category": {
            "_type": "DV_CODED_TEXT",
            "value": "event",
            "defining_code": {
                "_type": "CODE_PHRASE",
                "terminology_id": { "_type": "TERMINOLOGY_ID", "value": "openehr" },
                "code_string": "433"
            }
        },
        "composer": { "_type": "PARTY_IDENTIFIED", "name": "category tester" }
    })
}

fn opt_xml() -> String {
    std::fs::read_to_string(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/openehr-its/tests/fixtures/sdk/ips.v0.opt"),
    )
    .expect("ips.v0.opt vendored in openehr-its")
}

/// The IPS canonical composition, its stored `uid` removed.
fn ips_composition() -> Value {
    let text = std::fs::read_to_string(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(
        "../../crates/openehr-its/tests/vendor/openehr_sdk/composition/canonical_json/ips_canonical.json",
    ))
    .expect("ips_canonical.json vendored in openehr-its");
    let mut value: Value = serde_json::from_str(&text).expect("valid canonical composition");
    value.as_object_mut().unwrap().remove("uid");
    value
}

fn config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            swagger_ui: AccessLevel::Off,
            ..Default::default()
        },
        auth: AuthConfig {
            enabled: false,
            ..AuthConfig::default()
        },
        ..Default::default()
    }
}

/// An audited app over a fresh database with the local store on and `map` in
/// force, the IPS template uploaded and one EHR created; returns the EHR id.
///
/// The template upload and the EHR create are themselves audited; every case
/// selects the records it asserts on by operation.
async fn app(
    map: Value,
    observability: Observability,
) -> (testkit::TestDb, PgPool, Router, String) {
    let (db, pool) = common::migrated_pool().await;
    let categories: CategoryMapConfig = serde_json::from_value(map).expect("the map parses");
    let audit = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        categories,
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = Arc::new(
        FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
            .with_audit(sender),
    );
    let app = ferroehr_rest::build_full(config(), svc, None, observability).expect("build app");
    let (status, _, body) = send(
        &app,
        Request::post(format!("{BASE}/definition/template/adl1.4"))
            .header(header::CONTENT_TYPE, "application/xml")
            .body(Body::from(opt_xml()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "OPT upload: {body}");
    let (status, headers, _) = send(
        &app,
        Request::post(format!("{BASE}/ehr"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "EHR create");
    let ehr = etag(&headers);
    (db, pool, app, ehr)
}

async fn send(app: &Router, req: Request<Body>) -> (StatusCode, http::HeaderMap, String) {
    let resp = app.clone().oneshot(req).await.expect("response");
    let status = resp.status();
    let headers = resp.headers().clone();
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    (
        status,
        headers,
        String::from_utf8_lossy(&bytes).into_owned(),
    )
}

fn etag(headers: &http::HeaderMap) -> String {
    headers
        .get(header::ETAG)
        .and_then(|v| v.to_str().ok())
        .expect("ETag")
        .trim_start_matches("W/")
        .trim_matches('"')
        .to_owned()
}

/// Commit a canonical composition; returns its versioned-object id.
async fn commit(app: &Router, ehr: &str, body: &Value) -> String {
    let (status, headers, text) = send(
        app,
        Request::post(format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {text}");
    etag(&headers).split("::").next().unwrap().to_owned()
}

/// Run an ad-hoc AQL query.
async fn query(app: &Router, aql: &str) {
    let (status, _, text) = send(
        app,
        Request::post(format!("{BASE}/query/aql"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(json!({ "q": aql }).to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "query {aql}: {text}");
}

/// One stored classification.
#[derive(Debug)]
struct Classified {
    categories: Value,
    basis: Option<String>,
    evidence: Value,
    digest: Option<String>,
    fhir: Value,
}

/// The four classification columns and the FHIR document of one stored record.
type StoredClassification = (
    Option<Value>,
    Option<String>,
    Option<Value>,
    Option<String>,
    Value,
);

/// The classification of the newest record of `operation` on `resource_class`,
/// polled until the drain has written it.
async fn classified(pool: &PgPool, operation: &str, resource_class: &str) -> Classified {
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let found: Option<StoredClassification> = sqlx::query_as(
            "SELECT categories, category_basis, category_evidence, category_map_digest, fhir \
                 FROM audit.audit_event WHERE operation = $1 AND resource_class = $2 \
                 ORDER BY chain_seq DESC LIMIT 1",
        )
        .bind(operation)
        .bind(resource_class)
        .fetch_optional(pool)
        .await
        .expect("read the trail");
        if let Some((categories, basis, evidence, digest, fhir)) = found {
            return Classified {
                categories: categories.unwrap_or(Value::Null),
                basis,
                evidence: evidence.unwrap_or(Value::Null),
                digest,
                fhir,
            };
        }
        assert!(
            std::time::Instant::now() < deadline,
            "no {operation} record on {resource_class} landed"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

/// A query serving two categories records both, on the statement record and
/// on the record of the EHR that disclosed them, with the map's digest and the
/// FHIR entity details beside them.
#[tokio::test]
async fn a_multi_category_query_records_every_category_it_served() {
    let (_db, pool, app, ehr) = app(map(&json!({})), Observability::default()).await;
    commit(&app, &ehr, &composition(ENCOUNTER)).await;
    commit(&app, &ehr, &composition(REPORT)).await;

    query(&app, "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c").await;

    let statement = classified(&pool, "query_execute_adhoc_query_body", "query").await;
    assert_eq!(statement.categories, json!(["imaging", "test-results"]));
    assert_eq!(statement.basis.as_deref(), Some("archetype"));
    assert_eq!(
        statement.evidence,
        json!([ENCOUNTER.to_ascii_lowercase(), REPORT.to_ascii_lowercase()])
    );
    let digest = statement.digest.expect("the map digest");
    assert!(digest.starts_with("sha256:"), "{digest}");
    let entities = statement.fhir["entity"].as_array().expect("entities");
    let category_entity = entities
        .iter()
        .find(|e| e["name"] == "categories of the data accessed")
        .expect("the FHIR rendering carries the categories");
    let details = category_entity["detail"].as_array().expect("details");
    assert!(
        details
            .iter()
            .any(|d| d["type"] == "category" && d["valueString"] == "imaging"),
        "{category_entity}"
    );
    assert!(
        details
            .iter()
            .any(|d| d["type"] == "category-map" && d["valueString"] == digest.as_str()),
        "{category_entity}"
    );

    let per_ehr = classified(&pool, "query_execute_adhoc_query_body", "ehr").await;
    assert_eq!(per_ehr.categories, json!(["imaging", "test-results"]));
}

/// A template the map does not name, on a root archetype it does not name
/// either, is recorded `unclassified` with both ids as evidence, and the read
/// is served.
#[tokio::test]
async fn an_unmapped_template_is_unclassified_with_its_ids_as_evidence() {
    let (_db, pool, app, ehr) = app(map(&json!({})), Observability::default()).await;
    let vo = commit(&app, &ehr, &ips_composition()).await;
    let (status, _, _) = send(
        &app,
        Request::get(format!("{BASE}/ehr/{ehr}/composition/{vo}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(
        status,
        StatusCode::OK,
        "an unclassified read is never refused"
    );

    let read = classified(&pool, "composition_get", "composition").await;
    assert_eq!(read.categories, json!(["unclassified"]));
    assert_eq!(read.basis.as_deref(), Some("archetype"));
    assert_eq!(
        read.evidence,
        json!([IPS, "openehr-ehr-composition.health_summary.v1"])
    );
}

/// A leaf-only answer of a `DISTINCT` projection carries no object, so the
/// query's positive archetype constraint classifies it.
#[tokio::test]
async fn a_leaf_only_query_is_classified_from_its_constraints() {
    let (_db, pool, app, ehr) = app(map(&json!({})), Observability::default()).await;
    commit(&app, &ehr, &composition(ENCOUNTER)).await;

    query(
        &app,
        "SELECT DISTINCT c/name/value FROM EHR e \
         CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1]",
    )
    .await;

    let statement = classified(&pool, "query_execute_adhoc_query_body", "query").await;
    assert_eq!(statement.categories, json!(["test-results"]));
    assert_eq!(statement.basis.as_deref(), Some("query"));
    assert_eq!(statement.evidence, json!([ENCOUNTER]));
}

/// A query that serves nothing is still classified by what it asked for.
#[tokio::test]
async fn a_zero_row_query_is_classified_from_its_constraints() {
    let (_db, pool, app, _ehr) = app(map(&json!({})), Observability::default()).await;

    query(
        &app,
        "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.discharge.v1]",
    )
    .await;

    let statement = classified(&pool, "query_execute_adhoc_query_body", "query").await;
    assert_eq!(statement.categories, json!(["discharge-report"]));
    assert_eq!(statement.basis.as_deref(), Some("query"));
}

/// An operand under `NOT CONTAINS` is excluded content, so its category never
/// reaches the record.
#[tokio::test]
async fn a_not_contains_operand_never_counts() {
    let (_db, pool, app, ehr) = app(map(&json!({})), Observability::default()).await;
    commit(&app, &ehr, &composition(ENCOUNTER)).await;

    query(
        &app,
        "SELECT COUNT(*) FROM EHR e \
         CONTAINS COMPOSITION c[openEHR-EHR-COMPOSITION.encounter.v1] \
         NOT CONTAINS OBSERVATION o[openEHR-EHR-OBSERVATION.imaging_exam_result.v0]",
    )
    .await;

    let statement = classified(&pool, "query_execute_adhoc_query_body", "query").await;
    assert_eq!(statement.categories, json!(["test-results"]));
    assert_eq!(statement.basis.as_deref(), Some("query"));
}

/// A FLAT write is classified by the template it was committed against.
#[tokio::test]
async fn a_flat_write_is_classified_by_its_template() {
    let templates = json!({ IPS: ["patient-summary"] });
    let (_db, pool, app, ehr) = app(map(&templates), Observability::default()).await;

    let opt = openehr_its::opt14::from_xml(&opt_xml()).expect("parse OPT");
    let web_template =
        openehr_sdt::flat::webtemplate::builder::build_web_template(&opt).expect("web template");
    let flat = openehr_sdt::flat::convert::composition_to_flat(&ips_composition(), &web_template)
        .expect("flatten");
    let flat_map: serde_json::Map<String, Value> = flat.into_iter().collect();
    let (status, _, text) = send(
        &app,
        Request::post(format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::CONTENT_TYPE, "application/openehr.wt.flat+json")
            .header("openehr-template-id", IPS)
            .body(Body::from(serde_json::to_string(&flat_map).unwrap()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "flat commit: {text}");

    let write = classified(&pool, "composition_create", "composition").await;
    assert_eq!(write.categories, json!(["patient-summary"]));
    assert_eq!(write.basis.as_deref(), Some("template"));
    assert_eq!(write.evidence, json!([IPS]));
}

/// Commit a canonical composition; returns its full version uid.
async fn commit_version(app: &Router, ehr: &str, body: &Value) -> String {
    let (status, headers, text) = send(
        app,
        Request::post(format!("{BASE}/ehr/{ehr}/composition"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::CREATED, "commit: {text}");
    etag(&headers)
}

/// A logical delete writes no content, so its record carries the categories of
/// the version it deleted (#3653).
#[tokio::test]
async fn a_logical_delete_is_classified_by_the_version_it_deletes() {
    let (_db, pool, app, ehr) = app(map(&json!({})), Observability::default()).await;
    let uid = commit_version(&app, &ehr, &composition(ENCOUNTER)).await;
    let (status, _, text) = send(
        &app,
        Request::delete(format!("{BASE}/ehr/{ehr}/composition/{uid}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::NO_CONTENT, "delete: {text}");

    let delete = classified(&pool, "composition_delete", "composition").await;
    assert_eq!(delete.categories, json!(["test-results"]));
    assert_eq!(delete.basis.as_deref(), Some("archetype"));
}

/// A `VERSIONED_COMPOSITION` read is classified by the COMPOSITION it holds
/// (#3653).
#[tokio::test]
async fn a_versioned_composition_read_is_classified_by_its_composition() {
    let (_db, pool, app, ehr) = app(map(&json!({})), Observability::default()).await;
    let vo = commit(&app, &ehr, &composition(REPORT)).await;
    let (status, _, text) = send(
        &app,
        Request::get(format!("{BASE}/ehr/{ehr}/versioned_composition/{vo}"))
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "versioned composition: {text}");

    let read = classified(&pool, "versioned_composition_get", "composition").await;
    assert_eq!(read.categories, json!(["imaging"]));
    assert_eq!(read.basis.as_deref(), Some("archetype"));
}

/// A CONTRIBUTION read is classified by the versions it committed, with and
/// without `Prefer: resolve_refs` (#3653).
#[tokio::test]
async fn a_contribution_read_is_classified_by_its_versions() {
    let templates = json!({ IPS: ["patient-summary"] });
    let (_db, pool, app, ehr) = app(map(&templates), Observability::default()).await;
    let uid = commit_version(&app, &ehr, &ips_composition()).await;
    let vo = uid.split("::").next().unwrap().to_owned();
    let (status, _, text) = send(
        &app,
        Request::get(format!(
            "{BASE}/ehr/{ehr}/versioned_composition/{vo}/version/{uid}"
        ))
        .body(Body::empty())
        .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "version envelope: {text}");
    let envelope: Value = serde_json::from_str(&text).unwrap();
    let contribution = envelope["contribution"]["id"]["value"]
        .as_str()
        .expect("the version names its contribution")
        .to_owned();

    for prefer in [None, Some("return=representation, resolve_refs")] {
        let mut request = Request::get(format!("{BASE}/ehr/{ehr}/contribution/{contribution}"));
        if let Some(prefer) = prefer {
            request = request.header("prefer", prefer);
        }
        let (status, _, text) = send(&app, request.body(Body::empty()).unwrap()).await;
        assert_eq!(status, StatusCode::OK, "contribution {prefer:?}: {text}");

        let read = classified(&pool, "contribution_get", "contribution").await;
        assert_eq!(read.categories, json!(["patient-summary"]), "{prefer:?}");
        assert_eq!(read.basis.as_deref(), Some("template"), "{prefer:?}");
        assert_eq!(read.evidence, json!([IPS]), "{prefer:?}");
    }
}

/// A tracing layer recording every span field and event field it sees.
#[derive(Clone, Default)]
struct Capture(Arc<Mutex<Vec<String>>>);

#[derive(Default)]
struct Fields(Vec<String>);

impl Visit for Fields {
    fn record_str(&mut self, field: &Field, value: &str) {
        self.0.push(format!("{}={value}", field.name()));
    }
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        self.0.push(format!("{}={value:?}", field.name()));
    }
}

impl<S> Layer<S> for Capture
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    fn on_new_span(&self, attrs: &Attributes<'_>, _id: &Id, _ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        attrs.record(&mut fields);
        self.0.lock().unwrap().extend(fields.0);
    }

    fn on_record(&self, _id: &Id, values: &Record<'_>, _ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        values.record(&mut fields);
        self.0.lock().unwrap().extend(fields.0);
    }

    fn on_event(&self, event: &tracing::Event<'_>, _ctx: Context<'_, S>) {
        let mut fields = Fields::default();
        event.record(&mut fields);
        self.0.lock().unwrap().extend(fields.0);
    }
}

/// The global meter provider and its Prometheus registry, set once per test
/// process.
fn registry() -> &'static prometheus::Registry {
    static REGISTRY: OnceLock<prometheus::Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        let (provider, registry) = ferroehr::telemetry::metrics::build_provider(
            opentelemetry_sdk::Resource::builder().build(),
            None::<opentelemetry_sdk::metrics::PeriodicReader<opentelemetry_otlp::MetricExporter>>,
        )
        .expect("build the meter provider");
        opentelemetry::global::set_meter_provider(provider);
        ferroehr::telemetry::metrics::init(&opentelemetry::global::meter(
            ferroehr::telemetry::metrics::SCOPE,
        ));
        registry
    })
}

/// The classification is audit content: neither the template id it rests on
/// nor the category it yields appears in a span, a log event or a metric label
/// of the classified request.
#[tokio::test]
async fn the_classification_never_reaches_spans_logs_or_metrics() {
    let observability = Observability {
        management: ManagementConfig {
            enabled: true,
            endpoints: EndpointLevels {
                prometheus: AccessLevel::Public,
                ..EndpointLevels::default()
            },
            ..ManagementConfig::default()
        },
        prometheus: Some(registry().clone()),
        ..Observability::default()
    };
    let templates = json!({ IPS: ["patient-summary"] });
    let (_db, pool, app, ehr) = app(map(&templates), observability).await;
    let vo = commit(&app, &ehr, &ips_composition()).await;

    let capture = Capture::default();
    let lines = Arc::clone(&capture.0);
    let read = {
        let _guard = tracing::subscriber::set_default(tracing_subscriber::registry().with(capture));
        send(
            &app,
            Request::get(format!("{BASE}/ehr/{ehr}/composition/{vo}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
    };
    assert_eq!(read.0, StatusCode::OK, "the classified read is served");
    let record = classified(&pool, "composition_get", "composition").await;
    assert_eq!(
        record.categories,
        json!(["patient-summary"]),
        "the record itself is classified"
    );

    let lines = lines.lock().unwrap().clone();
    assert!(!lines.is_empty(), "the capture saw the request");
    for line in &lines {
        assert!(
            !line.contains(IPS) && !line.contains("patient-summary"),
            "the classification leaked into telemetry: {line}"
        );
    }

    let (status, _, exposition) = send(
        &app,
        Request::get("/management/prometheus")
            .body(Body::empty())
            .unwrap(),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "the exposition is served");
    assert!(!exposition.is_empty(), "the exposition carries metrics");
    for needle in [IPS, "patient-summary", "health_summary"] {
        assert!(
            !exposition.contains(needle),
            "a metric label carries {needle}: {exposition}"
        );
    }
}
