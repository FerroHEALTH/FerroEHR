// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The CDR's **operational surfaces** the viewer consumes.
//!
//! The CDR's dependency health (`/management/health`, or the public
//! `/health/readiness` statuses when that endpoint is not offered) and its
//! management surface (build info, metric views, the redacted effective
//! config, the live log-filter control).
//!
//! NOTE: no openEHR spec governs any of this — our own operational surface /
//! product extension. The vendored ITS-REST System API defines exactly one
//! operation (`OPTIONS {base_path}`, the conformance manifest —
//! `docs/specs/openehr/ITS-REST/specifications/docs/system/`) and no health,
//! metrics, config or logging resource at all.
//!
//! **Two health readers would be one too many.** The application shell's status
//! pill polls the product status document (`{rest root}/status`); this module
//! reads the *other* health contract, the dependency indicators, and nothing
//! else re-reads either claim. The CDR serves the indicators twice: each one's
//! status on the public `/health/readiness`, and with its detail on the
//! authenticated `/management/health`. [`fetch_readiness`] reads the detailed
//! one and falls back to the statuses only when it is not offered.
//!
//! **Probe-and-hide.** The management surface is off by default and each of its
//! endpoints is independently opt-in, so the viewer discovers it
//! ([`probe_management_api`]) before offering any of it, exactly as
//! [`crate::admin`] discovers the CDR's admin group. Capability is not
//! authorization: a mounted-but-refused endpoint (`401`/`403`) still counts as
//! present, and the refusal surfaces on the screen that asked.
//!
//! Every `#[server]` fn guards with
//! [`require_session`](crate::session::require_session) first (a server fn is a
//! publicly reachable HTTP endpoint) and keeps the CDR credential server-side.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use leptos::prelude::*;
use leptos::server;
use serde::{Deserialize, Serialize};

use crate::error::ViewerError;

/// One headline metric: its name, its tile label, and an optional
/// `(label, value)` sample filter.
#[cfg(feature = "ssr")]
type HeadlineMetric = (
    &'static str,
    &'static str,
    Option<(&'static str, &'static str)>,
);

/// The headline metric tiles, in display order.
///
/// Each is a counter or gauge the CDR registers
/// (`ferroehr::telemetry::metrics`); a histogram is deliberately absent, because
/// the CDR's detail view folds a histogram's `_bucket`/`_sum`/`_count` lines
/// into one sample list and summing it would report a meaningless number.
// NOTE: these are the Prometheus exporter's RENDERED names — `_total` is derived
// from the counter kind, never written on the instrument — and the correspondence
// is pinned by the CDR's `exporter_renders_the_viewer_metric_names` test.
#[cfg(feature = "ssr")]
const HEADLINE_METRICS: [HeadlineMetric; 4] = [
    ("http_server_active_requests", "In-flight requests", None),
    (
        "compositions_committed_total",
        "Compositions committed",
        None,
    ),
    ("aql_queries_total", "AQL queries", None),
    (
        "db_pool_connections",
        "DB connections in use",
        Some(("state", "in_use")),
    ),
];

/// Whether the CDR serves its management surface at the configured base URL.
///
/// Carries only fixed-size, client-safe data — it crosses the server-fn
/// boundary.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ManagementAvailability {
    /// The surface answered: it is mounted, so the operations panel is offered.
    /// Whether THIS session may read each endpoint is a per-request answer.
    Available,
    /// The surface is not mounted (`management.enabled = false`, or the probed
    /// endpoint's access level is `Off`) — the CDR answered `404`.
    Disabled,
}

impl ManagementAvailability {
    /// Whether the viewer may offer the operations panel.
    #[must_use]
    pub fn usable(self) -> bool {
        matches!(self, Self::Available)
    }
}

