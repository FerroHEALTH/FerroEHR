// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The usage report, `ferrohealth-usage-report` wired over FerroEHR's
//! PostgreSQL store, against a real database and a local mock collector.
//!
//! No openEHR spec governs this — our own design. Every payload a test sends is
//! validated against `FerroPULSE`'s report v2 JSON Schema, vendored with its
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
use ferroehr::telemetry::indicators::{DbHealth, MigrationsHealth};
use ferroehr::usage_report::product::{self, EventKind, FerroEhr, NOTICE, Preview};
use ferroehr::usage_report::store::PgStore;
use ferrohealth_health::document::State;
use ferrohealth_health::lifecycle::Lifecycle;
use ferrohealth_health::registry::{HealthIndicator, Registry};
use ferrohealth_usage_report::reporter::{
    Identity, ReportError, Reporter, StartOutcome, start_with_store,
};
use ferrohealth_usage_report::section::{Deployment, Endpoint, UsageReportSection};
use ferrohealth_usage_report::window::Window;
use http::{Method, StatusCode};
use serde_json::Value;
use sqlx::PgPool;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// The default `server.base_path`, the mount the recorded routes sit under.
const BASE_PATH: &str = "/ferroehr/rest/openehr/v1";

/// The vendored contract.
fn contract_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/ferropulse/schemas")
}

fn read_json(file: &Path) -> Value {
    let text = std::fs::read_to_string(file).expect("read a vendored contract file");
    serde_json::from_str(&text).expect("a vendored contract file is JSON")
}

/// The envelope validator; `status` and `metrics` are `$defs` of the one
/// schema, so every `$ref` resolves inside it.
fn envelope_validator() -> jsonschema::Validator {
    jsonschema::validator_for(&read_json(&contract_dir().join("report-envelope.v2.json")))
        .expect("the envelope schema compiles")
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
    product::identity(&BuildInfo::current(), &embedded_licence())
}

fn config_for(server: &MockServer) -> UsageReportSection {
    UsageReportSection {
        endpoint: Endpoint::new(&format!("{}/v1/report", server.uri()))
            .expect("the mock collector is on loopback"),
        deployment: Deployment::Compose,
        ..UsageReportSection::default()
    }
}

/// A clinical pool of its own over the test database, as one replica holds.
fn replica_pool(db: &testkit::TestDb) -> PgPool {
    DomainPools::from_shared(&db.pool()).clinical
}

/// The installation id the database holds, without creating one.
async fn stored_instance(pool: &PgPool) -> Result<Option<uuid::Uuid>, sqlx::Error> {
    sqlx::query_scalar("SELECT instance_id FROM usage_report_instance")
        .fetch_optional(pool)
        .await
}

/// One replica's reporter over the shared database, with its own window.
async fn reporter(
    db: &testkit::TestDb,
    server: &MockServer,
    window: &Arc<Window>,
) -> Reporter<FerroEhr> {
    replica(db, &config_for(server), window).await
}

