// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The management surface: **ops introspection** — info, Prometheus, metrics,
//! env, loggers, the on-demand CPU flamegraph, the per-indicator health detail
//! and the full status document.
//!
//! No openEHR spec governs this — our own operational surface. The orchestrator
//! probes do not live here: `/health` and its siblings are always-on and public
//! ([`crate::extensions::health`]) because they must not depend on an operator
//! remembering to enable an introspection surface. What a probe or an
//! anonymous client may learn ends at a status: the detail behind readiness
//! (`/management/health`) and the licence, deployment gaps and support period
//! (`/management/status`) live here with the redacted effective config, the
//! live log-filter control and the metric views, all sensitive and all off by
//! default.
//!
//! Every endpoint is off by default, opt-in via [`ManagementConfig`], gated by
//! its own access-level layer and optionally served from a separate internal
//! port: observability must never widen the clinical API's attack surface.
//!
//! Every operation is documented unconditionally in the served `OpenAPI` (see
//! [`openapi`]), but the live [`router`] mounts only the opted-in endpoints, so
//! a disabled one is absent and answers `404`. The `AccessGuard` layer yields
//! the documented `401`/`403`: `401` when the level is Private or `AdminOnly`
//! and the caller is unauthenticated, `403` when the level is `AdminOnly` and
//! the caller lacks `authz.rbac.admin_role`.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

mod env;
pub mod flamegraph;
pub mod http_metrics;
mod info_routes;
mod logger_routes;
mod metrics;

use ferroehr::config::authz::RbacConfig;
use ferroehr::config::deployment::DeploymentPosture;
use ferroehr::config::management::{AccessLevel, ManagementConfig};
use ferroehr::licence::state::LicenceStatus;
use ferroehr::telemetry::build_info::BuildInfo;
use ferroehr::telemetry::health::{AggregateHealth, HealthRegistry};
use ferroehr::telemetry::log_reload::LogReload;

use std::sync::Arc;

use axum::Router;
use axum::extract::{Path, Query, State};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Json, Response};
use axum::routing::get;
use http::{HeaderMap, StatusCode, header};
use serde_json::Value;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::extensions::access::authn::Authenticator;
use crate::extensions::access::authz::classify::OperationClass;
use crate::extensions::access::authz::roles::RbacDecision;
use crate::overview::error::RestError;
use crate::overview::status::FullStatus;
use crate::state::AppState;
use openehr_its::rest::runtime::ApiError;

/// Everything the management router needs. Assembled by the binary (which owns
/// the telemetry handles, the health indicators, and the effective config) and
/// handed to [`router`]. Cheap to clone.
#[derive(Clone)]
pub struct ManagementState {
    /// The management configuration (which endpoints, what access levels).
    pub config: ManagementConfig,
    /// The authentication layer, reused for access gating.
    pub authenticator: Arc<Authenticator>,
    /// The RBAC configuration the `AdminOnly` access level gates on.
    pub rbac: RbacConfig,
    /// The Prometheus render handle (present iff the recorder is installed).
    pub prometheus: Option<::prometheus::Registry>,
    /// The runtime log-filter control (present iff the reloadable filter is set).
    pub log_reload: Option<LogReload>,
    /// Build/spec provenance for `/info` and the build-info gauge.
    pub build_info: BuildInfo,
    /// The effective configuration snapshot for `/env` (redacted at render).
    pub env_snapshot: Arc<Value>,
    /// The one process-wide profiling permit (`/flamegraph` concurrency guard).
    pub profiler: flamegraph::ProfilerSlot,
    /// The health-indicator registry `/health` evaluates with every detail.
    pub health: HealthRegistry,
    /// The licence in force, as `/status` reports it.
    pub licence: LicenceStatus,
    /// The evaluated deployment posture, as `/status` reports it.
    pub deployment: DeploymentPosture,
}

impl std::fmt::Debug for ManagementState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagementState")
            .field("config", &self.config)
            .field("prometheus", &self.prometheus.is_some())
            .field("log_reload", &self.log_reload.is_some())
            .finish_non_exhaustive()
    }
}