/// Classify the probe's HTTP status: only a `404` means "not mounted".
///
/// A `401`/`403` is a mounted endpoint refusing THIS session (the access level
/// is `Private`/`AdminOnly`), and a `503` is a mounted endpoint whose telemetry
/// facility is absent — both are the surface existing, which is what the panel
/// gate asks about.
#[must_use]
pub fn availability_of_status(status: http::StatusCode) -> ManagementAvailability {
    if status == http::StatusCode::NOT_FOUND {
        ManagementAvailability::Disabled
    } else {
        ManagementAvailability::Available
    }
}

/// The single predicate every management-gated affordance uses: render only
/// for a probe that succeeded and found the surface mounted.
///
/// A failed probe (CDR or management listener unreachable, expired session)
/// hides it — never offer a screen that cannot work.
#[must_use]
pub fn renders_management_ops(probe: &Result<ManagementAvailability, ViewerError>) -> bool {
    probe
        .as_ref()
        .copied()
        .is_ok_and(ManagementAvailability::usable)
}

/// The management gate: one probe [`Resource`].
///
/// Created in component setup — never inside a `Suspend` closure, which re-runs
/// and would re-create the resource.
#[must_use]
pub fn management_gate() -> Resource<Result<ManagementAvailability, ViewerError>> {
    Resource::new(|| (), |()| async move { probe_management_api().await })
}

/// Render `affordance` only when the gate found the management surface mounted;
/// otherwise render nothing at all (probe-and-hide).
///
/// The probe is resolved INSIDE the `<Suspense>`: an SSR'd `ErrorBoundary`
/// fallback mismatches at hydration in leptos 0.8, and a render-time resource
/// read is itself a hydration mismatch. `affordance` creates no resources, so
/// re-runs are safe, and it is shared through an `Arc` because the `Suspend`
/// closure must not consume its environment.
#[must_use]
pub fn when_management_usable(
    gate: Resource<Result<ManagementAvailability, ViewerError>>,
    affordance: impl Fn() -> AnyView + Send + Sync + 'static,
) -> AnyView {
    let affordance: std::sync::Arc<dyn Fn() -> AnyView + Send + Sync> =
        std::sync::Arc::new(affordance);
    view! {
        <Suspense fallback=|| ()>
            {move || {
                let affordance = std::sync::Arc::clone(&affordance);
                Suspend::new(async move {
                    if renders_management_ops(&gate.await) { affordance() } else { ().into_any() }
                })
            }}
        </Suspense>
    }
    .into_any()
}

// ── Health: the dependency-health indicators ────────────────────────────────

/// One health indicator's contribution to readiness.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndicatorRow {
    /// The component name (`db`, `migrations`, …).
    pub name: String,
    /// `UP`, `DEGRADED` or `DOWN`, as the CDR reported it.
    pub status: String,
    /// The CDR's human detail, when it gave one (never clinical content).
    pub detail: String,
}

/// The dependency-health body: the aggregate plus every indicator.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ReadinessView {
    /// The aggregate status (`UP` / `DEGRADED` / `DOWN`).
    pub status: String,
    /// Per-indicator rows, name-sorted so both render passes agree
    /// (hydration determinism).
    pub components: Vec<IndicatorRow>,
    /// Whether the rows carry the indicators' detail: `true` when read from
    /// `/management/health`, `false` from the status-only public probe.
    pub detailed: bool,
}

/// Distil a `/management/health` or `/health/readiness` body into a
/// [`ReadinessView`].
///
/// Both carry `{status, components: {<name>: {status}}}`; only the management
/// body adds a `detail` per component. Read defensively: a missing/renamed
/// field yields an empty string rather than dropping the row, and an
/// unparseable aggregate renders as `UNKNOWN` instead of erroring — the probe
/// answering at all is itself information.
#[must_use]
pub fn readiness_view(body: &serde_json::Value, detailed: bool) -> ReadinessView {
    let status = body
        .get("status")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("UNKNOWN")
        .to_owned();
    let mut components: Vec<IndicatorRow> = body
        .get("components")
        .and_then(serde_json::Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(name, value)| IndicatorRow {
                    name: name.clone(),
                    status: value
                        .get("status")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or("UNKNOWN")
                        .to_owned(),
                    detail: value
                        .get("detail")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                })
                .collect()
        })
        .unwrap_or_default();
    components.sort_by(|a, b| a.name.cmp(&b.name));
    ReadinessView {
        status,
        components,
        detailed,
    }
}

