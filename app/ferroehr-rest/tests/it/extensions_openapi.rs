// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Tests for the generated extension-surface `OpenAPI` document
//! ([`extensions_document`]) and its consistency with the live router.
//!
//! No openEHR spec governs the extension surface — our own operational +
//! extension design. These tests assert (1) the document is non-empty, (2)
//! every documented path actually routes on a fully-enabled server (a `404`
//! would mean the path is documented but not mounted; an auth `401` never
//! occurs here because auth is disabled), and (3) a set of representative
//! extension paths are present so the document cannot silently shrink.

#![expect(
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::string_slice,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::{Arc, OnceLock};

use axum::Router;
use axum::body::Body;
use http::{Request, StatusCode};
use http_body_util::BodyExt;
use serde_json::Value;
use tower::ServiceExt;

use ferroehr::config::auth::AuthConfig;
use ferroehr::config::management::{AccessLevel, EndpointLevels, ManagementConfig};
use ferroehr::config::server::{AdminConfig, ServerConfig};
use ferroehr::config::smart::SmartConfig;
use ferroehr::telemetry::build_info::BuildInfo;
use ferroehr::telemetry::health::HealthRegistry;
use ferroehr::telemetry::log_reload::LogReload;
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::management::Observability;
use ferroehr_rest::extensions::openapi::extensions_document;

use crate::common;

const BASE: &str = "/ferroehr/rest/openehr/v1";

/// The known HTTP methods a documented path item may carry (everything else on
/// a path-item object — `parameters`, `summary`, … — is metadata, not an op).
const HTTP_METHODS: &[&str] = &[
    "get", "put", "post", "delete", "patch", "head", "options", "trace",
];

/// The spec-selector family slugs offered by `FAMILIES` in
/// `extensions::openapi` (private there; the selector URLs are the stable
/// public contract).
const FAMILY_SLUGS: &[&str] = &[
    "ehr",
    "query",
    "definition",
    "demographic",
    "admin",
    "management",
    "terminology",
    "relationships",
    "messaging",
    "events",
    "fhir",
    "smart",
];

// ── Test 1: the document is non-empty and covers the extension surface ────────

#[test]
fn extensions_doc_is_non_empty() {
    let doc = serde_json::to_value(extensions_document(&app_config())).expect("serialise doc");
    let paths = doc["paths"].as_object().expect("paths object");
    assert!(!paths.is_empty(), "the extension document has no paths");

    // A floor on coverage so the document cannot silently shrink below the
    // surface enumerated from the router.
    let op_count: usize = paths
        .values()
        .map(|item| {
            item.as_object()
                .map_or(0, |o| o.keys().filter(|k| is_method(k)).count())
        })
        .sum();
    assert!(
        op_count >= 40,
        "expected the extension surface to document >= 40 operations, found {op_count}"
    );

    // Representative paths from each documented extension group must be present.
    for expected in [
        "/ferroehr/rest/status",
        // The always-on public health family (never config-gated).
        "/health",
        "/health/liveness",
        "/health/readiness",
        "/management/info",
        "/management/health",
        "/management/status",
        "/ferroehr/rest/.well-known/smart-configuration",
        "/ferroehr/rest/api-docs/openapi.json",
        "/ferroehr/rest/openehr/v1/terminology",
        "/ferroehr/rest/openehr/v1/demographic/party_relationship",
        "/ferroehr/rest/openehr/v1/admin/event_subscription",
        "/ferroehr/rest/openehr/v1/fhir/r4/{resource_type}",
    ] {
        assert!(
            paths.contains_key(expected),
            "extension document is missing the path {expected}"
        );
    }
}

// ── Test 1b: a non-default base path moves every documented path ──────────────