impl ManagementState {
    /// Assembles the management state from the application state and the
    /// shared authenticator.
    ///
    /// The observability bundle the binary built supplies the configuration,
    /// the telemetry handles and the health registry; the service supplies the
    /// licence and the deployment posture, both fixed at boot.
    #[must_use]
    pub fn from_app_state(
        state: &AppState,
        authenticator: Arc<Authenticator>,
        rbac: RbacConfig,
    ) -> Self {
        let obs = state.observability().clone();
        Self {
            config: obs.management,
            authenticator,
            rbac,
            prometheus: obs.prometheus,
            log_reload: obs.log_reload,
            build_info: obs.build_info,
            env_snapshot: obs.env_snapshot,
            profiler: flamegraph::ProfilerSlot::default(),
            health: obs.health,
            licence: state.backend().licence().status(),
            deployment: state.backend().deployment().clone(),
        }
    }
}

/// The observability inputs the binary assembles for the application state.
///
/// Carried in [`AppState`]: the management
/// configuration, the telemetry render/reload handles, the health registry,
/// build provenance, and the redacted config snapshot. Everything defaults
/// **off** (management disabled, no handles, empty registry) so a server
/// without observability is the clean default, not a special case.
#[derive(Clone)]
pub struct Observability {
    /// The management surface configuration.
    pub management: ManagementConfig,
    /// The Prometheus render handle (present iff the recorder is installed).
    pub prometheus: Option<::prometheus::Registry>,
    /// The runtime log-filter control (present iff the reloadable filter is set).
    pub log_reload: Option<LogReload>,
    /// The health-indicator registry.
    pub health: HealthRegistry,
    /// Build/spec provenance.
    pub build_info: BuildInfo,
    /// The redacted effective-config snapshot for `/management/env`.
    pub env_snapshot: Arc<Value>,
}

impl Default for Observability {
    fn default() -> Self {
        Self {
            management: ManagementConfig::default(),
            prometheus: None,
            log_reload: None,
            health: HealthRegistry::default(),
            build_info: BuildInfo::current(),
            env_snapshot: Arc::new(Value::Null),
        }
    }
}

impl std::fmt::Debug for Observability {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Observability")
            .field("management", &self.management)
            .field("prometheus", &self.prometheus.is_some())
            .field("log_reload", &self.log_reload.is_some())
            .field("health_indicators", &!self.health.is_empty())
            .finish_non_exhaustive()
    }
}