/// One replica's reporter to the collector `config` names.
async fn replica(
    db: &testkit::TestDb,
    config: &UsageReportSection,
    window: &Arc<Window>,
) -> Reporter<FerroEhr> {
    let pool = replica_pool(db);
    let instance = ferroehr::usage_report::store::ensure_instance(&pool)
        .await
        .expect("the instance id");
    Reporter::with_store(
        config,
        identity(),
        (instance, Arc::new(PgStore::new(pool.clone()))),
        Arc::new(FerroEhr::new(pool)),
        Arc::clone(window),
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

/// Starts the report the way the server boots it: the crate's
/// `start_with_store` over FerroEHR's store, product and window.
async fn start_report(
    config: &UsageReportSection,
    pool: &PgPool,
    ci: Option<&'static str>,
) -> Option<tokio::task::JoinHandle<()>> {
    start_with_store(
        config,
        &NOTICE,
        identity(),
        Arc::new(PgStore::new(pool.clone())),
        Arc::new(FerroEhr::new(pool.clone())),
        product::window(&[BASE_PATH.to_owned()]),
        Instant::now(),
        ci,
    )
    .await
}

/// Traffic over several route groups, recorded the way the request log
/// records it: the matched route template, never the path.
fn record_traffic(window: &Window) {
    let base = "/ferroehr/rest/openehr/v1";
    let record = |method: Method, route: &str, status: StatusCode, ms: u64| {
        window.record(&method, Some(route), status, Duration::from_millis(ms));
    };
    for _ in 0..40 {
        record(
            Method::GET,
            &format!("{base}/ehr/{{ehr_id}}/composition/{{uid_based_id}}"),
            StatusCode::OK,
            12,
        );
    }
    record(
        Method::POST,
        &format!("{base}/ehr/{{ehr_id}}/composition"),
        StatusCode::INTERNAL_SERVER_ERROR,
        7_500,
    );
    record(
        Method::POST,
        &format!("{base}/query/aql"),
        StatusCode::OK,
        60,
    );
    record(Method::POST, &format!("{base}/ehr"), StatusCode::CREATED, 3);
    record(
        Method::GET,
        &format!("{base}/ehr/{{ehr_id}}"),
        StatusCode::NOT_FOUND,
        2,
    );
    record(Method::GET, "/health/readiness", StatusCode::OK, 1);
}

// ── the validator itself ──────────────────────────────────────────────────────

/// The vendored examples decide the validator first: every valid example
/// passes and every invalid one fails.
#[test]
fn the_contract_examples_decide_the_validator() {
    let root = contract_dir().join("examples");
    let cases = [("report-envelope.v2", envelope_validator())];
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

/// Both events validate against the contract, and the v2 field set is pinned:
/// a field added to the payload fails here before it reaches a collector.
#[tokio::test]
async fn start_and_daily_payloads_carry_exactly_the_v2_fields() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let window = product::window(&[BASE_PATH.to_owned()]);
    let reporter = reporter(&db, &server, &window).await;
    let validator = envelope_validator();

    assert_eq!(
        reporter.send_start().await.expect("start sent"),
        StartOutcome::Sent(StatusCode::NO_CONTENT)
    );
    record_traffic(&window);
    assert_eq!(
        reporter.send_daily().await.expect("daily sent"),
        StatusCode::NO_CONTENT
    );

    let bodies = received(&server).await;
    let [start, daily] = bodies.as_slice() else {
        panic!("expected one start and one daily report: {bodies:?}");
    };
    assert_valid_report(&validator, start);
    assert_valid_report(&validator, daily);

    let envelope = [
        "cpu_bucket",
        "deployment",
        "event",
        "git_sha",
        "instance_id",
        "licence",
        "memory_bucket",
        "product",
        "schema",
        "status",
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
    let status = ["db_ok", "migrations_ok", "postgres_major"];
    assert_eq!(keys(&start["status"]), status);
    assert_eq!(keys(&daily["status"]), status);
    assert_eq!(keys(&daily["metrics"]), ["window_24h"]);
    assert_eq!(
        keys(&daily["metrics"]["window_24h"]),
        ["counters", "groups", "requests_bucket"]
    );
    assert_eq!(
        keys(&daily["metrics"]["window_24h"]["counters"]),
        ["responses_4xx"]
    );
    let group_members = ["errors_5xx", "histogram", "p50_ms", "p95_ms", "p99_ms"];
    assert_eq!(
        keys(&daily["metrics"]["window_24h"]["groups"]["composition"]),
        group_members
    );

    assert_eq!(start["event"], "start");
    assert_eq!(start["schema"], 2);
    assert_eq!(start["product"], "ferroehr");
    assert_eq!(start["licence"], "non-commercial");
    assert_eq!(start["deployment"], "compose");
    assert_eq!(start["status"]["db_ok"], true);
    assert_eq!(start["status"]["migrations_ok"], true);
    assert_eq!(start["status"]["postgres_major"], 18);
    assert_eq!(start["instance_id"], daily["instance_id"]);

    let window = &daily["metrics"]["window_24h"];
    assert_eq!(window["requests_bucket"], "1-100");
    assert_eq!(keys(&window["groups"]), ["composition", "ehr", "query"]);
    let composition = &window["groups"]["composition"];
    assert_eq!(composition["errors_5xx"], 1);
    assert_eq!(
        composition["histogram"],
        serde_json::json!([0, 0, 40, 0, 0, 0, 0, 0, 0, 0, 1])
    );
    assert_eq!(window["counters"]["responses_4xx"], 1);
}

/// No route parameter, AQL text or licence identity can reach a report: a
/// concrete path handed to the window keeps only its group, an executed query
/// leaves only its duration, and the licence in force is reported as its grant
/// type alone.
#[tokio::test]
async fn no_route_parameter_query_text_or_licence_id_reaches_the_payload() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let window = product::window(&[BASE_PATH.to_owned()]);
    let reporter = reporter(&db, &server, &window).await;

    let sentinel_ehr = "5e171e1c-0000-4000-8000-5e171e1c0001";
    window.record(
        &Method::GET,
        Some(&format!(
            "/ferroehr/rest/openehr/v1/ehr/{sentinel_ehr}/composition/SENTINEL-UID::x::1"
        )),
        StatusCode::OK,
        Duration::from_millis(9),
    );
    let service = FerroEhrService::new(&DomainPools::from_shared(&db.pool()));
    let aql = "SELECT e/ehr_id/value FROM EHR e WHERE e/ehr_id/value = 'SENTINEL-AQL-LITERAL'";
    service
        .execute_aql(aql, None, &AqlQueryRequest::default())
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
    let counted = &daily["metrics"]["window_24h"];
    // The query ran through the service, not the HTTP layer, so it adds no route.
    assert_eq!(keys(&counted["groups"]), ["composition"]);
}

// ── behaviour ────────────────────────────────────────────────────────────────

/// Two replicas on one database claim the daily window at the same moment and
/// exactly one report reaches the collector.
#[tokio::test]
async fn one_daily_report_per_window_across_two_replicas() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let first = reporter(&db, &server, &product::window(&[BASE_PATH.to_owned()])).await;
    let second = reporter(&db, &server, &product::window(&[BASE_PATH.to_owned()])).await;

    let (a, b) = tokio::join!(first.send_daily(), second.send_daily());
    let outcomes = [a, b];
    let sent = outcomes.iter().filter(|outcome| outcome.is_ok()).count();
    assert_eq!(sent, 1, "exactly one replica sends: {outcomes:?}");
    assert!(
        outcomes
            .iter()
            .any(|outcome| matches!(outcome, Err(ReportError::SentElsewhere))),
        "the other replica finds the window claimed: {outcomes:?}"
    );

    // A later attempt in the same window, from either replica, sends nothing.
    assert!(matches!(
        first.send_daily().await,
        Err(ReportError::SentElsewhere)
    ));
    let bodies = received(&server).await;
    assert_eq!(bodies.len(), 1);
    assert_eq!(
        bodies.first().map(|body| &body["event"]),
        Some(&Value::from("daily"))
    );
}

/// A replica that does not hold the daily claim leaves its counts in the
/// database, and the next daily report carries every replica's counts.
#[tokio::test]
async fn the_daily_report_carries_every_replicas_counts() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let first_window = product::window(&[BASE_PATH.to_owned()]);
    let second_window = product::window(&[BASE_PATH.to_owned()]);
    let first = reporter(&db, &server, &first_window).await;
    let second = reporter(&db, &server, &second_window).await;

    first.send_daily().await.expect("the first daily report");
    record_traffic(&second_window);
    assert!(matches!(
        second.send_daily().await,
        Err(ReportError::SentElsewhere)
    ));
    assert_eq!(
        second_window.take(),
        ferrohealth_usage_report::window::Counts::default(),
        "the counts moved to the store"
    );

    sqlx::query(
        "UPDATE usage_report_instance \
         SET last_daily_sent_at = last_daily_sent_at - interval '1 day'",
    )
    .execute(&replica_pool(&db))
    .await
    .expect("age the daily claim");
    first.send_daily().await.expect("the next daily report");

    let bodies = received(&server).await;
    let [_, daily] = bodies.as_slice() else {
        panic!("expected two daily reports: {bodies:?}");
    };
    let counted = &daily["metrics"]["window_24h"];
    assert_eq!(keys(&counted["groups"]), ["composition", "ehr", "query"]);
    assert_eq!(counted["groups"]["composition"]["errors_5xx"], 1);
    assert_eq!(counted["counters"]["responses_4xx"], 1);
}