/// The served document must describe the paths THIS deployment serves. A
/// `#[utoipa::path]` literal can only spell the default deployment, so the
/// endpoints that hang off the REST root rather than the API base path —
/// `/status`, the three OAS meta-endpoint paths, the SMART discovery document —
/// and the System `OPTIONS` manifest at the base-path root are all re-homed
/// from the same configuration the live router mounts from. A non-default
/// `server.base_path` therefore moves the document with the routes.
#[test]
fn a_non_default_base_path_moves_the_documented_paths() {
    const BASE: &str = "/gateway/v1/openehr/v1";
    const ROOT: &str = "/gateway/v1";

    let mut cfg = app_config();
    cfg.server.base_path = BASE.to_owned();
    let doc = serde_json::to_value(extensions_document(&cfg)).expect("serialise doc");
    let paths = doc["paths"].as_object().expect("paths object");

    for expected in [
        // The endpoints whose declaration literal is the default spelling.
        format!("{ROOT}/status"),
        format!("{ROOT}/api-docs/openapi.json"),
        format!("{ROOT}/api-docs/ferroehr-{{family}}.openapi.json"),
        format!("{ROOT}/swagger-ui"),
        format!("{ROOT}/.well-known/smart-configuration"),
        // The System Options manifest sits at the API base-path root itself.
        BASE.to_owned(),
        // …and the nested API groups follow the base path as they always did.
        format!("{BASE}/ehr"),
        format!("{BASE}/terminology"),
    ] {
        assert!(
            paths.contains_key(&expected),
            "a non-default base path must move the documented path to {expected}; \
             document has: {:?}",
            paths.keys().collect::<Vec<_>>()
        );
    }

    // …and nothing may still be documented at the DEFAULT deployment paths.
    for stale in [
        "/ferroehr/rest/status",
        "/ferroehr/rest/api-docs/openapi.json",
        "/ferroehr/rest/api-docs/ferroehr-{family}.openapi.json",
        "/ferroehr/rest/swagger-ui",
        "/ferroehr/rest/.well-known/smart-configuration",
        "/ferroehr/rest/openehr/v1",
    ] {
        assert!(
            !paths.contains_key(stale),
            "{stale} is the DEFAULT spelling and must not survive a non-default base path"
        );
    }

    // The process-root health family is base-path-independent by design.
    for always in ["/health", "/health/liveness", "/health/readiness"] {
        assert!(
            paths.contains_key(always),
            "the health family is mounted at the process root and must not move"
        );
    }
}

// ── Test 2: every documented path routes on a fully-enabled server ────────────

#[tokio::test]
async fn every_documented_path_routes() {
    let app = full_app().await;

    // Warm the HTTP metrics so `/management/metrics/{name}` has a real metric to
    // resolve (an unknown metric legitimately 404s — that would be a false
    // negative for a routing check).
    let _response = app
        .clone()
        .oneshot(get(&format!(
            "{BASE}/ehr/00000000-0000-0000-0000-000000000000"
        )))
        .await
        .expect("warmup response");
    let metric_name = first_metric_name(&app).await;

    let doc = serde_json::to_value(extensions_document(&app_config())).expect("serialise doc");
    let paths = doc["paths"].as_object().expect("paths object");

    let mut checked = 0usize;
    for (template, item) in paths {
        let item = item.as_object().expect("path item object");
        for method in item.keys().filter(|k| is_method(k)) {
            let concrete = concretize(template, &metric_name);
            let req = Request::builder()
                .method(method.to_uppercase().as_str())
                .uri(&concrete)
                .body(Body::empty())
                .expect("request");
            let resp = app.clone().oneshot(req).await.expect("response");
            let status = resp.status();
            let body = resp.into_body().collect().await.expect("body").to_bytes();
            // A documented op must be *mounted*. Under the real service a mounted
            // handler may legitimately answer 404 for a missing resource (e.g. a
            // stored query that does not exist) — that still means the route is
            // mounted (the Mock backend previously masked this by answering 501
            // everywhere). An *unmounted* path is axum's bare fallback 404 with an
            // empty body; a mounted handler's 404 carries the openEHR
            // `{error,message}` body. So "not mounted" = a 404 with an empty body.
            assert!(
                status != StatusCode::NOT_FOUND || !body.is_empty(),
                "documented op {} {template} (driven as {concrete}) is not mounted (bare 404)",
                method.to_uppercase()
            );
            checked += 1;
        }
    }
    assert!(checked >= 40, "drove only {checked} documented operations");
}