/// Build the management router (a self-contained `Router` with its own state).
///
/// Only the opted-in endpoints are mounted; a disabled endpoint is simply
/// absent → `404`. When [`ManagementConfig::enabled`] is `false`, the router is
/// empty. This same router is either merged into the main app (when
/// `management.port` is unset) or served on its own listener by the binary.
pub fn router(state: ManagementState) -> Router {
    if !state.config.enabled {
        return Router::new();
    }

    let base = state.config.base_path.trim_end_matches('/').to_owned();
    let cfg = state.config.clone();
    let mut router = Router::new();

    // The per-endpoint access-level layer builder: an owned clone of the
    // authenticator (so `router.with_state(state)` below can still move `state`)
    // wrapped with the required level for one route.
    let authenticator = Arc::clone(&state.authenticator);
    let rbac = state.rbac.clone();
    let mk = |level: AccessLevel| {
        from_fn_with_state(
            AccessGuard {
                authenticator: Arc::clone(&authenticator),
                rbac: rbac.clone(),
                level,
            },
            access_middleware,
        )
    };

    // ── Info ─────────────────────────────────────────────────────────────────
    if cfg.endpoints.info.is_mounted() {
        router = router.route(
            &format!("{base}/info"),
            get(info_view).route_layer(mk(cfg.endpoints.info)),
        );
    }

    // ── Prometheus text exposition (only when the recorder is installed) ─────
    if cfg.endpoints.prometheus.is_mounted() && state.prometheus.is_some() {
        router = router.route(
            &format!("{base}/prometheus"),
            get(prometheus_text).route_layer(mk(cfg.endpoints.prometheus)),
        );
    }

    // ── Actuator-style JSON metrics view (only when the recorder is installed)
    if cfg.endpoints.metrics.is_mounted() && state.prometheus.is_some() {
        router = router
            .route(
                &format!("{base}/metrics"),
                get(metrics_list).route_layer(mk(cfg.endpoints.metrics)),
            )
            .route(
                &format!("{base}/metrics/{{name}}"),
                get(metrics_detail).route_layer(mk(cfg.endpoints.metrics)),
            );
    }

    // ── Env (redacted config) ──────────────────────────────────────────────
    if cfg.endpoints.env.is_mounted() {
        router = router.route(
            &format!("{base}/env"),
            get(env_view).route_layer(mk(cfg.endpoints.env)),
        );
    }

    // ── On-demand CPU flamegraph ─────────────────────────────────────────────
    if cfg.endpoints.flamegraph.is_mounted() {
        router = router.route(
            &format!("{base}/flamegraph"),
            get(flamegraph_view).route_layer(mk(cfg.endpoints.flamegraph)),
        );
    }

    // ── Health: every readiness indicator with its detail ──────────────────
    if cfg.endpoints.health.is_mounted() {
        router = router.route(
            &format!("{base}/health"),
            get(health_view).route_layer(mk(cfg.endpoints.health)),
        );
    }

    // ── Status: the full status document ───────────────────────────────────
    if cfg.endpoints.status.is_mounted() {
        router = router.route(
            &format!("{base}/status"),
            get(status_view).route_layer(mk(cfg.endpoints.status)),
        );
    }

    // ── Loggers (only when a reloadable filter is present) ──────────────────
    if cfg.endpoints.loggers.is_mounted() && state.log_reload.is_some() {
        router = router.route(
            &format!("{base}/loggers"),
            get(loggers_get)
                .post(loggers_post)
                .delete(loggers_reset)
                .route_layer(mk(cfg.endpoints.loggers)),
        );
    }

    router
        .layer(axum::middleware::from_fn(stamp_audit_op))
        .with_state(state)
}

/// Stamp the audit operation id on every management response (#3244), so the
/// audit middleware records the request under a `management_*` operation the
/// classification table names, rather than the anonymous default.
async fn stamp_audit_op(req: axum::extract::Request, next: Next) -> Response {
    let op = audit_op_for(req.method(), req.uri().path());
    let mut resp = next.run(req).await;
    resp.extensions_mut()
        .insert(crate::system_log::middleware::AuditOpId(op));
    resp
}

/// The audit operation id of a management request: the endpoint, and for the
/// loggers the verb, because a filter change is a different act from a read.
fn audit_op_for(method: &http::Method, path: &str) -> &'static str {
    // `/metrics/{name}`: the endpoint is the segment before the name, whatever
    // the name spells.
    if path.contains("/metrics/") {
        return "management_metrics";
    }
    let endpoint = path
        .trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    match (method, endpoint) {
        (_, "info") => "management_info",
        (_, "prometheus") => "management_prometheus",
        (_, "metrics") => "management_metrics",
        (_, "env") => "management_env",
        (_, "flamegraph") => "management_flamegraph",
        (_, "health") => "management_health",
        (_, "status") => "management_status",
        (m, "loggers") if m == http::Method::POST => "management_loggers_set",
        (m, "loggers") if m == http::Method::DELETE => "management_loggers_reset",
        (_, "loggers") => "management_loggers_get",
        _ => "management",
    }
}

// ── OpenAPI document (the full management surface, documented unconditionally) ─

/// The management surface's `OpenAPI` document.
///
/// Every management operation is documented **unconditionally** (the document
/// describes the product surface; the live [`router`] mounts only the opted-in
/// endpoints). Built natively with `utoipa-axum` so each operation's route +
/// `OpenAPI` path come from the one `#[utoipa::path]` handler; only the
/// `OpenApi` half is kept here (the mounted, access-gated router is built by
/// [`router`]).
///
/// No openEHR spec governs the management surface — our own operational design.
#[must_use]
pub fn openapi() -> utoipa::openapi::OpenApi {
    OpenApiRouter::<ManagementState>::new()
        .routes(routes!(info_view))
        .routes(routes!(prometheus_text))
        .routes(routes!(metrics_list))
        .routes(routes!(metrics_detail))
        .routes(routes!(env_view))
        .routes(routes!(loggers_get, loggers_post, loggers_reset))
        .routes(routes!(flamegraph_view))
        .routes(routes!(health_view))
        .routes(routes!(status_view))
        .into_openapi()
}