/// A daily report that does not reach the collector puts its counts back in
/// the database and releases its claim, so the retry carries them.
#[tokio::test]
async fn an_unsent_daily_report_keeps_its_counts_for_the_retry() {
    let db = testkit::db().await.expect("testkit database");
    // A loopback port nothing listens on: bound, read and released.
    let port = std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|listener| listener.local_addr())
        .expect("a free loopback port")
        .port();
    let config = UsageReportSection {
        endpoint: Endpoint::new(&format!("http://127.0.0.1:{port}/v1/report"))
            .expect("a loopback collector"),
        ..UsageReportSection::default()
    };
    let window = product::window(&[BASE_PATH.to_owned()]);
    let lost = replica(&db, &config, &window).await;
    record_traffic(&window);
    assert!(matches!(
        lost.send_daily().await,
        Err(ReportError::Request(_))
    ));

    let server = collector(204).await;
    let retry = reporter(&db, &server, &product::window(&[BASE_PATH.to_owned()])).await;
    assert_eq!(
        retry.send_daily().await.expect("the retry is sent"),
        StatusCode::NO_CONTENT
    );
    let bodies = received(&server).await;
    let [daily] = bodies.as_slice() else {
        panic!("expected one daily report: {bodies:?}");
    };
    assert_eq!(
        daily["metrics"]["window_24h"]["groups"]["composition"]["errors_5xx"],
        1
    );
}

