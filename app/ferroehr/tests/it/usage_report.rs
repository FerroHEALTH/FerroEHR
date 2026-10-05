// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The usage report client against a real database and a local mock collector.
//!
//! No openEHR spec governs this — our own design. Every payload a test sends is
//! validated against `FerroPULSE`'s report v1 JSON Schemas, vendored with their
//! own valid and invalid examples under `corpus/ferropulse/`; the examples run
//! through the validator first, so a validator that accepts everything cannot
//! pass the suite.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this module's \
              helpers; a failing fixture must panic at the fixture (the Rust Book ch11)"
)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ferroehr::db::domain::DomainPools;
use ferroehr::licence::state::LicenceState;
use ferroehr::service::FerroEhrService;
use ferroehr::service::query::request::AqlQueryRequest;
use ferroehr::telemetry::build_info::BuildInfo;
use ferroehr::telemetry::health::{HealthIndicator, HealthRegistry, HealthStatus};
use ferroehr::telemetry::indicators::{DbHealth, MigrationsHealth};
use ferroehr::usage_report::config::{Deployment, UsageReportConfig};
use ferroehr::usage_report::reporter::{
    DailyOutcome, EventKind, Identity, StartOutcome, UsageReporter, start,
};
use ferroehr::usage_report::window::{record_aql, record_request};
use http::StatusCode;
use serde_json::Value;
use sqlx::PgPool;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The `$id` the envelope schema's `$ref` to the metrics schema resolves to.
const METRICS_SCHEMA_ID: &str = "https://ferropulse.eu/schemas/ferroehr-metrics.v1.json";

/// The vendored contract.
fn contract_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/ferropulse/schemas")
}

fn read_json(file: &Path) -> Value {
    let text = std::fs::read_to_string(file).expect("read a vendored contract file");
    serde_json::from_str(&text).expect("a vendored contract file is JSON")
}

/// The envelope validator, with the metrics schema registered under its `$id`
/// so the envelope's `$ref` resolves offline.
fn envelope_validator() -> jsonschema::Validator {
    let metrics = read_json(&contract_dir().join("ferroehr-metrics.v1.json"));
    let envelope = read_json(&contract_dir().join("report-envelope.v1.json"));
    let registry = jsonschema::Registry::new()
        .add(METRICS_SCHEMA_ID, metrics)
        .expect("the metrics schema id is a URI")
        .prepare()
        .expect("the registry prepares");
    jsonschema::options()
        .with_registry(&registry)
        .build(&envelope)
        .expect("the envelope schema compiles")
}

fn metrics_validator() -> jsonschema::Validator {
    jsonschema::validator_for(&read_json(&contract_dir().join("ferroehr-metrics.v1.json")))
        .expect("the metrics schema compiles")
}

/// Every JSON file under `dir`, sorted.
fn examples(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read an examples directory")
        .map(|entry| entry.expect("read a directory entry").path())
        .filter(|file| file.extension().is_some_and(|ext| ext == "json"))
        .collect();
    files.sort();
    files
}

/// Asserts `body` is a valid report, naming every violation otherwise.
fn assert_valid_report(validator: &jsonschema::Validator, body: &Value) {
    let errors: Vec<String> = validator
        .iter_errors(body)
        .map(|error| format!("{} at {}", error, error.instance_path()))
        .collect();
    assert!(
        errors.is_empty(),
        "the report breaks the contract: {errors:?}\n{body}"
    );
}

/// The licence the build embeds: a real verified licence with a licence id
/// and a licensee, neither of which may reach a report.
fn embedded_licence() -> LicenceState {
    let anchors = ferroehr::licence::anchors().expect("the embedded anchors parse");
    LicenceState::load_now(
        &ferroehr::licence::config::LicenceConfig::default(),
        ferroehr::licence::EMBEDDED_TOKEN,
        &anchors,
    )
}

fn identity() -> Identity {
    Identity::new(
        &BuildInfo::current(),
        &embedded_licence(),
        Deployment::Compose,
    )
}