/// The Swagger UI mount path must land somewhere its RELATIVE asset URLs
/// resolve, and must not loop doing it.
///
/// The dist `index.html` references its bundle relatively (`./swagger-ui.css`,
/// `./swagger-ui-bundle.js`). Served at the slash-less mount path, a browser
/// resolves those against the PARENT directory — every one `404`s and the page
/// renders empty with nothing a user can see. So the assertion is not "the mount
/// path answers 200": it is that following the mount path reaches a document
/// whose relative references resolve to routes that exist. The redirect target
/// also may not be the trailing-slash form, which `NormalizePathLayer` strips
/// straight back here.
#[tokio::test]
async fn the_swagger_mount_path_lands_where_its_relative_assets_resolve() {
    let app = full_app().await;
    let mount = "/ferroehr/rest/swagger-ui";

    let resp = app.clone().oneshot(get(mount)).await.expect("response");
    assert_eq!(
        resp.status(),
        StatusCode::TEMPORARY_REDIRECT,
        "the mount path must redirect rather than serve a body whose assets cannot resolve"
    );
    let location = resp
        .headers()
        .get(http::header::LOCATION)
        .expect("a redirect must carry Location")
        .to_str()
        .expect("ascii Location")
        .to_owned();
    assert_eq!(location, format!("{mount}/index.html"));
    assert!(
        !location.ends_with('/'),
        "a trailing-slash target is stripped by NormalizePathLayer back to {mount} — an \
         infinite loop, which is the bug this replaced: {location}"
    );

    // Following it once must terminate in the document itself, never another
    // redirect.
    let page = app.clone().oneshot(get(&location)).await.expect("response");
    assert_eq!(
        page.status(),
        StatusCode::OK,
        "{location} must serve the UI"
    );
    let html = page.into_body().collect().await.expect("body").to_bytes();
    let html = String::from_utf8_lossy(&html).into_owned();
    assert!(html.contains("swagger-ui"), "not the Swagger UI: {html}");

    // Every relative reference in that document, resolved against the directory
    // the redirect target sits in, must be a route that exists. This is the
    // assertion that actually catches the blank page.
    let dir = location
        .rsplit_once('/')
        .map(|(head, _)| head)
        .expect("the target has a directory");
    let mut resolved = 0usize;
    for reference in relative_references(&html) {
        let asset = format!("{dir}/{}", reference.trim_start_matches("./"));
        let resp = app.clone().oneshot(get(&asset)).await.expect("response");
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "the UI references {reference}, which resolves to {asset} and does not serve"
        );
        resolved += 1;
    }
    assert!(
        resolved >= 4,
        "expected the UI to reference its CSS and JS bundle; found {resolved} relative references"
    );
}

/// Every `href`/`src` in `html` that is relative (not absolute, not a URL).
fn relative_references(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for attribute in ["href=\"", "src=\""] {
        let mut rest = html;
        while let Some(at) = rest.find(attribute) {
            rest = &rest[at + attribute.len()..];
            let Some(end) = rest.find('"') else { break };
            let value = &rest[..end];
            rest = &rest[end..];
            let absolute = value.starts_with('/') || value.contains("://");
            if !absolute && !value.is_empty() && !value.starts_with('#') {
                out.push(value.to_owned());
            }
        }
    }
    out
}

// ── Test 3: every spec-selector family document is non-empty ──────────────────