/// Whether a `/management/health` answer sends the reader to the public
/// status-only probe instead: `404` (the endpoint is not mounted) or `403`
/// (its access level refuses this session's role). A `401` is not a
/// fallback: the CDR no longer accepts the session, which the screen reports.
#[must_use]
pub fn falls_back_to_public_readiness(status: http::StatusCode) -> bool {
    status == http::StatusCode::NOT_FOUND || status == http::StatusCode::FORBIDDEN
}

/// Read the CDR's dependency health.
///
/// `GET /management/health` with the session's credential serves every
/// indicator with its detail. Where that endpoint is not mounted or refuses
/// this session's role ([`falls_back_to_public_readiness`]), the public
/// `GET /health/readiness` on the CDR's API origin still gives each
/// indicator's status, without detail. On either, `503` is the DOWN state,
/// not a failure: the body carries the indicators that explain it, which is
/// exactly what the panel renders.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] when the CDR no longer accepts this
/// session; [`ViewerError::CdrUnreachable`] on transport failure;
/// [`ViewerError::Cdr`] on any other status than `200`/`503`;
/// [`ViewerError::Internal`] when the body is not JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_readiness() -> Result<ReadinessView, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.management_url("health");
    let detailed = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    let (response, is_detailed) = if falls_back_to_public_readiness(detailed.status) {
        let url = state.cdr.origin_url("health/readiness");
        (state.cdr.get_public(&url, "application/json").await?, false)
    } else {
        (detailed, true)
    };
    let body = if response.is(http::StatusCode::SERVICE_UNAVAILABLE) {
        response.body
    } else {
        crate::cdr::CdrClient::expect_success(response)?.body
    };
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("readiness JSON: {e}")))?;
    Ok(readiness_view(&value, is_detailed))
}

// ── Management: the probe ───────────────────────────────────────────────────

/// Probe the CDR's management surface.
///
/// `GET {management}/info` is the cheapest management operation (no I/O behind
/// it — the build facts are captured at boot), so it is the availability
/// signal: a `404` means the surface is not mounted for this deployment, any
/// other answer means it is. A deployment that mounts other management
/// endpoints while leaving `info` at `Off` therefore hides the whole panel, so
/// the panel's presence rests on one cheap, stable answer.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnreachable`] when the management listener is unreachable.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn probe_management_api() -> Result<ManagementAvailability, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.management_url("info");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    Ok(availability_of_status(response.status))
}

// ── Management: build + spec provenance ─────────────────────────────────────

/// The `GET /management/info` body as rendered rows.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct BuildInfoView {
    /// Top-level scalar facts (`version`, `git_sha`, `rustc`, …) as
    /// `(key, value)`, key-sorted.
    pub facts: Vec<(String, String)>,
    /// The `spec` sub-object's pinned specification versions as
    /// `(key, value)`, key-sorted.
    pub spec: Vec<(String, String)>,
}

/// Distil a `/management/info` body into a [`BuildInfoView`].
///
/// Generic over the field set on purpose: the CDR owns that shape and may add
/// facts, so every scalar leaf is rendered rather than a fixed list, and the
/// nested `spec` object is lifted into its own section.
#[must_use]
pub fn build_info_view(body: &serde_json::Value) -> BuildInfoView {
    BuildInfoView {
        facts: scalar_rows(body),
        spec: body.get("spec").map(scalar_rows).unwrap_or_default(),
    }
}

