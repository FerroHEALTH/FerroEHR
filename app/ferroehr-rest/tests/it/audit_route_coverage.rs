// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Every route of the assembled router writes exactly one access record.
//!
//! The route set is enumerated from the router itself
//! (`extensions_document`, built from the same `utoipa-axum` composition that
//! mounts the routes), each route is driven once by an admin principal of its
//! own, and the local Audit Record Repository must then hold exactly one record
//! of the route's operation id naming that principal. A route added without
//! the audit layer fails here, which is the hazard the logging
//! component's hazard log lists first: an access that is not recorded.
//! Regulation (EU) 2025/327 Annex II 3.2 asks the logging component to record
//! "every access event" (`docs/law/eu/ehds/text.html Annex II 3.2`). No openEHR
//! spec governs access logging beyond "System Log | IHE ATNA-compliant system
//! log" (SM `master02-overview.adoc` §openEHR Platform Model) — the coverage
//! property is our own design.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use axum::Router;
use axum::body::Body;
use ferroehr::config::authz::AuthzConfig;
use ferroehr::config::management::AccessLevel;
use ferroehr::config::server::{AdminConfig, ServerConfig};
use ferroehr::config::smart::SmartConfig;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::sender::start;
use ferroehr::system_log::store::AuditStore;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;
use http::Request;
use serde_json::{Value, json};
use sqlx::PgPool;
use tower::ServiceExt;

use crate::common;

const HMAC_SECRET: &str = "audit-coverage-secret";
const ISSUER: &str = "https://issuer.example";
const AUDIENCE: &str = "ferroehr";

/// The one value substituted for every `{param}` capture: a record is written
/// whatever the outcome, so a probe value only has to route.
const PROBE_PARAM: &str = "3fa85f64-5717-4562-b3fc-2c963f66afa6";

/// The routes that write no access record by design, each with its reason.
///
/// None of them serves or changes patient data: they are the surfaces reachable
/// without a credential, and the operator's management surface, which records
/// under its own `management_*` operation ids rather than the document's.
const EXEMPT: &[(&str, &str, &str)] = &[
    (
        "GET",
        "/health",
        "orchestrator probe outside the API router: no credential, no patient data",
    ),
    (
        "GET",
        "/health/liveness",
        "orchestrator probe outside the API router: no credential, no patient data",
    ),
    (
        "GET",
        "/health/readiness",
        "orchestrator probe outside the API router: no credential, no patient data",
    ),
    (
        "GET",
        "/ferroehr/rest/status",
        "public status document: versions and profile only, no patient data",
    ),
    (
        "GET",
        "/ferroehr/rest/.well-known/smart-configuration",
        "SMART discovery document: server metadata, read before any token exists",
    ),
    (
        "GET",
        "/ferroehr/rest/api-docs/openapi.json",
        "the served OpenAPI document: API description, no patient data",
    ),
    (
        "GET",
        "/ferroehr/rest/api-docs/ferroehr-{family}.openapi.json",
        "the served OpenAPI document: API description, no patient data",
    ),
    (
        "GET",
        "/ferroehr/rest/swagger-ui",
        "static Swagger UI assets, no patient data",
    ),
    (
        "OPTIONS",
        "/ferroehr/rest/openehr/v1",
        "the System OPTIONS manifest: capability metadata, no patient data",
    ),
    (
        "GET",
        "/management/info",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "GET",
        "/management/env",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "GET",
        "/management/prometheus",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "GET",
        "/management/metrics",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "GET",
        "/management/metrics/{name}",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "GET",
        "/management/loggers",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "POST",
        "/management/loggers",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "DELETE",
        "/management/loggers",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "GET",
        "/management/flamegraph",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "GET",
        "/management/health",
        "management surface: node state under management_* operation ids, no patient data",
    ),
    (
        "GET",
        "/management/status",
        "management surface: node state under management_* operation ids, no patient data",
    ),
];

/// Every `(method, path)` the assembled router mounts, with the operation id
/// the served document declares for it.
fn mounted(cfg: &AppConfig) -> BTreeMap<(String, String), Option<String>> {
    let doc = ferroehr_rest::extensions::openapi::extensions_document(cfg);
    let mut out = BTreeMap::new();
    for (path, item) in &doc.paths.paths {
        for (method, operation) in [
            ("GET", &item.get),
            ("PUT", &item.put),
            ("POST", &item.post),
            ("DELETE", &item.delete),
            ("PATCH", &item.patch),
            ("HEAD", &item.head),
            ("OPTIONS", &item.options),
            ("TRACE", &item.trace),
        ] {
            if let Some(operation) = operation {
                out.insert(
                    (method.to_owned(), path.clone()),
                    operation.operation_id.clone(),
                );
            }
        }
    }
    out
}

/// Authentication on with an HMAC bearer (cheap per request), the admin group
/// mounted, and every optional surface the exempt table names switched on so
/// the table is checked against what is served.
fn config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            swagger_ui: AccessLevel::Public,
            ..Default::default()
        },
        auth: common::hs256_auth_config(ISSUER, AUDIENCE, HMAC_SECRET),
        admin: AdminConfig { enabled: true },
        smart: SmartConfig {
            enabled: true,
            ..SmartConfig::default()
        },
        ..Default::default()
    }
}

/// An audited app over a fresh database with the local store on.
async fn app() -> (testkit::TestDb, PgPool, Router) {
    let (db, pool) = common::migrated_pool().await;
    let audit = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    let (sender, _handle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender)
        .with_audit_store(AuditStore::new(pool.clone()));
    let authz = AuthzHandle::build(
        &AuthzConfig::default(),
        &config().server.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new);
    let app = ferroehr_rest::build_full(config(), Arc::new(svc), authz, Observability::default())
        .expect("build app");
    (db, pool, app)
}