fn config_for(server: &MockServer) -> UsageReportConfig {
    UsageReportConfig {
        endpoint: format!("{}/v1/report", server.uri()),
        ..UsageReportConfig::default()
    }
}

/// A clinical pool of its own over the test database, as one replica holds.
fn replica_pool(db: &testkit::TestDb) -> PgPool {
    DomainPools::from_shared(&db.pool()).clinical
}

fn reporter(db: &testkit::TestDb, server: &MockServer) -> UsageReporter {
    UsageReporter::new(
        config_for(server),
        replica_pool(db),
        identity(),
        Instant::now(),
    )
    .expect("the client builds")
}

async fn collector(status: u16) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/v1/report"))
        .respond_with(ResponseTemplate::new(status))
        .mount(&server)
        .await;
    server
}

/// The bodies the collector received, in arrival order.
async fn received(server: &MockServer) -> Vec<Value> {
    server
        .received_requests()
        .await
        .expect("request recording is on")
        .iter()
        .map(|request| {
            assert_eq!(
                request
                    .headers
                    .get("content-type")
                    .and_then(|value| value.to_str().ok()),
                Some("application/json"),
                "a report is sent as JSON"
            );
            serde_json::from_slice(&request.body).expect("the body is JSON")
        })
        .collect()
}

/// The keys of a JSON object, sorted.
fn keys(value: &Value) -> Vec<String> {
    let mut keys: Vec<String> = value
        .as_object()
        .expect("an object")
        .keys()
        .cloned()
        .collect();
    keys.sort();
    keys
}

/// A body with its uptime removed, the one member two renderings a moment
/// apart may differ in.
fn without_uptime(mut body: Value) -> Value {
    if let Some(object) = body.as_object_mut() {
        object.remove("uptime_s");
    }
    body
}

/// Simulates the passage of the start rate limit.
async fn age_the_start_claim(pool: &PgPool) {
    sqlx::query(
        "UPDATE usage_report_instance \
         SET last_start_sent_at = last_start_sent_at - interval '11 minutes'",
    )
    .execute(pool)
    .await
    .expect("age the start claim");
}

/// Traffic over several route groups and AQL, recorded the way the HTTP layer
/// and the AQL engine record it.
fn record_traffic() {
    let base = "/ferroehr/rest/openehr/v1";
    for _ in 0..40 {
        record_request(
            &format!("{base}/ehr/{{ehr_id}}/composition/{{uid_based_id}}"),
            Duration::from_millis(12),
            StatusCode::OK,
        );
    }
    record_request(
        &format!("{base}/ehr/{{ehr_id}}/composition"),
        Duration::from_millis(7_500),
        StatusCode::INTERNAL_SERVER_ERROR,
    );
    record_request(
        &format!("{base}/query/aql"),
        Duration::from_millis(60),
        StatusCode::OK,
    );
    record_request(
        &format!("{base}/ehr"),
        Duration::from_millis(3),
        StatusCode::CREATED,
    );
    record_aql(Duration::from_millis(40));
    record_aql(Duration::from_millis(1_500));
}

// ── the validator itself ──────────────────────────────────────────────────────

/// The vendored examples decide the validator first: every valid example
/// passes and every invalid one fails, for both schemas.
#[test]
fn the_contract_examples_decide_the_validator() {
    let root = contract_dir().join("examples");
    let cases = [
        ("report-envelope.v1", envelope_validator()),
        ("ferroehr-metrics.v1", metrics_validator()),
    ];
    for (schema, validator) in cases {
        let valid = examples(&root.join(schema).join("valid"));
        let invalid = examples(&root.join(schema).join("invalid"));
        assert!(
            !valid.is_empty() && !invalid.is_empty(),
            "{schema} has no examples"
        );
        for file in valid {
            assert_valid_report(&validator, &read_json(&file));
        }
        for file in invalid {
            assert!(
                !validator.is_valid(&read_json(&file)),
                "{} must be refused",
                file.display()
            );
        }
    }
}