/// A second start report within ten minutes is not sent, from this replica or
/// another one.
#[tokio::test]
async fn the_start_report_is_rate_limited_to_one_per_ten_minutes() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let first = reporter(&db, &server, &product::window(&[BASE_PATH.to_owned()])).await;
    let second = reporter(&db, &server, &product::window(&[BASE_PATH.to_owned()])).await;

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

    reporter(&db, &server, &product::window(&[BASE_PATH.to_owned()]))
        .await
        .send_start()
        .await
        .expect("first boot");
    age_the_start_claim(&pool).await;
    reporter(&db, &server, &product::window(&[BASE_PATH.to_owned()]))
        .await
        .send_start()
        .await
        .expect("restart");

    let bodies = received(&server).await;
    let [first, restart] = bodies.as_slice() else {
        panic!("expected two start reports: {bodies:?}");
    };
    assert_eq!(first["first_start"], true);
    assert_eq!(restart["first_start"], false);
    assert_eq!(first["instance_id"], restart["instance_id"]);
}

/// `ferroehr usage-report --print` renders through [`product::preview`],
/// which must equal what a send then carries, for both events.
#[tokio::test]
async fn the_preview_equals_what_is_sent() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let window = product::window(&[BASE_PATH.to_owned()]);
    let reporter = reporter(&db, &server, &window).await;
    let pool = replica_pool(&db);
    let config = config_for(&server);
    let preview = |kind| product::preview(&config, identity(), &pool, &window, kind);

    reporter
        .send_start()
        .await
        .expect("the instance is created");
    age_the_start_claim(&pool).await;
    let start_preview = preview(EventKind::Start).await.expect("preview");
    assert!(start_preview.instance_stored);
    // A restart: the next start report comes from a new process's reporter.
    let reporter = self::reporter(&db, &server, &window).await;
    reporter.send_start().await.expect("start sent");

    record_traffic(&window);
    let daily_preview = preview(EventKind::Daily).await.expect("preview");
    reporter.send_daily().await.expect("daily sent");

    let bodies = received(&server).await;
    let [_, start, daily] = bodies.as_slice() else {
        panic!("expected three reports: {bodies:?}");
    };
    let validator = envelope_validator();
    let rendered = |preview: &Preview| {
        let body = serde_json::to_value(&preview.report).expect("the preview renders");
        assert_valid_report(&validator, &body);
        body
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
    let preview = product::preview(
        &config_for(&server),
        identity(),
        &replica_pool(&db),
        &product::window(&[BASE_PATH.to_owned()]),
        EventKind::Start,
    )
    .await
    .expect("preview");
    assert!(!preview.instance_stored);
    let stored = stored_instance(&replica_pool(&db))
        .await
        .expect("read the instance");
    assert_eq!(stored, None);
    assert_eq!(received(&server).await, Vec::<Value>::new());
}

/// With the switch off, nothing is started, sent or stored.
#[tokio::test]
async fn nothing_is_sent_when_disabled() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let pool = replica_pool(&db);
    let config = UsageReportSection {
        enabled: false,
        ..config_for(&server)
    };

    let handle = start_report(&config, &pool, None).await;
    assert!(handle.is_none(), "a disabled report starts no task");
    let stored = stored_instance(&pool).await.expect("read the instance");
    assert_eq!(stored, None, "a disabled report stores no instance id");
    assert_eq!(received(&server).await, Vec::<Value>::new());
}

/// In a CI run the report stays off with the switch on: nothing is started,
/// sent or stored.
#[tokio::test]
async fn nothing_is_sent_in_ci() {
    let db = testkit::db().await.expect("testkit database");
    let server = collector(204).await;
    let pool = replica_pool(&db);
    let config = config_for(&server);
    assert!(config.enabled, "the switch is on");
    let ci = ferrohealth_usage_report::section::ci_of(|name| {
        (name == "GITHUB_ACTIONS").then(|| "true".to_owned())
    });

    let handle = start_report(&config, &pool, ci).await;
    assert!(handle.is_none(), "a CI run starts no task");
    let stored = stored_instance(&pool).await.expect("read the instance");
    assert_eq!(stored, None, "a CI run stores no instance id");
    assert_eq!(received(&server).await, Vec::<Value>::new());
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
    let handle = start_report(&config_for(&server), &pool, None)
        .await
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
    let lifecycle = Lifecycle::default();
    lifecycle.booted();
    let readiness = tokio::time::timeout(
        Duration::from_secs(5),
        Registry::new(indicators).readiness(&lifecycle),
    )
    .await
    .expect("readiness answers while the collector hangs");
    assert_eq!(readiness.state, State::Up);
    handle.abort();
}