/// Every family the Swagger spec-selector offers must serve a non-empty
/// document — the path-prefix criterion (standard ITS-REST groups) and the
/// tag criterion (server extensions) must each match at least one operation on
/// a fully-enabled server.
#[tokio::test]
async fn every_family_document_is_non_empty() {
    let app = full_app().await;
    for slug in FAMILY_SLUGS {
        let uri = format!("/ferroehr/rest/api-docs/ferroehr-{slug}.openapi.json");
        let resp = app
            .clone()
            .oneshot(get(&uri))
            .await
            .expect("family response");
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "family {slug} must serve ({uri})"
        );
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let v: Value = serde_json::from_slice(&bytes).expect("family json");
        let paths = v["paths"].as_object().expect("paths object");
        assert!(
            !paths.is_empty(),
            "family {slug} document has no paths (its selector criterion matched nothing)"
        );
    }
}

// ── Test 3b: the family documents COVER the complete document ─────────────────

/// The spec selector must be able to reach every operation the server serves:
/// each operation of the complete composed document appears in at least one
/// family document. A tag or path root that belongs to no family is a hole —
/// the operation would be visible only in the complete document, which is not
/// what a reader browsing the selector sees.
#[tokio::test]
async fn every_operation_appears_in_a_family_document() {
    let app = full_app().await;

    let mut covered: std::collections::BTreeSet<(String, String)> =
        std::collections::BTreeSet::new();
    for slug in FAMILY_SLUGS {
        let uri = format!("/ferroehr/rest/api-docs/ferroehr-{slug}.openapi.json");
        let resp = app
            .clone()
            .oneshot(get(&uri))
            .await
            .expect("family response");
        assert_eq!(resp.status(), StatusCode::OK, "family {slug} must serve");
        let bytes = resp.into_body().collect().await.expect("body").to_bytes();
        let v: Value = serde_json::from_slice(&bytes).expect("family json");
        for (path, method) in document_operations(&v) {
            covered.insert((path, method));
        }
    }

    let full = serde_json::to_value(extensions_document(&app_config())).expect("serialise doc");
    let missing: Vec<String> = document_operations(&full)
        .into_iter()
        .filter(|op| !covered.contains(op))
        .map(|(path, method)| format!("{} {path}", method.to_uppercase()))
        .collect();
    assert!(
        missing.is_empty(),
        "these served operations belong to no spec-selector family document:\n{}",
        missing.join("\n")
    );
}

/// Every `(path, method)` pair of an `OpenAPI` document value.
fn document_operations(doc: &Value) -> Vec<(String, String)> {
    let mut out = Vec::new();
    if let Some(paths) = doc["paths"].as_object() {
        for (path, item) in paths {
            if let Some(item) = item.as_object() {
                for method in item.keys().filter(|k| is_method(k)) {
                    out.push((path.clone(), method.clone()));
                }
            }
        }
    }
    out
}

// ── Test 4: the once-built served document equals a fresh extensions_document ─