// ── payload shape ─────────────────────────────────────────────────────────────

/// Both events validate against the contract, and the v1 field set is pinned:
/// a field added to the payload fails here before it reaches a collector.
#[tokio::test]
async fn start_and_daily_payloads_carry_exactly_the_v1_fields() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let reporter = reporter(&db, &server);
    let validator = envelope_validator();

    assert_eq!(
        reporter.send_start().await.expect("start sent"),
        StartOutcome::Sent(StatusCode::NO_CONTENT)
    );
    record_traffic();
    assert_eq!(
        reporter.send_daily().await.expect("daily sent"),
        DailyOutcome::Sent(StatusCode::NO_CONTENT)
    );

    let bodies = received(&server).await;
    let [start, daily] = bodies.as_slice() else {
        panic!("expected one start and one daily report: {bodies:?}");
    };
    assert_valid_report(&validator, start);
    assert_valid_report(&validator, daily);

    let envelope = [
        "cpu_bucket",
        "db_ok",
        "deployment",
        "event",
        "git_sha",
        "instance_id",
        "licence",
        "memory_bucket",
        "migrations_ok",
        "postgres_major",
        "product",
        "schema",
        "spec_profile",
        "uptime_s",
        "version",
    ];
    let with = |extra: &str| {
        let mut set: Vec<String> = envelope.iter().map(|key| (*key).to_owned()).collect();
        set.push(extra.to_owned());
        set.sort();
        set
    };
    assert_eq!(keys(start), with("first_start"));
    assert_eq!(keys(daily), with("metrics"));
    assert_eq!(keys(&daily["metrics"]), ["window_24h"]);
    assert_eq!(
        keys(&daily["metrics"]["window_24h"]),
        ["aql", "requests_bucket", "routes"]
    );
    assert_eq!(
        keys(&daily["metrics"]["window_24h"]["aql"]),
        ["executions", "histogram", "p95_ms", "slow"]
    );
    assert_eq!(
        keys(&daily["metrics"]["window_24h"]["routes"]["composition"]),
        ["errors_5xx", "histogram", "p50_ms", "p95_ms", "p99_ms"]
    );

    assert_eq!(start["event"], "start");
    assert_eq!(start["schema"], 1);
    assert_eq!(start["product"], "ferroehr");
    assert_eq!(start["licence"], "non-commercial");
    assert_eq!(start["deployment"], "compose");
    assert_eq!(start["db_ok"], true);
    assert_eq!(start["migrations_ok"], true);
    assert_eq!(start["postgres_major"], 18);
    assert_eq!(start["instance_id"], daily["instance_id"]);

    let window = &daily["metrics"]["window_24h"];
    assert_eq!(window["requests_bucket"], "1-100");
    assert_eq!(keys(&window["routes"]), ["composition", "ehr", "query"]);
    let composition = &window["routes"]["composition"];
    assert_eq!(composition["errors_5xx"], 1);
    assert_eq!(
        composition["histogram"],
        serde_json::json!([0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 1])
    );
    assert_eq!(window["aql"]["executions"], 2);
    assert_eq!(window["aql"]["slow"], 1, "the default threshold is 1000 ms");
}