// ── Handlers ────────────────────────────────────────────────────────────────

// The handlers below have no I/O of their own but must be `async` — axum only
// implements `Handler` for async functions; the `unused_async` allow records
// that the work is synchronous and the async is the framework contract.

/// Build/spec provenance — version, git, spec pins, manufacturer
/// (`GET /management/info`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// Access-level gated by the `info` endpoint's configured [`AccessLevel`];
/// absent (a router `404`) unless opted in — and that `404` is answered
/// **before authentication**, because a non-opted-in endpoint is simply not a
/// route. Body: the [`BuildInfo`] record.
#[utoipa::path(
    get, path = "/management/info", tag = "management",
    responses(
        (status = 200, description = "Build + spec provenance and the manufacturer of this release.", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value)
    )
)]
async fn info_view(State(s): State<ManagementState>) -> Json<BuildInfo> {
    info_routes::info(s.build_info)
}

/// Prometheus text exposition (`GET /management/prometheus`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// Access-level gated by the `prometheus` endpoint's configured [`AccessLevel`];
/// the live [`router`] mounts this route only when the endpoint is opted in AND
/// the metrics recorder is installed (otherwise it is absent — a router `404`,
/// answered before authentication).
#[utoipa::path(
    get, path = "/management/prometheus", tag = "management",
    responses(
        (status = 200, description = "Prometheus exposition text.", content_type = "text/plain"),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value),
        (status = 503, description = "Metrics recorder not installed (defensive; the live router mounts this route only when the recorder is present).", body = serde_json::Value)
    )
)]
async fn prometheus_text(State(s): State<ManagementState>) -> Response {
    match &s.prometheus {
        Some(handle) => metrics::prometheus(handle),
        None => recorder_unavailable(),
    }
}

/// Actuator-style JSON list of known metric names (`GET /management/metrics`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// Access-level gated by the `metrics` endpoint's configured [`AccessLevel`];
/// the live [`router`] mounts this route only when the endpoint is opted in AND
/// the metrics recorder is installed (otherwise it is absent — a router `404`,
/// answered before authentication).
#[utoipa::path(
    get, path = "/management/metrics", tag = "management",
    responses(
        (status = 200, description = "Known metric names.", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value),
        (status = 503, description = "Metrics recorder not installed (defensive; the live router mounts this route only when the recorder is present).", body = serde_json::Value)
    )
)]
async fn metrics_list(State(s): State<ManagementState>) -> Response {
    match &s.prometheus {
        Some(handle) => metrics::list(handle).into_response(),
        None => recorder_unavailable(),
    }
}

/// Actuator-style JSON detail for one metric (`GET /management/metrics/{name}`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// Access-level gated by the `metrics` endpoint's configured [`AccessLevel`];
/// the live [`router`] mounts this route only when the endpoint is opted in AND
/// the metrics recorder is installed (otherwise it is absent — a router `404`,
/// answered before authentication).
#[utoipa::path(
    get, path = "/management/metrics/{name}", tag = "management",
    params(("name" = String, Path, description = "The metric name to inspect.")),
    responses(
        (status = 200, description = "The metric's current value(s).", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value),
        (status = 404, description = "No metric with that name is registered.", body = serde_json::Value),
        (status = 503, description = "Metrics recorder not installed (defensive; the live router mounts this route only when the recorder is present).", body = serde_json::Value)
    )
)]
async fn metrics_detail(State(s): State<ManagementState>, path: Path<String>) -> Response {
    match &s.prometheus {
        Some(handle) => metrics::detail(handle, path),
        None => recorder_unavailable(),
    }
}