/// Every scalar leaf of a JSON object as `(key, text)`, key-sorted for a
/// deterministic (hydration-safe) render order.
fn scalar_rows(value: &serde_json::Value) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = Vec::new();
    if let Some(object) = value.as_object() {
        for (key, value) in object {
            let text = match value {
                serde_json::Value::String(s) => Some(s.clone()),
                serde_json::Value::Number(n) => Some(n.to_string()),
                serde_json::Value::Bool(b) => Some(b.to_string()),
                _ => None,
            };
            if let Some(text) = text {
                rows.push((key.clone(), text));
            }
        }
    }
    rows.sort();
    rows
}

/// Read the CDR's build + spec provenance (`GET /management/info`).
///
/// `Ok(None)` is the first-class "not mounted" state (the CDR answered `404`),
/// not an error.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::CdrUnauthorized`] when the CDR no longer accepts this
/// session, [`ViewerError::Forbidden`] when the endpoint's access level
/// refuses it; [`ViewerError::Cdr`] / [`ViewerError::CdrUnreachable`] from the
/// CDR; [`ViewerError::Internal`] when the body is not JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_build_info() -> Result<Option<BuildInfoView>, ViewerError> {
    let Some(body) = management_get("info").await? else {
        return Ok(None);
    };
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("management info JSON: {e}")))?;
    Ok(Some(build_info_view(&value)))
}

// ── Management: metrics ─────────────────────────────────────────────────────

/// One headline metric tile.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MetricTile {
    /// The metric name (the browser's selection value).
    pub name: String,
    /// The human label.
    pub label: String,
    /// The formatted value, or `—` when the CDR registers no such metric.
    pub value: String,
}

/// One sample of a metric: its labels, flattened for display, and its value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricSample {
    /// The sample's labels as `k=v, k2=v2` (empty for an unlabelled sample).
    pub labels: String,
    /// The sample value.
    pub value: f64,
}

/// The `GET /management/metrics/{name}` body as rendered rows.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MetricDetailView {
    /// The metric name.
    pub name: String,
    /// The Prometheus metric type (`counter`, `gauge`, `histogram`), when the
    /// exposition declared one.
    pub kind: String,
    /// The metric's `# HELP` text, when the exposition declared one.
    pub help: String,
    /// The samples, in the order the CDR listed them.
    pub samples: Vec<MetricSample>,
}