/// No route parameter, AQL text or licence identity can reach a report: a
/// concrete path handed to the window keeps only its group, an executed query
/// leaves only its duration, and the licence in force is reported as its grant
/// type alone.
#[tokio::test]
async fn no_route_parameter_query_text_or_licence_id_reaches_the_payload() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let reporter = reporter(&db, &server);

    let sentinel_ehr = "5e171e1c-0000-4000-8000-5e171e1c0001";
    record_request(
        &format!("/ferroehr/rest/openehr/v1/ehr/{sentinel_ehr}/composition/SENTINEL-UID::x::1"),
        Duration::from_millis(9),
        StatusCode::OK,
    );
    let service = FerroEhrService::new(&DomainPools::from_shared(&db.pool()));
    let aql = "SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_id/value = 'SENTINEL-AQL-LITERAL'";
    service
        .execute_ad_hoc_query(aql.to_owned(), AqlQueryRequest::default())
        .await
        .expect("the query executes");

    reporter.send_start().await.expect("start sent");
    reporter.send_daily().await.expect("daily sent");

    let LicenceState::Licensed { verified, .. } = embedded_licence() else {
        panic!("the build embeds a licence");
    };
    let licence_id = verified.licence.id.to_string();
    let licensee = verified.licence.licensee.clone();
    let bodies = received(&server).await;
    assert_eq!(bodies.len(), 2);
    for body in &bodies {
        let text = body.to_string();
        for forbidden in [
            sentinel_ehr,
            "SENTINEL-UID",
            "SENTINEL-AQL-LITERAL",
            "SELECT",
            licence_id.as_str(),
            licensee.as_str(),
        ] {
            assert!(
                !text.contains(forbidden),
                "`{forbidden}` reached the report: {text}"
            );
        }
        assert_eq!(body["licence"], "non-commercial");
    }
    let daily = bodies.last().expect("the daily report");
    let window = &daily["metrics"]["window_24h"];
    assert_eq!(window["aql"]["executions"], 1);
    // The query ran through the service, not the HTTP layer, so it adds no route.
    assert_eq!(keys(&window["routes"]), ["composition"]);
}

// ── behaviour ────────────────────────────────────────────────────────────────

/// Two replicas on one database claim the daily window at the same moment and
/// exactly one report reaches the collector.
#[tokio::test]
async fn one_daily_report_per_window_across_two_replicas() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let first = reporter(&db, &server);
    let second = reporter(&db, &server);

    let (a, b) = tokio::join!(first.send_daily(), second.send_daily());
    let outcomes = [a.expect("first replica"), b.expect("second replica")];
    let sent = outcomes
        .iter()
        .filter(|outcome| matches!(outcome, DailyOutcome::Sent(_)))
        .count();
    assert_eq!(sent, 1, "exactly one replica sends: {outcomes:?}");
    assert!(
        outcomes
            .iter()
            .any(|outcome| matches!(outcome, DailyOutcome::NotDue(_))),
        "the other replica finds the window claimed: {outcomes:?}"
    );

    // A later attempt in the same window, from either replica, sends nothing.
    assert!(matches!(
        first.send_daily().await.expect("first replica again"),
        DailyOutcome::NotDue(_)
    ));
    let bodies = received(&server).await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(
        bodies.first().map(|body| &body["event"]),
        Some(&Value::from("daily"))
    );
}

/// A second start report within ten minutes is not sent, from this replica or
/// another one.
#[tokio::test]
async fn the_start_report_is_rate_limited_to_one_per_ten_minutes() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let first = reporter(&db, &server);
    let second = reporter(&db, &server);

    assert!(matches!(
        first.send_start().await.expect("first start"),
        StartOutcome::Sent(_)
    ));
    assert_eq!(
        first.send_start().await.expect("restart loop"),
        StartOutcome::RateLimited
    );
    assert_eq!(
        second.send_start().await.expect("another replica"),
        StartOutcome::RateLimited
    );
    assert_eq!(received(&server).await.len(), 1);
}

/// `first_start` is true for the start that created the instance id and false
/// after a restart, which keeps the id.
#[tokio::test]
async fn first_start_is_true_on_a_fresh_database_and_false_after_a_restart() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let pool = replica_pool(&db);

    reporter(&db, &server)
        .send_start()
        .await
        .expect("first boot");
    age_the_start_claim(&pool).await;
    reporter(&db, &server).send_start().await.expect("restart");

    let bodies = received(&server).await;
    let [first, restart] = bodies.as_slice() else {
        panic!("expected two start reports: {bodies:?}");
    };
    assert_eq!(first["first_start"], true);
    assert_eq!(restart["first_start"], false);
    assert_eq!(first["instance_id"], restart["instance_id"]);
}

