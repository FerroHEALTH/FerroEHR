// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Integration tests for the management surface: the access-level matrix, the
//! Prometheus exposition + route-template label + cardinality guard,
//! separate-port isolation, and the split between the always-on public probes
//! (statuses only) and the authenticated health detail and status document.
//!
//! No openEHR spec governs the management surface — our own operational design.

#![expect(
    clippy::expect_used,
    clippy::string_slice,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::sync::{Arc, OnceLock};

use async_trait::async_trait;

use argon2::Argon2;
use argon2::password_hash::PasswordHasher;
use axum::Router;
use axum::body::Body;
use ferroehr::config::auth::{AuthConfig, BasicConfig, BasicUser};
use ferroehr::config::management::{AccessLevel, EndpointLevels, ManagementConfig};
use ferroehr::config::server::ServerConfig;
use ferroehr::telemetry::build_info::BuildInfo;
use ferroehr::telemetry::health::{Health, HealthIndicator, HealthRegistry};
use ferroehr_rest::config::AppConfig;
use ferroehr_rest::extensions::access::authz::AuthzHandle;
use ferroehr_rest::extensions::management::Observability;

use crate::common;
use http::{Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

/// Base64 of `admin:pw` for a Basic credential.
const ADMIN_BASIC: &str = "Basic YWRtaW46cHc=";

fn hash(pw: &str) -> String {
    Argon2::default()
        .hash_password_with_salt(pw.as_bytes(), b"1234567890123456")
        .expect("hash")
        .to_string()
}

fn auth_config(roles: &[&str]) -> AuthConfig {
    AuthConfig {
        enabled: true,
        basic: Some(BasicConfig {
            users: vec![BasicUser {
                username: "admin".to_owned(),
                password_hash: ferroehr::config::secret::Secret::new(hash("pw")),
                password_hash_file: None,
                roles: roles.iter().map(|r| (*r).to_owned()).collect(),
            }],
        }),
        oidc: None,
        ..AuthConfig::default()
    }
}

/// The base app configuration every management fixture shares: Swagger off,
/// authentication from `auth`.
fn base_config(auth: AuthConfig) -> AppConfig {
    AppConfig {
        server: ServerConfig {
            swagger_ui: AccessLevel::Off,
            ..Default::default()
        },
        auth,
        ..Default::default()
    }
}

/// The RBAC gate over the default rule set, with RBAC itself on or off and no
/// attribute resolution (the management surface has no subject or template).
fn authz_for(config: &AppConfig, rbac_enabled: bool) -> Option<Arc<AuthzHandle>> {
    let mut authz_cfg = ferroehr::config::authz::AuthzConfig::default();
    authz_cfg.rbac.enabled = rbac_enabled;
    AuthzHandle::build(
        &authz_cfg,
        &config.server.base_path,
        None,
        common::null_resolvers(),
    )
    .map(Arc::new)
}

/// Assemble the full app over a real service on a fresh database.
///
/// Returns the harness guard with the router: dropping it releases the template
/// clone the app is still serving from, so every caller binds it for the test.
async fn build_app(
    config: AppConfig,
    authz: Option<Arc<AuthzHandle>>,
    observability: Observability,
) -> (testkit::TestDb, Router) {
    let (pg, service) = common::test_service().await;
    (
        pg,
        ferroehr_rest::build_full(config, service, authz, observability).expect("build"),
    )
}

/// One management endpoint mounted at `level`, everything else at its default.
fn one_endpoint(endpoints: EndpointLevels) -> Observability {
    Observability {
        management: ManagementConfig {
            enabled: true,
            endpoints,
            ..ManagementConfig::default()
        },
        ..Observability::default()
    }
}

/// Build an app with the management surface enabled, one endpoint (`info`) set
/// to `level`, the Basic user granted `roles`, and RBAC on or off — the
/// `AdminOnly` level gates on the RBAC `admin_role` (issue #1879 retired the
/// deprecated `admin_scope` alias).
async fn app_with(
    level: AccessLevel,
    roles: &[&str],
    rbac_enabled: bool,
) -> (testkit::TestDb, Router) {
    let config = base_config(auth_config(roles));
    let authz = authz_for(&config, rbac_enabled);
    let observability = one_endpoint(EndpointLevels {
        info: level,
        ..EndpointLevels::default()
    });
    build_app(config, authz, observability).await
}

async fn status_of(app: Router, req: Request<Body>) -> StatusCode {
    app.oneshot(req).await.expect("response").status()
}

fn get(path: &str) -> Request<Body> {
    common::get(path)
}

fn get_auth(path: &str) -> Request<Body> {
    common::get_authorized(path, ADMIN_BASIC)
}

#[tokio::test]
async fn off_endpoint_is_404() {
    let (_db, app) = app_with(AccessLevel::Off, &["ADMIN"], true).await;
    assert_eq!(
        status_of(app, get("/management/info")).await,
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn public_endpoint_needs_no_auth() {
    let (_db, app) = app_with(AccessLevel::Public, &["ADMIN"], true).await;
    assert_eq!(
        status_of(app, get("/management/info")).await,
        StatusCode::OK
    );
}

/// `GET /management/info` names the manufacturer with its postal address, its
/// single point of contact and its website (Regulation (EU) 2025/327,
/// `docs/law/eu/ehds/text.html` Art. 30(1)(g)).
#[tokio::test]
async fn info_names_the_manufacturer() {
    let (_db, app) = app_with(AccessLevel::Public, &["ADMIN"], true).await;
    let response = app
        .oneshot(get("/management/info"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let body: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
    assert_eq!(
        body["manufacturer"],
        serde_json::json!({
            "name": "Cadasto B.V.",
            "postal_address": "Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands",
            "email": "info@cadasto.com",
            "website": "https://www.cadasto.com/contact/",
        })
    );
}

#[tokio::test]
async fn private_endpoint_401_then_200() {
    let (_db, app) = app_with(AccessLevel::Private, &["ADMIN"], true).await;
    assert_eq!(
        status_of(app.clone(), get("/management/info")).await,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        status_of(app, get_auth("/management/info")).await,
        StatusCode::OK
    );
}

#[tokio::test]
async fn admin_only_401_403_200() {
    // 401 unauthenticated.
    let (_db, app) = app_with(AccessLevel::AdminOnly, &["ADMIN"], true).await;
    assert_eq!(
        status_of(app, get("/management/info")).await,
        StatusCode::UNAUTHORIZED
    );
    // 403 authenticated but without the RBAC admin role.
    let (_db, app) = app_with(AccessLevel::AdminOnly, &["USER"], true).await;
    assert_eq!(
        status_of(app, get_auth("/management/info")).await,
        StatusCode::FORBIDDEN
    );
    // 200 with the admin role.
    let (_db, app) = app_with(AccessLevel::AdminOnly, &["ADMIN"], true).await;
    assert_eq!(
        status_of(app, get_auth("/management/info")).await,
        StatusCode::OK
    );
    // 200 with RBAC disabled: authenticated is enough (auth-only deployments).
    let (_db, app) = app_with(AccessLevel::AdminOnly, &["USER"], false).await;
    assert_eq!(
        status_of(app, get_auth("/management/info")).await,
        StatusCode::OK
    );
}

/// Build an app with FOUR endpoints at four different levels at once, so a
/// leak between them is observable.
async fn app_with_mixed_levels(roles: &[&str]) -> (testkit::TestDb, Router) {
    let config = base_config(auth_config(roles));
    let authz = authz_for(&config, true);
    let mut observability = one_endpoint(EndpointLevels {
        prometheus: AccessLevel::Public,
        info: AccessLevel::Private,
        env: AccessLevel::AdminOnly,
        metrics: AccessLevel::Off,
        ..EndpointLevels::default()
    });
    // The recorder is what MOUNTS prometheus/metrics at all; without it they
    // answer 404 whatever their level says, which would make the `public` row
    // below pass for the wrong reason and the `off` row vacuous.
    observability.prometheus = Some(recorder().clone());
    build_app(config, authz, observability).await
}

/// Each endpoint answers at ITS OWN level, and no endpoint's level leaks to a
/// neighbour.
///
/// This is the property an operator actually relies on when locking the surface
/// down, and it is not implied by the per-level tests above: those set one
/// endpoint at a time, so a guard that accidentally applied the most permissive
/// configured level to every route would pass all of them. There is no global
/// default to fall back to either — an endpoint left `Off` is simply not
/// mounted.
#[tokio::test]
async fn endpoint_levels_are_independent() {
    // Anonymous: only the `public` endpoint answers. The `admin_only` and
    // `private` ones challenge; the `off` one is not mounted at all, and says
    // 404 rather than 401 — its absence is not a credential problem.
    let (_db, app) = app_with_mixed_levels(&["ADMIN"]).await;
    assert_eq!(
        status_of(app, get("/management/prometheus")).await,
        StatusCode::OK
    );
    for (path, expected) in [
        ("/management/info", StatusCode::UNAUTHORIZED),
        ("/management/env", StatusCode::UNAUTHORIZED),
        ("/management/metrics", StatusCode::NOT_FOUND),
    ] {
        let (_db, app) = app_with_mixed_levels(&["ADMIN"]).await;
        assert_eq!(
            status_of(app, get(path)).await,
            expected,
            "anonymous {path}"
        );
    }

    // A non-admin principal: `private` admits it, `admin_only` refuses it, and
    // the refusal does not spill onto the endpoint next door.
    for (path, expected) in [
        ("/management/prometheus", StatusCode::OK),
        ("/management/info", StatusCode::OK),
        ("/management/env", StatusCode::FORBIDDEN),
        ("/management/metrics", StatusCode::NOT_FOUND),
    ] {
        let (_db, app) = app_with_mixed_levels(&["USER"]).await;
        assert_eq!(
            status_of(app, get_auth(path)).await,
            expected,
            "USER {path}"
        );
    }

    // The admin principal clears every mounted level; `off` stays absent.
    for (path, expected) in [
        ("/management/prometheus", StatusCode::OK),
        ("/management/info", StatusCode::OK),
        ("/management/env", StatusCode::OK),
        ("/management/metrics", StatusCode::NOT_FOUND),
    ] {
        let (_db, app) = app_with_mixed_levels(&["ADMIN"]).await;
        assert_eq!(
            status_of(app, get_auth(path)).await,
            expected,
            "ADMIN {path}"
        );
    }
}

/// With the `health` endpoint left off (the default), the management surface
/// routes no health path: with the surface enabled and `info` public, every
/// `/management/health*` path is a plain `404`, while the public health family
/// answers on the same app.
#[tokio::test]
async fn management_serves_no_health_route() {
    let (_db, app) = app_with(AccessLevel::Public, &["ADMIN"], true).await;
    for gone in [
        "/management/health",
        "/management/health/liveness",
        "/management/health/readiness",
    ] {
        assert_eq!(
            status_of(app.clone(), get(gone)).await,
            StatusCode::NOT_FOUND,
            "{gone} must no longer be routed"
        );
    }
    for public in ["/health", "/health/liveness", "/health/readiness"] {
        assert_eq!(
            status_of(app.clone(), get(public)).await,
            StatusCode::OK,
            "{public} must answer unauthenticated"
        );
    }
}

/// The public health family does not depend on the management surface at all:
/// with `management.enabled = false` (the default posture) the three probes
/// still answer, and `/management/info` is absent.
#[tokio::test]
async fn health_family_survives_management_disabled() {
    let config = base_config(auth_config(&["ADMIN"]));
    let (_db, app) = build_app(config, None, Observability::default()).await;

    for public in ["/health", "/health/liveness", "/health/readiness"] {
        assert_eq!(
            status_of(app.clone(), get(public)).await,
            StatusCode::OK,
            "{public} must answer with the management surface disabled"
        );
    }
    assert_eq!(
        status_of(app, get("/management/info")).await,
        StatusCode::NOT_FOUND
    );
}

/// A fixed indicator whose detail names exactly the kind of fact the public
/// probe must not disclose.
#[derive(Debug)]
struct Detailed;

/// The detail [`Detailed`] reports.
const INDICATOR_DETAIL: &str = "3 findings; chain broken at position 42 (10.0.0.5:5432)";

#[async_trait]
impl HealthIndicator for Detailed {
    fn name(&self) -> &'static str {
        "audit_chain"
    }

    async fn check(&self) -> Health {
        Health::degraded(INDICATOR_DETAIL)
    }

    fn required(&self) -> bool {
        false
    }
}

/// Builds an app with authentication and RBAC on, the [`Detailed`] indicator
/// registered, and the `health` and `status` endpoints at `level`.
async fn app_with_detail(level: AccessLevel, roles: &[&str]) -> (testkit::TestDb, Router) {
    let config = base_config(auth_config(roles));
    let authz = authz_for(&config, true);
    let mut observability = one_endpoint(EndpointLevels {
        health: level,
        status: level,
        ..EndpointLevels::default()
    });
    observability.health = HealthRegistry::new(vec![Arc::new(Detailed)]);
    build_app(config, authz, observability).await
}

/// Sends `req` and returns the status with the body parsed as JSON.
async fn json_of(app: Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
    let response = app.oneshot(req).await.expect("response");
    let status = response.status();
    let bytes = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    (status, serde_json::from_slice(&bytes).expect("json body"))
}

/// The public readiness body names each indicator and its status and nothing
/// more; the detail behind it is `GET /management/health`, which needs a
/// credential (`401`), then the admin role at `admin_only` (`403`), and then
/// serves every indicator with its detail. No openEHR spec governs health
/// probes — our own design.
#[tokio::test]
async fn readiness_is_status_only_and_management_health_carries_the_detail() {
    let (_db, app) = app_with_detail(AccessLevel::AdminOnly, &["ADMIN"]).await;
    let (status, body) = json_of(app.clone(), get("/health/readiness")).await;
    assert_eq!(status, StatusCode::OK, "DEGRADED still serves: {body}");
    assert_eq!(
        body,
        serde_json::json!({
            "status": "DEGRADED",
            "components": { "audit_chain": { "status": "DEGRADED" } }
        }),
        "the public probe carries statuses only"
    );
    assert!(
        !body.to_string().contains("detail"),
        "no detail field reaches the public probe: {body}"
    );

    assert_eq!(
        status_of(app.clone(), get("/management/health")).await,
        StatusCode::UNAUTHORIZED
    );
    let (status, body) = json_of(app, get_auth("/management/health")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(body["status"], "DEGRADED");
    assert_eq!(
        body["components"]["audit_chain"],
        serde_json::json!({ "status": "DEGRADED", "detail": INDICATOR_DETAIL }),
        "the management view carries the detail: {body}"
    );

    let (_db, app) = app_with_detail(AccessLevel::AdminOnly, &["USER"]).await;
    assert_eq!(
        status_of(app, get_auth("/management/health")).await,
        StatusCode::FORBIDDEN
    );
}

/// The licence, the deployment gaps and the support period are served by
/// `GET /management/status` behind its access level and are absent from the
/// public `GET /ferroehr/rest/status`, which keeps only the profile of the
/// deployment block (CRA support statement: `docs/law/eu/cra/text.html`
/// Art. 13(19); the rest is our own design).
#[tokio::test]
async fn management_status_carries_what_the_public_status_withholds() {
    let (_db, app) = app_with_detail(AccessLevel::Private, &["USER"]).await;

    let (status, public) = json_of(app.clone(), get("/ferroehr/rest/status")).await;
    assert_eq!(status, StatusCode::OK);
    for withheld in ["licence", "support"] {
        assert!(
            public.get(withheld).is_none(),
            "{withheld} must not be public: {public}"
        );
    }
    assert_eq!(
        public["deployment"],
        serde_json::json!({ "profile": "sandbox" })
    );

    assert_eq!(
        status_of(app.clone(), get("/management/status")).await,
        StatusCode::UNAUTHORIZED
    );
    let (status, full) = json_of(app, get_auth("/management/status")).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(full["status"], "UP");
    assert_eq!(full["server_version"], public["server_version"]);
    assert!(full["licence"].get("state").is_some(), "licence: {full}");
    assert_eq!(full["deployment"]["profile"], "sandbox");
    assert!(full["deployment"]["gaps"].is_array(), "gaps: {full}");
    assert!(
        full["deployment"]["accepted"].is_array(),
        "accepted: {full}"
    );
    let support =
        ferroehr::support::SupportPeriod::current().report_on(ferroehr::support::today_utc());
    assert_eq!(
        full["support"],
        serde_json::to_value(&support).expect("support report"),
        "the management status states the support period"
    );
}

/// A process-wide Prometheus recorder (installed once; the global `metrics`
/// facade the HTTP layer emits through requires a single global recorder).
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

async fn app_with_metrics() -> (testkit::TestDb, Router) {
    // Authentication off: this test exercises the API path, not the gate.
    let config = base_config(AuthConfig {
        enabled: false,
        ..AuthConfig::default()
    });
    let observability = Observability {
        management: ManagementConfig {
            enabled: true,
            endpoints: EndpointLevels {
                prometheus: AccessLevel::Public,
                metrics: AccessLevel::Public,
                ..EndpointLevels::default()
            },
            ..ManagementConfig::default()
        },
        prometheus: Some(recorder().clone()),
        health: HealthRegistry::default(),
        build_info: BuildInfo::current(),
        ..Observability::default()
    };
    build_app(config, None, observability).await
}

#[tokio::test]
async fn prometheus_has_route_template_label_and_no_ids() {
    let (_db, app) = app_with_metrics().await;

    // Drive an API request that matches a templated route carrying an id. The
    // HTTP metrics layer must label it by the *template*, never the raw id.
    let ehr_id = "3fa85f64-5717-4562-b3fc-2c963f66afa6";
    let _response = app
        .clone()
        .oneshot(get(&format!(
            "/ferroehr/rest/openehr/v1/ehr/{ehr_id}/composition/x"
        )))
        .await
        .expect("api response");

    // Scrape the exposition text.
    let resp = app
        .oneshot(get("/management/prometheus"))
        .await
        .expect("prometheus response");
    assert_eq!(resp.status(), StatusCode::OK);
    let body = resp.into_body().collect().await.expect("body").to_bytes();
    let text = String::from_utf8(body.to_vec()).expect("utf8");

    assert!(
        text.contains("http_server_request_duration_seconds"),
        "missing HTTP duration metric:\n{text}"
    );
    assert!(
        text.contains("http_route="),
        "missing route-template label:\n{text}"
    );
    // Cardinality guard: no label VALUE may look like a UUID or a bare number id.
    for line in text.lines().filter(|l| !l.starts_with('#')) {
        for value in label_values(line) {
            assert!(
                !looks_like_id(value),
                "id-shaped label value {value:?} leaked into metric line: {line}"
            );
        }
    }
}

/// Extract the label values (`k="v"`) from an exposition sample line.
fn label_values(line: &str) -> Vec<&str> {
    let Some(open) = line.find('{') else {
        return Vec::new();
    };
    let Some(close) = line.rfind('}') else {
        return Vec::new();
    };
    line[open + 1..close]
        .split(',')
        .filter_map(|pair| pair.split_once('='))
        .map(|(_, v)| v.trim().trim_matches('"'))
        .collect()
}

/// Whether a label value looks like a UUID or a long bare numeric id (the
/// cardinality budget forbids these).
fn looks_like_id(value: &str) -> bool {
    let uuid_shaped =
        value.len() == 36 && value.split('-').map(str::len).collect::<Vec<_>>() == [8, 4, 4, 4, 12];
    let long_number = value.len() >= 6 && value.chars().all(|c| c.is_ascii_digit());
    uuid_shaped || long_number
}

#[tokio::test]
async fn separate_port_mode_keeps_management_off_the_main_app() {
    // With `management.port` set, the main app must NOT mount /management…
    let config = base_config(AuthConfig {
        enabled: false,
        ..AuthConfig::default()
    });
    let observability = Observability {
        management: ManagementConfig {
            enabled: true,
            port: Some(9099),
            endpoints: EndpointLevels {
                info: AccessLevel::Public,
                ..EndpointLevels::default()
            },
            ..ManagementConfig::default()
        },
        ..Observability::default()
    };
    let (_db, main_app) = build_app(config, None, observability).await;

    // …the main app 404s the management route.
    assert_eq!(
        status_of(main_app.clone(), get("/management/info")).await,
        StatusCode::NOT_FOUND
    );
    // …but the health family is a sibling of the API tree, not part of the
    // management surface, so it stays on the MAIN port (which is what the
    // orchestrator probes point at).
    for public in ["/health", "/health/liveness", "/health/readiness"] {
        assert_eq!(
            status_of(main_app.clone(), get(public)).await,
            StatusCode::OK,
            "{public} must stay on the main listener in separate-port mode"
        );
    }
}

// ── The on-demand CPU flamegraph (`/management/flamegraph`) ─────────────────

/// Build an app with the management surface enabled and only the `flamegraph`
/// endpoint mounted, at `level`, with the given profiling limits.
async fn app_with_flamegraph(
    level: AccessLevel,
    profiling: ferroehr::config::management::ProfilingConfig,
) -> (testkit::TestDb, Router) {
    let config = base_config(auth_config(&["ADMIN"]));
    let observability = Observability {
        management: ManagementConfig {
            enabled: true,
            endpoints: EndpointLevels {
                flamegraph: level,
                ..EndpointLevels::default()
            },
            profiling,
            ..ManagementConfig::default()
        },
        ..Observability::default()
    };
    build_app(config, None, observability).await
}

/// Off (the default) means the route is simply absent — a `404` answered
/// before authentication.
#[tokio::test]
async fn flamegraph_off_is_404() {
    let (_db, app) = app_with_flamegraph(
        AccessLevel::Off,
        ferroehr::config::management::ProfilingConfig::default(),
    )
    .await;
    assert_eq!(
        status_of(app, get("/management/flamegraph")).await,
        StatusCode::NOT_FOUND
    );
}

/// Opted in: a short sample window answers `200` with a rendered SVG.
#[tokio::test]
async fn flamegraph_samples_and_renders_svg() {
    let (_db, app) = app_with_flamegraph(
        AccessLevel::Public,
        ferroehr::config::management::ProfilingConfig::default(),
    )
    .await;
    let response = app
        .oneshot(get("/management/flamegraph?seconds=1&frequency=99"))
        .await
        .expect("response");
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        response
            .headers()
            .get(header::CONTENT_TYPE)
            .expect("content type"),
        "image/svg+xml"
    );
    let body = response
        .into_body()
        .collect()
        .await
        .expect("body")
        .to_bytes();
    let text = String::from_utf8(body.to_vec()).expect("svg is utf-8");
    assert!(
        text.contains("<svg"),
        "the body must be a rendered flamegraph SVG, got ({} bytes): {}",
        text.len(),
        text.get(..400).unwrap_or(&text)
    );
}

/// A request beyond a configured cap is refused with `400` — never silently
/// clamped.
#[tokio::test]
async fn flamegraph_over_cap_is_400() {
    let (_db, app) = app_with_flamegraph(
        AccessLevel::Public,
        ferroehr::config::management::ProfilingConfig::default(),
    )
    .await;
    assert_eq!(
        status_of(app.clone(), get("/management/flamegraph?seconds=31")).await,
        StatusCode::BAD_REQUEST,
        "seconds beyond management.profiling.max_seconds must refuse"
    );
    assert_eq!(
        status_of(app, get("/management/flamegraph?frequency=1000")).await,
        StatusCode::BAD_REQUEST,
        "frequency beyond management.profiling.max_frequency must refuse"
    );
}

/// The management surface sits inside the audit trail (#3244): a request to
/// `/management/info` leaves a system-domain access record under its own
/// operation id, so a runtime filter change or an env read is never an
/// unrecorded administrative act.
#[tokio::test]
async fn a_management_request_is_recorded_in_the_audit_trail() {
    use ferroehr::system_log::config::{AuditConfig, StoreConfig};
    use ferroehr::system_log::sender::{AuditHandle, start};

    let (pg, pool) = common::migrated_pool().await;
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
    let (sender, _handle): (_, AuditHandle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let service = Arc::new(
        ferroehr::service::FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(
            &pool,
        ))
        .with_audit(sender),
    );
    let config = base_config(auth_config(&["ADMIN"]));
    let authz = authz_for(&config, true);
    let observability = one_endpoint(EndpointLevels {
        info: AccessLevel::AdminOnly,
        ..EndpointLevels::default()
    });
    let app = ferroehr_rest::build_full(config, service, authz, observability).expect("build");
    assert_eq!(
        status_of(app, get_auth("/management/info")).await,
        StatusCode::OK
    );

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let row = loop {
        let found: Option<(String, Option<String>, String)> = sqlx::query_as(
            "SELECT operation, principal, domain FROM audit.audit_event \
             WHERE operation = 'management_info' ORDER BY recorded_at DESC LIMIT 1",
        )
        .fetch_optional(&pool)
        .await
        .expect("read the trail");
        if let Some(row) = found {
            break row;
        }
        if std::time::Instant::now() >= deadline {
            let landed: Vec<Option<String>> =
                sqlx::query_scalar("SELECT operation FROM audit.audit_event ORDER BY recorded_at")
                    .fetch_all(&pool)
                    .await
                    .expect("read the trail");
            panic!("no management_info record landed; the trail holds {landed:?}");
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    };
    assert_eq!(row.0, "management_info");
    assert!(row.1.is_some(), "the admin who read it is named");
    assert_eq!(
        row.2, "system",
        "a management read touches no pseudonymisation domain"
    );
    drop(pg);
}