/// Distil a `/management/metrics/{name}` body into a [`MetricDetailView`].
#[must_use]
pub fn metric_detail_view(body: &serde_json::Value) -> MetricDetailView {
    let text = |key: &str| {
        body.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let samples = body
        .get("measurements")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .map(|item| MetricSample {
                    labels: item
                        .get("labels")
                        .and_then(serde_json::Value::as_object)
                        .map(|labels| {
                            labels
                                .iter()
                                .map(|(k, v)| match v.as_str() {
                                    Some(text) => format!("{k}={text}"),
                                    None => format!("{k}={v}"),
                                })
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default(),
                    value: item
                        .get("value")
                        .and_then(serde_json::Value::as_f64)
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    MetricDetailView {
        name: text("name"),
        kind: text("metric_type"),
        help: text("help"),
        samples,
    }
}

/// The total across a metric's samples, optionally restricted to the samples
/// carrying `label=value`. `None` when nothing matched (nothing to report is
/// not zero).
#[must_use]
pub fn metric_total(detail: &MetricDetailView, filter: Option<(&str, &str)>) -> Option<f64> {
    let matching = detail.samples.iter().filter(|sample| match filter {
        None => true,
        Some((label, value)) => sample.labels.split(", ").any(|pair| {
            pair.split_once('=')
                .is_some_and(|(k, v)| k == label && v == value)
        }),
    });
    let mut total = 0.0;
    let mut seen = false;
    for sample in matching {
        total += sample.value;
        seen = true;
    }
    seen.then_some(total)
}

/// Format a metric total for a tile: integral values without a fraction,
/// everything else to two decimals.
#[must_use]
pub fn format_metric(value: f64) -> String {
    if value.fract() == 0.0 {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

/// List the metric names the CDR's registry knows (`GET /management/metrics`).
///
/// `Ok(None)` = the endpoint is not mounted (`404`), a first-class state.
///
/// # Errors
/// As [`fetch_build_info`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_metric_names() -> Result<Option<Vec<String>>, ViewerError> {
    let Some(body) = management_get("metrics").await? else {
        return Ok(None);
    };
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("metric list JSON: {e}")))?;
    let mut names: Vec<String> = value
        .get("names")
        .and_then(serde_json::Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(serde_json::Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    Ok(Some(names))
}

/// Read one metric's current samples (`GET /management/metrics/{name}`).
///
/// The name is a CDR-supplied string, so the path segment is percent-encoded
/// with the `urlencoding` crate. `Ok(None)` = no such metric, or the endpoint is
/// not mounted — both a `404`, both a first-class rendered state.
///
/// # Errors
/// [`ViewerError::Invalid`] for an empty name; otherwise as
/// [`fetch_build_info`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_metric_detail(
    /// The metric name to read, as listed by the metrics index.
    name: String,
) -> Result<Option<MetricDetailView>, ViewerError> {
    if name.trim().is_empty() {
        return Err(ViewerError::Invalid("no metric name given".to_owned()));
    }
    let path = format!("metrics/{}", urlencoding::encode(&name));
    let Some(body) = management_get(&path).await? else {
        return Ok(None);
    };
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("metric detail JSON: {e}")))?;
    Ok(Some(metric_detail_view(&value)))
}

/// The headline metric tiles: the registry's names, then one detail read per
/// `HEADLINE_METRICS` entry the CDR actually registers (so a tile never
/// provokes a `404`, and a metric the deployment does not record renders `—`).
///
/// `Ok(None)` = the metrics endpoint is not mounted.
///
/// # Errors
/// As [`fetch_build_info`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_headline_metrics() -> Result<Option<Vec<MetricTile>>, ViewerError> {
    let Some(names) = fetch_metric_names().await? else {
        return Ok(None);
    };
    let mut tiles = Vec::with_capacity(HEADLINE_METRICS.len());
    for (name, label, filter) in HEADLINE_METRICS {
        let value = if names.iter().any(|known| known == name) {
            fetch_metric_detail(name.to_owned())
                .await?
                .as_ref()
                .and_then(|detail| metric_total(detail, filter))
                .map_or_else(|| "—".to_owned(), format_metric)
        } else {
            "—".to_owned()
        };
        tiles.push(MetricTile {
            name: name.to_owned(),
            label: label.to_owned(),
            value,
        });
    }
    Ok(Some(tiles))
}

// ── Management: the live log filter ─────────────────────────────────────────

/// The `GET /management/loggers` body: the effective filter and the boot filter
/// a reset restores.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LoggerView {
    /// The filter directives in effect right now.
    pub filter: String,
    /// The boot-time directives — the reset target.
    pub boot_filter: String,
}

/// Distil a `/management/loggers` body into a [`LoggerView`].
#[must_use]
pub fn logger_view(body: &serde_json::Value) -> LoggerView {
    let text = |key: &str| {
        body.get(key)
            .and_then(serde_json::Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    LoggerView {
        filter: text("filter"),
        boot_filter: text("boot_filter"),
    }
}

/// Read the CDR's live log filter (`GET /management/loggers`).
///
/// `Ok(None)` = the endpoint is not mounted, which the CDR also answers when it
/// runs without a reloadable filter installed.
///
/// # Errors
/// As [`fetch_build_info`].
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn fetch_loggers() -> Result<Option<LoggerView>, ViewerError> {
    let Some(body) = management_get("loggers").await? else {
        return Ok(None);
    };
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("loggers JSON: {e}")))?;
    Ok(Some(logger_view(&value)))
}

/// Swap the CDR's live log filter (`POST /management/loggers`,
/// `{"filter": "…"}`).
///
/// A mutation, so a `404` here IS an error: the affordance renders only after
/// [`fetch_loggers`] found the endpoint, so an absent route means the surface
/// changed underneath the screen. The CDR's `400` (unparseable directives)
/// carries its own diagnostic verbatim.
///
/// # Errors
/// [`ViewerError::Unauthenticated`] without a viewer session;
/// [`ViewerError::Invalid`] for an empty filter;
/// [`ViewerError::CdrUnauthorized`] / [`ViewerError::Forbidden`] / [`ViewerError::Cdr`] /
/// [`ViewerError::CdrUnreachable`] from the CDR; [`ViewerError::Internal`]
/// when the body is not JSON.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn set_log_filter(
    /// The replacement log filter, one or more `target=level` directives.
    filter: String,
) -> Result<LoggerView, ViewerError> {
    let session = crate::session::require_session().await?;
    let filter = filter.trim().to_owned();
    if filter.is_empty() {
        return Err(ViewerError::Invalid(
            "a log filter needs at least one directive, e.g. `ferroehr=debug`".to_owned(),
        ));
    }
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.management_url("loggers");
    let body = serde_json::json!({ "filter": filter }).to_string();
    let response = state
        .cdr
        .post(
            &session.credential,
            &url,
            "application/json",
            "application/json",
            &[],
            body,
        )
        .await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("loggers JSON: {e}")))?;
    Ok(logger_view(&value))
}