/// the `OpenAPI` document is built once at router assembly and served as
/// pre-serialized bytes. The served bytes must be byte-for-byte the same
/// document `extensions_document(cfg)` produces fresh — the optimization is a
/// pure serving-mechanics change, no content change.
#[tokio::test]
async fn served_openapi_json_equals_fresh_document() {
    let app = full_app().await;
    let resp = app
        .clone()
        .oneshot(get("/ferroehr/rest/api-docs/openapi.json"))
        .await
        .expect("openapi.json response");
    assert_eq!(resp.status(), StatusCode::OK);
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let served: Value = serde_json::from_slice(&bytes).expect("served openapi json");

    let fresh = serde_json::to_value(extensions_document(&app_config()))
        .expect("fresh extensions_document");
    assert_eq!(
        served, fresh,
        "the once-built served OpenAPI document must equal a fresh extensions_document(cfg)"
    );
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn is_method(key: &str) -> bool {
    HTTP_METHODS.contains(&key)
}

/// Substitute `{param}` template segments with a value that routes: the real
/// metric name for `{name}` (so `/management/metrics/{name}` resolves), a real
/// family slug for `{family}` (the per-family OAS documents are twelve static
/// routes, so only a known slug resolves), any non-empty token otherwise
/// (malformed uuids yield `400`, never a routing `404`).
fn concretize(template: &str, metric_name: &str) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let close = rest[open..].find('}').expect("closing brace") + open;
        let param = &rest[open + 1..close];
        out.push_str(match param {
            "name" => metric_name,
            "family" => FAMILY_SLUGS[0],
            _ => "x",
        });
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

fn get(uri: &str) -> Request<Body> {
    Request::builder()
        .method("GET")
        .uri(uri)
        .body(Body::empty())
        .expect("request")
}

/// The first metric name the management list endpoint reports (after warmup).
async fn first_metric_name(app: &Router) -> String {
    let resp = app
        .clone()
        .oneshot(get("/management/metrics"))
        .await
        .expect("metrics list response");
    assert_eq!(
        resp.status(),
        StatusCode::OK,
        "metrics list must be mounted"
    );
    let bytes = resp.into_body().collect().await.expect("body").to_bytes();
    let v: Value = serde_json::from_slice(&bytes).expect("metrics json");
    let names = v["names"].as_array().expect("names array");
    assert!(!names.is_empty(), "no metrics available after warmup");
    names[0].as_str().expect("metric name").to_owned()
}

/// The global Prometheus recorder (install-once per test process).
fn recorder() -> &'static prometheus::Registry {
    static REGISTRY: OnceLock<prometheus::Registry> = OnceLock::new();
    REGISTRY.get_or_init(|| {
        // A real provider, not a bare registry: instruments bind to the global
        // meter provider, so a registry with nothing attached renders empty.
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

/// A no-op reloadable-filter handle so `/management/loggers` mounts.
fn log_reload() -> LogReload {
    LogReload::new(
        "info",
        Arc::new(|| "info".to_owned()),
        Arc::new(|_f: &str| Ok(())),
    )
}

/// The `AppConfig` used by both the served document and [`full_app`], so the
/// documented paths and the mounted routes are derived from one config (same
/// base path). Auth off + every surface enabled, so every documented path is
/// mounted.
fn app_config() -> AppConfig {
    AppConfig {
        server: ServerConfig {
            swagger_ui: AccessLevel::Public,
            ..Default::default()
        },
        auth: AuthConfig {
            enabled: false,
            ..AuthConfig::default()
        },
        admin: AdminConfig { enabled: true },
        smart: SmartConfig {
            enabled: true,
            ..SmartConfig::default()
        },
        fhir_api_enabled: true,
        terminology_api_enabled: true,
        events_admin_api: true,
        audit_organization_claim: None,
        spec_profile: ferroehr::config::profile::SpecProfile::default(),
    }
}

/// A server with auth off and every extension surface enabled, so every
/// documented path is mounted.
async fn full_app() -> Router {
    let config = app_config();
    let public = EndpointLevels {
        info: AccessLevel::Public,
        metrics: AccessLevel::Public,
        prometheus: AccessLevel::Public,
        env: AccessLevel::Public,
        loggers: AccessLevel::Public,
        flamegraph: AccessLevel::Public,
        health: AccessLevel::Public,
        status: AccessLevel::Public,
    };
    let observability = Observability {
        management: ManagementConfig {
            enabled: true,
            endpoints: public,
            // The routing parity test drives every documented op with default
            // parameters; cap the sample window at 1 s so the flamegraph op
            // answers quickly (the default window shrinks to the cap).
            profiling: ferroehr::config::management::ProfilingConfig {
                max_seconds: 1,
                ..ferroehr::config::management::ProfilingConfig::default()
            },
            ..ManagementConfig::default()
        },
        prometheus: Some(recorder().clone()),
        log_reload: Some(log_reload()),
        health: HealthRegistry::default(),
        build_info: BuildInfo::current(),
        ..Observability::default()
    };
    let (_pg, service) = common::test_service().await;
    ferroehr_rest::build_full(config, service, None, observability).expect("build")
}