/// The redacted effective-configuration snapshot (`GET /management/env`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// Access-level gated by the `env` endpoint's configured [`AccessLevel`]; absent
/// (a router `404`, answered before authentication) unless opted in. Body: the
/// effective config with secrets redacted at render.
#[utoipa::path(
    get, path = "/management/env", tag = "management",
    responses(
        (status = 200, description = "Redacted effective configuration.", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value)
    )
)]
async fn env_view(State(s): State<ManagementState>) -> Json<Value> {
    env::env(&s.env_snapshot)
}

/// The effective log-filter directives + boot filter (`GET /management/loggers`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// Access-level gated by the `loggers` endpoint's configured [`AccessLevel`];
/// the live [`router`] mounts the `loggers` routes only when the endpoint is
/// opted in AND a reloadable filter is installed (otherwise absent — a router
/// `404`, answered before authentication).
#[utoipa::path(
    get, path = "/management/loggers", tag = "management",
    responses(
        (status = 200, description = "Effective + boot log filter.", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value),
        (status = 503, description = "No reloadable filter installed (defensive; the live router mounts this route only when a reloadable filter is present).", body = serde_json::Value)
    )
)]
async fn loggers_get(State(s): State<ManagementState>) -> Response {
    match &s.log_reload {
        Some(reload) => logger_routes::get(reload).into_response(),
        None => recorder_unavailable(),
    }
}

/// Swap the live log filter (`POST /management/loggers`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// Body: `{"filter": "ferroehr=debug,sqlx=warn"}`. Access-level gated by the
/// `loggers` endpoint's configured [`AccessLevel`]; the live [`router`] mounts
/// this route only when the endpoint is opted in AND a reloadable filter is
/// installed (otherwise absent — a router `404`, answered before
/// authentication).
#[utoipa::path(
    post, path = "/management/loggers", tag = "management",
    request_body(content = serde_json::Value, description = "`{\"filter\": \"<env-filter directives>\"}`"),
    responses(
        (status = 200, description = "Filter applied; the new effective filter is returned.", body = serde_json::Value),
        (status = 400, description = "Malformed `env-filter` directives (parse error).", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value),
        (status = 503, description = "No reloadable filter installed (defensive; the live router mounts this route only when a reloadable filter is present).", body = serde_json::Value)
    )
)]
async fn loggers_post(
    State(s): State<ManagementState>,
    body: Json<logger_routes::SetFilter>,
) -> Response {
    match &s.log_reload {
        Some(reload) => logger_routes::set(reload, &body.0),
        None => recorder_unavailable(),
    }
}

/// Reset the log filter to the boot filter (`DELETE /management/loggers`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// Access-level gated by the `loggers` endpoint's configured [`AccessLevel`];
/// the live [`router`] mounts this route only when the endpoint is opted in AND
/// a reloadable filter is installed (otherwise absent — a router `404`,
/// answered before authentication).
#[utoipa::path(
    delete, path = "/management/loggers", tag = "management",
    responses(
        (status = 200, description = "Filter reset to the boot value; the restored filter is returned.", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value),
        (status = 503, description = "No reloadable filter installed (defensive; the live router mounts this route only when a reloadable filter is present).", body = serde_json::Value)
    )
)]
async fn loggers_reset(State(s): State<ManagementState>) -> Response {
    match &s.log_reload {
        Some(reload) => logger_routes::reset(reload),
        None => recorder_unavailable(),
    }
}