/// Restore the CDR's boot log filter (`DELETE /management/loggers`).
///
/// # Errors
/// As [`set_log_filter`], minus the empty-input case.
#[server(client = crate::session_client::SessionAwareClient)]
pub async fn reset_log_filter() -> Result<LoggerView, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.management_url("loggers");
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body)
        .map_err(|e| ViewerError::Internal(format!("loggers JSON: {e}")))?;
    Ok(logger_view(&value))
}

/// GET one management endpoint as the session's credential, mapping the CDR's
/// `404` to `Ok(None)` — the "endpoint not mounted" state every management read
/// renders as a first-class absence rather than an error.
///
/// Guards the viewer session first: the server fns above are publicly
/// reachable endpoints, and this is the one place their CDR call is made.
#[cfg(feature = "ssr")]
async fn management_get(path: &str) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session().await?;
    let state: crate::state::AppState = expect_context();
    let url = state.cdr.management_url(path);
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    if response.is(http::StatusCode::NOT_FOUND) {
        return Ok(None);
    }
    Ok(Some(crate::cdr::CdrClient::expect_success(response)?.body))
}

#[cfg(test)]
mod tests {
    use super::{
        BuildInfoView, ManagementAvailability, MetricDetailView, MetricSample,
        availability_of_status, build_info_view, falls_back_to_public_readiness, format_metric,
        logger_view, metric_detail_view, metric_total, readiness_view, renders_management_ops,
    };
    use crate::error::ViewerError;

    #[test]
    fn only_a_404_means_the_surface_is_absent() {
        assert_eq!(
            availability_of_status(http::StatusCode::OK),
            ManagementAvailability::Available
        );
        assert_eq!(
            availability_of_status(http::StatusCode::NOT_FOUND),
            ManagementAvailability::Disabled
        );
        // Mounted but refused (Private/AdminOnly), and mounted but without its
        // telemetry facility, are both the surface EXISTING.
        for status in [
            http::StatusCode::UNAUTHORIZED,
            http::StatusCode::FORBIDDEN,
            http::StatusCode::SERVICE_UNAVAILABLE,
        ] {
            assert_eq!(
                availability_of_status(status),
                ManagementAvailability::Available,
                "{status}"
            );
        }
    }