/// `ferroehr usage-report --print` renders through [`UsageReporter::preview`],
/// which must equal what a send then carries, for both events.
#[tokio::test]
async fn the_preview_equals_what_is_sent() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let reporter = reporter(&db, &server);
    let pool = replica_pool(&db);

    reporter
        .send_start()
        .await
        .expect("the instance is created");
    age_the_start_claim(&pool).await;
    let start_preview = reporter.preview(EventKind::Start).await.expect("preview");
    assert!(start_preview.instance_stored);
    reporter.send_start().await.expect("start sent");

    record_traffic();
    let daily_preview = reporter.preview(EventKind::Daily).await.expect("preview");
    reporter.send_daily().await.expect("daily sent");

    let bodies = received(&server).await;
    let [_, start, daily] = bodies.as_slice() else {
        panic!("expected three reports: {bodies:?}");
    };
    let rendered = |preview: &ferroehr::usage_report::reporter::Preview| {
        serde_json::to_value(&preview.report).expect("the preview renders")
    };
    assert_eq!(
        without_uptime(rendered(&start_preview)),
        without_uptime(start.clone())
    );
    assert_eq!(
        without_uptime(rendered(&daily_preview)),
        without_uptime(daily.clone())
    );
}

/// The preview writes nothing: on a fresh database it shows a sample id and
/// leaves no instance behind.
#[tokio::test]
async fn the_preview_writes_nothing() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let preview = reporter(&db, &server)
        .preview(EventKind::Start)
        .await
        .expect("preview");
    assert!(!preview.instance_stored);
    let stored = ferroehr::usage_report::store::read_instance(&replica_pool(&db))
        .await
        .expect("read the instance");
    assert_eq!(stored, None);
    assert!(received(&server).await.is_empty());
}

/// With the switch off, nothing is started, sent or stored.
#[tokio::test]
async fn nothing_is_sent_when_disabled() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let pool = replica_pool(&db);
    let config = UsageReportConfig {
        enabled: false,
        ..config_for(&server)
    };

    let handle = start(&config, &pool, identity(), Instant::now());
    assert!(handle.is_none(), "a disabled report starts no task");
    let stored = ferroehr::usage_report::store::read_instance(&pool)
        .await
        .expect("read the instance");
    assert_eq!(stored, None, "a disabled report stores no instance id");
    assert!(received(&server).await.is_empty());
}

/// A collector that never answers holds up neither boot nor readiness: the
/// start returns at once while the request hangs, and readiness reads UP.
#[tokio::test]
async fn boot_and_readiness_are_unaffected_when_the_collector_hangs() {
    let db = testkit::db().await.expect("testkit database");
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(204).set_delay(Duration::from_secs(600)))
        .mount(&server)
        .await;
    let pool = replica_pool(&db);

    let booted = Instant::now();
    let handle = start(&config_for(&server), &pool, identity(), Instant::now())
        .expect("an enabled report starts its task");
    assert!(
        booted.elapsed() < Duration::from_secs(1),
        "starting the report must not wait on the collector"
    );

    // The start report is in flight and unanswered.
    let deadline = Instant::now() + Duration::from_secs(30);
    while received(&server).await.is_empty() {
        assert!(Instant::now() < deadline, "the start report never left");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(
        !handle.is_finished(),
        "the send is still waiting on the collector"
    );

    let indicators: Vec<Arc<dyn HealthIndicator>> = vec![
        Arc::new(DbHealth::new(pool.clone())),
        Arc::new(MigrationsHealth::new(pool.clone())),
    ];
    let readiness = tokio::time::timeout(
        Duration::from_secs(5),
        HealthRegistry::new(indicators).evaluate(),
    )
    .await
    .expect("readiness answers while the collector hangs");
    assert_eq!(readiness.status, HealthStatus::Up);
    handle.abort();
}