/// Sample the process CPU and render a flamegraph SVG
/// (`GET /management/flamegraph`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// Samples the whole process for `seconds` (default 10) at `frequency` Hz
/// (default 99) via the `pprof` sampler and answers with the rendered
/// flamegraph SVG — the "where does the time go" instrument. One sample window
/// at a time (`409` while one runs); parameters beyond the configured
/// `management.profiling` caps are refused with `400`, never clamped.
/// Access-level gated by the `flamegraph` endpoint's configured
/// [`AccessLevel`]; absent (a router `404`, answered before authentication)
/// unless opted in.
#[utoipa::path(
    get, path = "/management/flamegraph", tag = "management",
    params(
        ("seconds" = Option<u16>, Query, description = "Sample window in seconds (default 10; capped by management.profiling.max_seconds)."),
        ("frequency" = Option<i32>, Query, description = "Sampling frequency in Hz (default 99; capped by management.profiling.max_frequency).")
    ),
    responses(
        (status = 200, description = "The rendered CPU flamegraph.", content_type = "image/svg+xml"),
        (status = 400, description = "A parameter is outside the configured management.profiling caps.", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value),
        (status = 409, description = "A profiling sample window is already running.", body = serde_json::Value),
        (status = 500, description = "The profiler failed to start, sample, or render.", body = serde_json::Value)
    )
)]
async fn flamegraph_view(
    State(s): State<ManagementState>,
    Query(params): Query<flamegraph::FlamegraphParams>,
) -> Response {
    match flamegraph::sample_flamegraph_svg(&s.profiler, s.config.profiling, &params).await {
        Ok(svg) => ([(header::CONTENT_TYPE, "image/svg+xml")], svg).into_response(),
        Err(err) => RestError(err).into_response(),
    }
}

/// Every readiness indicator with its detail (`GET /management/health`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// The same registry evaluation as the public `GET /health/readiness`, with
/// each indicator's detail, which the public probe withholds. The status code
/// follows the aggregate the same way: `200` while `UP` or `DEGRADED`, `503`
/// once a required indicator is `DOWN`. Access-level gated by the `health`
/// endpoint's configured [`AccessLevel`]; absent (a router `404`, answered
/// before authentication) unless opted in.
#[utoipa::path(
    get, path = "/management/health", tag = "management",
    responses(
        (status = 200, description = "Aggregate UP or DEGRADED; `{status, components: {<name>: {status, detail?}}}`.", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value),
        (status = 503, description = "A required indicator is DOWN; the same body, naming it and why.", body = serde_json::Value)
    )
)]
async fn health_view(State(s): State<ManagementState>) -> Response {
    AggregateHealthResponse(s.health.evaluate().await).into_response()
}

/// The aggregate rendered with its own HTTP status
/// ([`AggregateHealth::http_status`]).
struct AggregateHealthResponse(AggregateHealth);

impl IntoResponse for AggregateHealthResponse {
    fn into_response(self) -> Response {
        (self.0.http_status(), Json(self.0)).into_response()
    }
}

/// The full status document (`GET /management/status`).
///
/// OUR OWN EXTENSION — no openEHR spec governs this: the ITS-REST resource set
/// defines no management or introspection surface.
///
/// The public `GET /ferroehr/rest/status` fields plus the licence in force,
/// the complete deployment posture (the separations not made and the ones
/// accepted by name) and the support period of this release. Access-level
/// gated by the `status` endpoint's configured [`AccessLevel`]; absent (a
/// router `404`, answered before authentication) unless opted in.
#[utoipa::path(
    get, path = "/management/status", tag = "management",
    responses(
        (status = 200, description = "A JSON `{status, server_version, openehr_rest_api_version, timestamp, licence: {state, use?, licensee?, not_after?, configured_token}, deployment: {profile, gaps, accepted}, support: {release_date, support_ends, status}}` object.", body = serde_json::Value),
        (status = 401, description = "Authentication required (access level Private/AdminOnly with auth enabled).", body = serde_json::Value),
        (status = 403, description = "Caller lacks the configured admin scope (access level AdminOnly).", body = serde_json::Value)
    )
)]
async fn status_view(State(s): State<ManagementState>) -> Json<FullStatus> {
    Json(FullStatus::now(s.licence, s.deployment))
}

fn recorder_unavailable() -> Response {
    (
        StatusCode::SERVICE_UNAVAILABLE,
        Json(
            serde_json::json!({ "error": "the corresponding telemetry facility is not installed" }),
        ),
    )
        .into_response()
}

// ── Access-level layer ────────────────────────────────────────────────────