/// An admin bearer token for `subject`, so no role gate stops a route before
/// its handler and the record names the probe that caused it.
fn admin_bearer(subject: &str) -> String {
    let exp = u64::try_from(jiff::Timestamp::now().as_second()).expect("timestamp") + 3600;
    let claims: Value = json!({
        "sub": subject,
        "iss": ISSUER,
        "aud": AUDIENCE,
        "exp": exp,
        "roles": ["ADMIN"],
    });
    common::hs256_bearer(HMAC_SECRET, &claims)
}

/// Substitute every `{param}` capture with [`PROBE_PARAM`].
fn concrete(template: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut chars = template.chars();
    while let Some(c) = chars.next() {
        if c == '{' {
            for skipped in chars.by_ref() {
                if skipped == '}' {
                    break;
                }
            }
            out.push_str(PROBE_PARAM);
        } else {
            out.push(c);
        }
    }
    out
}

/// Drive one route as `subject`.
async fn drive(app: &Router, subject: &str, method: &str, path: &str) {
    let request = Request::builder()
        .method(method)
        .uri(concrete(path))
        .header("content-type", "application/json")
        .header("authorization", admin_bearer(subject))
        .body(Body::from("{}"))
        .expect("request");
    app.clone().oneshot(request).await.expect("oneshot");
}

/// The stored records of `subjects`, as operation ids keyed by principal.
async fn records(pool: &PgPool, subjects: &[String]) -> BTreeMap<String, Vec<Option<String>>> {
    let rows: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT principal, operation FROM audit.audit_event WHERE principal = ANY($1)",
    )
    .bind(subjects)
    .fetch_all(pool)
    .await
    .expect("read the trail");
    let mut out: BTreeMap<String, Vec<Option<String>>> = BTreeMap::new();
    for (subject, operation) in rows {
        out.entry(subject).or_default().push(operation);
    }
    out
}

/// The records of `subjects` once the drain has settled: every probe has a
/// record, or no new record has landed for two seconds.
async fn settled(pool: &PgPool, subjects: &[String]) -> BTreeMap<String, Vec<Option<String>>> {
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut seen = 0;
    let mut quiet_since = Instant::now();
    loop {
        let stored = records(pool, subjects).await;
        let count: usize = stored.values().map(Vec::len).sum();
        if count != seen {
            seen = count;
            quiet_since = Instant::now();
        }
        if stored.len() == subjects.len()
            || quiet_since.elapsed() > Duration::from_secs(2)
            || Instant::now() > deadline
        {
            return stored;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// The exempt table names only mounted routes, and every other mounted route
/// writes exactly one access record carrying its operation id.
///
/// Each route is driven by its own principal, which the record carries, so a
/// record is attributed to the probe that caused it.
#[tokio::test]
async fn every_route_writes_exactly_one_access_record() {
    let cfg = config();
    let mounted = mounted(&cfg);
    let stale: Vec<String> = EXEMPT
        .iter()
        .filter(|(m, p, _)| !mounted.contains_key(&((*m).to_owned(), (*p).to_owned())))
        .map(|(m, p, _)| format!("{m} {p}"))
        .collect();
    assert!(
        stale.is_empty(),
        "exempt routes the router no longer mounts: {stale:#?}"
    );

    let (_db, pool, app) = app().await;
    let mut probes = Vec::new();
    for ((method, path), operation_id) in &mounted {
        if EXEMPT.iter().any(|(m, p, _)| m == method && p == path) {
            continue;
        }
        let expected = operation_id
            .clone()
            .unwrap_or_else(|| panic!("{method} {path} declares no operation id"));
        let subject = format!("route-probe-{}", probes.len());
        drive(&app, &subject, method, path).await;
        probes.push((format!("{method} {path}"), expected, subject));
    }
    assert!(!probes.is_empty(), "the router mounts audited routes");

    let subjects: Vec<String> = probes.iter().map(|(_, _, s)| s.clone()).collect();
    let stored = settled(&pool, &subjects).await;
    let mut defects = Vec::new();
    for (route, expected, subject) in &probes {
        let operations = stored.get(subject).cloned().unwrap_or_default();
        let matching = operations
            .iter()
            .filter(|op| op.as_deref() == Some(expected.as_str()))
            .count();
        if matching != 1 {
            defects.push(format!(
                "{route}: expected one `{expected}` record, found {operations:?}"
            ));
        }
    }
    assert!(
        defects.is_empty(),
        "routes without exactly one access record: {defects:#?}"
    );
}

/// The access record carries the correlation id the client received in
/// `x-request-id`, so a record can be matched to the response that caused it.
#[tokio::test]
async fn the_access_record_carries_the_request_id_the_client_received() {
    let (_db, pool, app) = app().await;
    let subject = "request-id-probe";
    let request = Request::builder()
        .method("GET")
        .uri(concrete(&format!("{}/ehr/{{ehr_id}}", common::BASE)))
        .header("authorization", admin_bearer(subject))
        .body(Body::empty())
        .expect("request");
    let response = app.oneshot(request).await.expect("oneshot");
    let sent = response
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::to_owned)
        .expect("the response carries x-request-id");

    let deadline = Instant::now() + Duration::from_secs(30);
    let stored: Option<String> = loop {
        let row: Option<(Option<String>,)> =
            sqlx::query_as("SELECT request_id FROM audit.audit_event WHERE principal = $1")
                .bind(subject)
                .fetch_optional(&pool)
                .await
                .expect("read the trail");
        if let Some((request_id,)) = row {
            break request_id;
        }
        assert!(Instant::now() < deadline, "no access record landed");
        tokio::time::sleep(Duration::from_millis(100)).await;
    };
    assert_eq!(stored.as_deref(), Some(sent.as_str()));
}