    #[test]
    fn only_an_available_surface_renders_the_operations_panel() {
        // The probe-and-hide contract, asserted on the one predicate the nav
        // entry calls: with the CDR running `management.enabled = false` the
        // entry is not rendered at all — nor when the probe itself failed
        // (an unreachable management listener, an expired session).
        assert!(renders_management_ops(&Ok(
            ManagementAvailability::Available
        )));
        assert!(!renders_management_ops(&Ok(
            ManagementAvailability::Disabled
        )));
        assert!(!renders_management_ops(&Err(ViewerError::Unauthenticated)));
        assert!(!renders_management_ops(&Err(ViewerError::CdrUnreachable(
            "connection refused".to_owned()
        ))));
        assert!(ManagementAvailability::Available.usable());
        assert!(!ManagementAvailability::Disabled.usable());
    }

    #[test]
    fn readiness_rows_are_name_sorted_and_carry_the_detail() {
        // The CDR's `/management/health` body shape (AggregateHealth): the
        // aggregate plus one entry per indicator, `detail` only when the
        // indicator gave one.
        let body = serde_json::json!({
            "status": "DEGRADED",
            "components": {
                "migrations": { "status": "UP" },
                "audit": { "status": "DOWN", "detail": "sender queue full" },
                "db": { "status": "UP" }
            }
        });
        let view = readiness_view(&body, true);
        assert!(view.detailed);
        assert_eq!(view.status, "DEGRADED");
        let names: Vec<&str> = view.components.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec!["audit", "db", "migrations"]);
        let audit = &view.components[0];
        assert_eq!(audit.status, "DOWN");
        assert_eq!(audit.detail, "sender queue full");
        assert!(view.components[1].detail.is_empty());
    }

    #[test]
    fn a_readiness_body_without_the_expected_fields_still_renders() {
        let view = readiness_view(&serde_json::json!({}), false);
        assert_eq!(
            view,
            super::ReadinessView {
                status: "UNKNOWN".to_owned(),
                components: Vec::new(),
                detailed: false,
            }
        );
    }

    #[test]
    fn the_public_readiness_body_renders_statuses_without_detail() {
        // The CDR's public `/health/readiness` body (HealthSummary): names and
        // statuses, no detail.
        let body = serde_json::json!({
            "status": "UP",
            "components": { "db": { "status": "UP" }, "migrations": { "status": "UP" } }
        });
        let view = readiness_view(&body, false);
        assert!(!view.detailed);
        assert_eq!(view.components.len(), 2);
        assert!(view.components.iter().all(|c| c.detail.is_empty()));
    }

    #[test]
    fn only_an_absent_or_refused_management_health_falls_back() {
        assert!(falls_back_to_public_readiness(http::StatusCode::NOT_FOUND));
        assert!(falls_back_to_public_readiness(http::StatusCode::FORBIDDEN));
        for status in [
            http::StatusCode::OK,
            http::StatusCode::SERVICE_UNAVAILABLE,
            http::StatusCode::UNAUTHORIZED,
        ] {
            assert!(!falls_back_to_public_readiness(status), "{status}");
        }
    }

    #[test]
    fn build_info_lifts_the_spec_pins_into_their_own_section() {
        // The CDR's `BuildInfo` shape (telemetry::build_info): scalar facts
        // plus a nested `spec` object of pinned specification versions.
        let body = serde_json::json!({
            "name": "ferroehr",
            "version": "3.10.0",
            "git_sha": "abc123",
            "build_date": "2026-07-25T10:00:00Z",
            "rustc": "1.96.1",
            "postgres_target": "18",
            "spec": {
                "its_rest": "1.1.0",
                "rm": "1.2.0",
                "am": "1.4.0 + 2.4.0"
            }
        });
        let BuildInfoView { facts, spec } = build_info_view(&body);
        assert_eq!(
            facts,
            vec![
                ("build_date".to_owned(), "2026-07-25T10:00:00Z".to_owned()),
                ("git_sha".to_owned(), "abc123".to_owned()),
                ("name".to_owned(), "ferroehr".to_owned()),
                ("postgres_target".to_owned(), "18".to_owned()),
                ("rustc".to_owned(), "1.96.1".to_owned()),
                ("version".to_owned(), "3.10.0".to_owned()),
            ]
        );
        // The nested object is NOT a fact row (only scalars are), and its own
        // leaves land in `spec`, key-sorted.
        assert_eq!(
            spec,
            vec![
                ("am".to_owned(), "1.4.0 + 2.4.0".to_owned()),
                ("its_rest".to_owned(), "1.1.0".to_owned()),
                ("rm".to_owned(), "1.2.0".to_owned()),
            ]
        );
    }

    #[test]
    fn metric_detail_flattens_labels_and_keeps_every_sample() {
        // The CDR's actuator-style detail shape (management::metrics).
        let body = serde_json::json!({
            "name": "db_pool_connections",
            "metric_type": "gauge",
            "help": "pooled connections by state",
            "measurements": [
                { "labels": { "state": "idle" }, "value": 2.0 },
                { "labels": { "state": "in_use" }, "value": 3.0 },
                { "value": 1.0 }
            ]
        });
        let detail = metric_detail_view(&body);
        assert_eq!(detail.name, "db_pool_connections");
        assert_eq!(detail.kind, "gauge");
        assert_eq!(detail.help, "pooled connections by state");
        assert_eq!(detail.samples.len(), 3);
        assert_eq!(detail.samples[1].labels, "state=in_use");
        // An unlabelled sample keeps an empty label string, never a fabricated one.
        assert_eq!(detail.samples[2].labels, String::new());
    }

    #[test]
    fn metric_total_sums_all_or_only_the_filtered_samples() {
        let detail = MetricDetailView {
            name: "db_pool_connections".to_owned(),
            kind: "gauge".to_owned(),
            help: String::new(),
            samples: vec![
                MetricSample {
                    labels: "state=idle".to_owned(),
                    value: 2.0,
                },
                MetricSample {
                    labels: "state=in_use".to_owned(),
                    value: 3.0,
                },
                MetricSample {
                    labels: "http_route=/ehr, state=in_use".to_owned(),
                    value: 4.0,
                },
            ],
        };
        assert_eq!(metric_total(&detail, None), Some(9.0));
        assert_eq!(metric_total(&detail, Some(("state", "in_use"))), Some(7.0));
        // Nothing matched is ABSENT, never a fabricated zero.
        assert_eq!(metric_total(&detail, Some(("state", "broken"))), None);
        assert_eq!(
            metric_total(
                &MetricDetailView {
                    samples: Vec::new(),
                    ..detail
                },
                None
            ),
            None
        );
    }

    #[test]
    fn tile_values_drop_a_zero_fraction_and_keep_a_real_one() {
        assert_eq!(format_metric(0.0), "0");
        assert_eq!(format_metric(41_237.0), "41237");
        // A real fraction keeps two decimals (0.75 and 1.5 are exact in binary
        // floating point, so this pins the format, not a rounding mode).
        assert_eq!(format_metric(0.75), "0.75");
        assert_eq!(format_metric(1.5), "1.50");
    }

    #[test]
    fn logger_view_reads_the_effective_and_boot_filters() {
        let view = logger_view(&serde_json::json!({
            "filter": "ferroehr=debug,sqlx=warn",
            "boot_filter": "info"
        }));
        assert_eq!(view.filter, "ferroehr=debug,sqlx=warn");
        assert_eq!(view.boot_filter, "info");
        // A body missing a field renders empty, never a panic.
        assert_eq!(
            logger_view(&serde_json::json!({})),
            super::LoggerView {
                filter: String::new(),
                boot_filter: String::new(),
            }
        );
    }
}