/// The per-endpoint access guard: the shared authenticator plus the required
/// level for one route.
#[derive(Clone)]
pub(crate) struct AccessGuard {
    pub(crate) authenticator: Arc<Authenticator>,
    pub(crate) rbac: RbacConfig,
    pub(crate) level: AccessLevel,
}

/// The access-level middleware: enforce the guard's level, then run the route
/// (or short-circuit with `401`/`403`/`404`). Installed per-route via the `mk`
/// closure in [`router`], and around the Swagger surface by the main router.
pub(crate) async fn access_middleware(
    State(guard): State<AccessGuard>,
    req: axum::extract::Request,
    next: Next,
) -> Response {
    match guard.check(req.headers()).await {
        Ok(principal) => {
            let mut resp = next.run(req).await;
            // Republished for the audit layer outside this router, which
            // names the person behind a management request from the response
            // extensions the way the API's authentication layer does (#3244).
            if let Some(principal) = principal {
                resp.extensions_mut().insert(principal);
            }
            resp
        }
        Err(resp) => *resp,
    }
}

impl AccessGuard {
    /// Enforce the access level against the request headers, handing back the
    /// authenticated principal where one was established.
    async fn check(
        &self,
        headers: &HeaderMap,
    ) -> Result<Option<crate::extensions::access::authn::Principal>, Box<Response>> {
        match self.level {
            // Defensive: an `Off` endpoint is never mounted; if reached, 404.
            AccessLevel::Off => Err(Box::new(StatusCode::NOT_FOUND.into_response())),
            AccessLevel::Public => Ok(None),
            AccessLevel::Private | AccessLevel::AdminOnly => {
                // Auth disabled (dev): the surface is unauthenticated by design.
                if !self.authenticator.enabled() {
                    return Ok(None);
                }
                #[expect(
                    clippy::map_err_ignore,
                    reason = "the gate answers one 401 challenge for every \
                              authentication failure: which part of the credential \
                              was rejected is never disclosed"
                )]
                let authenticated = self
                    .authenticator
                    .authenticate(headers)
                    .await
                    .map_err(|_| Box::new(unauthorized(&self.authenticator)))?;
                if self.level == AccessLevel::AdminOnly
                    && let RbacDecision::Deny(reason) =
                        crate::extensions::access::authz::roles::authorize(
                            OperationClass::Admin,
                            &authenticated.principal.roles,
                            &self.rbac,
                        )
                {
                    return Err(Box::new(forbidden(&reason)));
                }
                Ok(Some(authenticated.principal))
            }
        }
    }
}

fn unauthorized(authenticator: &Authenticator) -> Response {
    let mut resp =
        RestError(ApiError::Unauthorized("authentication required".to_owned())).into_response();
    // No credential was presented, so the challenge carries no RFC 6750 §3.1
    // error code: "If the request lacks any authentication information … the
    // resource server SHOULD NOT include an error code".
    resp.headers_mut()
        .insert(header::WWW_AUTHENTICATE, authenticator.challenge(None));
    resp
}

fn forbidden(reason: &str) -> Response {
    RestError(ApiError::Forbidden(format!(
        "management endpoint: {reason}"
    )))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::audit_op_for;
    use http::Method;

    /// Every management endpoint maps to its own audit operation, and a metric
    /// whose name spells another endpoint is still a metrics read.
    #[test]
    fn each_endpoint_has_its_audit_operation() {
        for (method, path, op) in [
            (Method::GET, "/management/info", "management_info"),
            (Method::GET, "/management/health", "management_health"),
            (Method::GET, "/management/status", "management_status"),
            (Method::GET, "/management/metrics", "management_metrics"),
            (
                Method::GET,
                "/management/metrics/status",
                "management_metrics",
            ),
            (
                Method::GET,
                "/management/metrics/health",
                "management_metrics",
            ),
            (
                Method::POST,
                "/management/loggers",
                "management_loggers_set",
            ),
            (
                Method::DELETE,
                "/management/loggers",
                "management_loggers_reset",
            ),
            (Method::GET, "/management/loggers", "management_loggers_get"),
        ] {
            assert_eq!(audit_op_for(&method, path), op, "{method} {path}");
        }
    }
}
