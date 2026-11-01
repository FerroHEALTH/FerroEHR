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
//! Every read guards with
//! [`require_session`](crate::session::require_session) first and keeps the
//! CDR credential server-side.

#![expect(
    clippy::disallowed_types,
    reason = "the viewer consumes the CDR JSON wire over ITS-REST — not the CDR internal seams \
              (#1694)"
)]

use ferrohealth_health::document::Readiness;

use crate::error::ViewerError;
use crate::locale::{Msg, tr};
use crate::locale::{tr_served, tr_served_with};

/// One headline metric: its name, its tile label, and an optional
/// `(label, value)` sample filter.
type HeadlineMetric = (&'static str, Msg, Option<(&'static str, &'static str)>);

/// The headline metric tiles, in display order.
///
/// Each is a counter or gauge the CDR registers
/// (`ferroehr::telemetry::metrics`); a histogram is deliberately absent, because
/// the CDR's detail view folds a histogram's `_bucket`/`_sum`/`_count` lines
/// into one sample list and summing it would report a meaningless number.
// NOTE: these are the Prometheus exporter's RENDERED names — `_total` is derived
// from the counter kind, never written on the instrument — and the correspondence
// is pinned by the CDR's `exporter_renders_the_viewer_metric_names` test.
const HEADLINE_METRICS: [HeadlineMetric; 3] = [
    (
        "compositions_committed_total",
        Msg::CompositionsCommitted,
        None,
    ),
    ("aql_queries_total", Msg::AqlQueries, None),
    (
        "db_pool_connections",
        Msg::DbConnectionsInUse,
        Some(("state", "in_use")),
    ),
];

/// Whether the CDR serves its management surface at the configured base URL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

// ── Health: the dependency-health indicators ────────────────────────────────

/// The CDR's readiness document and where it was read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadinessView {
    /// The family readiness document, its indicators sorted by name.
    pub readiness: Readiness,
    /// Whether the indicators carry their detail: `true` when read from
    /// `/management/health`, `false` from the public probe.
    pub detailed: bool,
}

/// Reads a `/management/health` or `/health/readiness` body into a
/// [`ReadinessView`].
///
/// Both carry the family readiness document; only the management body adds
/// each indicator's `detail`. The indicators are sorted by name so every draw
/// agrees.
///
/// # Errors
/// The parse error when `body` is not the readiness document.
pub fn readiness_view(body: &str, detailed: bool) -> Result<ReadinessView, serde_json::Error> {
    let document = serde_json::from_str::<Readiness>(body)?;
    Ok(ReadinessView {
        readiness: Readiness::new(document.phase, document.indicators),
        detailed,
    })
}

/// Whether a `/management/health` answer falls back to the public probe.
///
/// The fallback is the status-only probe, taken on `404` (the endpoint is not
/// mounted) or `403` (its access level refuses this session's role). A `401`
/// is not a fallback: the CDR no longer accepts the session, which the screen
/// reports.
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
pub async fn fetch_readiness() -> Result<ReadinessView, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
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
    readiness_view(&body, is_detailed).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::ReadinessJson,
            &[("e", (e).to_string().into())],
        ))
    })
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
pub async fn probe_management_api() -> Result<ManagementAvailability, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.management_url("info");
    let response = state
        .cdr
        .get(&session.credential, &url, "application/json")
        .await?;
    Ok(availability_of_status(response.status))
}

// ── Management: build + spec provenance ─────────────────────────────────────

/// The `GET /management/info` body as rendered rows.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BuildInfoView {
    /// Top-level non-object members (`version`, `git_sha`, `rustc`, …) as
    /// `(key, text)`, key-sorted.
    pub facts: Vec<(String, String)>,
    /// One section per top-level object member (`spec`, `manufacturer`,
    /// `support`, …), key-sorted.
    pub sections: Vec<InfoSection>,
}

/// One nested object of `/management/info`, rendered one level deep.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct InfoSection {
    /// The member name in the body, unique among the sections.
    pub key: String,
    /// The object's members as `(key, text)`, key-sorted; a member that is
    /// itself an object or an array is shown as its compact JSON.
    pub rows: Vec<(String, String)>,
}

/// Distils a `/management/info` body into a [`BuildInfoView`].
///
/// Generic over the field set on purpose: the CDR owns that shape and may add
/// members, so every top-level member is rendered rather than a fixed list.
/// Non-object members become fact rows; each object member (`spec`,
/// `manufacturer`, `support`, `audit`, …) becomes its own section, one level
/// deep. Every list is key-sorted, so the render order is deterministic.
#[must_use]
pub fn build_info_view(body: &serde_json::Value) -> BuildInfoView {
    let mut view = BuildInfoView::default();
    let Some(object) = body.as_object() else {
        return view;
    };
    for (key, value) in object {
        match value {
            serde_json::Value::Object(members) => view.sections.push(InfoSection {
                key: key.clone(),
                rows: leaf_rows(members),
            }),
            other => view.facts.push((key.clone(), leaf_text(other))),
        }
    }
    view.facts.sort();
    view.sections.sort_by(|a, b| a.key.cmp(&b.key));
    view
}

/// Every member of a JSON object as `(key, text)`, key-sorted.
fn leaf_rows(object: &serde_json::Map<String, serde_json::Value>) -> Vec<(String, String)> {
    let mut rows: Vec<(String, String)> = object
        .iter()
        .map(|(key, value)| (key.clone(), leaf_text(value)))
        .collect();
    rows.sort();
    rows
}

/// One JSON value as display text: a string verbatim, a number or boolean as
/// written, `null` as an em dash, and an array or object as compact JSON.
fn leaf_text(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Null => tr(Msg::NoValue),
        other => other.to_string(),
    }
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
pub async fn fetch_build_info() -> Result<Option<BuildInfoView>, ViewerError> {
    let Some(body) = management_get("info").await? else {
        return Ok(None);
    };
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::ManagementInfoJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    Ok(Some(build_info_view(&value)))
}

// ── Management: metrics ─────────────────────────────────────────────────────

/// One headline metric tile.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MetricTile {
    /// The metric name (the browser's selection value).
    pub name: String,
    /// The human label.
    pub label: String,
    /// The formatted value, or `—` when the CDR registers no such metric.
    pub value: String,
}

/// One sample of a metric: its labels, flattened for display, and its value.
#[derive(Debug, Clone, PartialEq)]
pub struct MetricSample {
    /// The sample's labels as `k=v, k2=v2` (empty for an unlabelled sample).
    pub labels: String,
    /// The sample value.
    pub value: f64,
}

/// The `GET /management/metrics/{name}` body as rendered rows.
#[derive(Debug, Clone, PartialEq)]
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
pub async fn fetch_metric_names() -> Result<Option<Vec<String>>, ViewerError> {
    let Some(body) = management_get("metrics").await? else {
        return Ok(None);
    };
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::MetricListJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
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
pub async fn fetch_metric_detail(
    // The metric name to read, as listed by the metrics index.
    name: String,
) -> Result<Option<MetricDetailView>, ViewerError> {
    if name.trim().is_empty() {
        return Err(ViewerError::Invalid(tr_served(Msg::NoMetricNameGiven)));
    }
    let path = format!("metrics/{}", urlencoding::encode(&name));
    let Some(body) = management_get(&path).await? else {
        return Ok(None);
    };
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::MetricDetailJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
    Ok(Some(metric_detail_view(&value)))
}

/// The headline metric tiles.
///
/// The registry's names, then one detail read per `HEADLINE_METRICS` entry
/// the CDR actually registers (so a tile never provokes a `404`, and a
/// metric the deployment does not record renders `—`).
///
/// `Ok(None)` = the metrics endpoint is not mounted.
///
/// # Errors
/// As [`fetch_build_info`].
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
                .map_or_else(|| tr(Msg::NoValue), format_metric)
        } else {
            tr(Msg::NoValue)
        };
        tiles.push(MetricTile {
            name: name.to_owned(),
            label: tr_served(label),
            value,
        });
    }
    Ok(Some(tiles))
}

// ── Management: the live log filter ─────────────────────────────────────────

/// The `GET /management/loggers` body: the effective filter and the boot filter
/// a reset restores.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
pub async fn fetch_loggers() -> Result<Option<LoggerView>, ViewerError> {
    let Some(body) = management_get("loggers").await? else {
        return Ok(None);
    };
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::LoggersJson,
            &[("e", (e).to_string().into())],
        ))
    })?;
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
pub async fn set_log_filter(
    // The replacement log filter, one or more `target=level` directives.
    filter: String,
) -> Result<LoggerView, ViewerError> {
    let session = crate::session::require_session()?;
    let filter = filter.trim().to_owned();
    if filter.is_empty() {
        return Err(ViewerError::Invalid(tr_served(
            Msg::LogFilterNeedsLeastOneDirective,
        )));
    }
    let state = crate::scope::state()?;
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
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::LoggersJson2,
            &[("e", (e).to_string().into())],
        ))
    })?;
    Ok(logger_view(&value))
}

/// Restore the CDR's boot log filter (`DELETE /management/loggers`).
///
/// # Errors
/// As [`set_log_filter`], minus the empty-input case.
pub async fn reset_log_filter() -> Result<LoggerView, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
    let url = state.cdr.management_url("loggers");
    let response = state.cdr.delete(&session.credential, &url, &[]).await?;
    let body = crate::cdr::CdrClient::expect_success(response)?.body;
    let value = serde_json::from_str::<serde_json::Value>(&body).map_err(|e| {
        ViewerError::Internal(tr_served_with(
            Msg::LoggersJson3,
            &[("e", (e).to_string().into())],
        ))
    })?;
    Ok(logger_view(&value))
}

/// GET one management endpoint as the session's credential, mapping the CDR's
/// `404` to `Ok(None)` — the "endpoint not mounted" state every management read
/// renders as a first-class absence rather than an error.
///
/// Guards the viewer session first: the pages reading through the reads
/// above are publicly reachable endpoints, and this is the one place their CDR call is made.
async fn management_get(path: &str) -> Result<Option<String>, ViewerError> {
    let session = crate::session::require_session()?;
    let state = crate::scope::state()?;
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
        BuildInfoView, InfoSection, ManagementAvailability, MetricDetailView, MetricSample,
        availability_of_status, build_info_view, falls_back_to_public_readiness, format_metric,
        logger_view, metric_detail_view, metric_total, readiness_view, renders_management_ops,
    };
    use crate::error::ViewerError;
    use crate::locale::{Msg, tr};
    use ferrohealth_health::document::State;

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
        // The CDR's `/management/health` body: the family readiness document,
        // `detail` only when the indicator gave one.
        let body = serde_json::json!({
            "state": "degraded",
            "phase": "serving",
            "indicators": [
                { "name": "migrations", "state": "up" },
                { "name": "audit", "state": "degraded", "detail": "sender queue full" },
                { "name": "db", "state": "up" }
            ]
        })
        .to_string();
        let view = readiness_view(&body, true).expect("the readiness document");
        assert!(view.detailed);
        assert_eq!(view.readiness.state, State::Degraded);
        let names: Vec<&str> = view
            .readiness
            .indicators
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert_eq!(names, vec!["audit", "db", "migrations"]);
        let audit = &view.readiness.indicators[0];
        assert_eq!(audit.state, State::Degraded);
        assert_eq!(audit.detail.as_deref(), Some("sender queue full"));
        assert_eq!(view.readiness.indicators[1].detail, None);
    }

    #[test]
    fn a_body_that_is_not_the_readiness_document_is_refused() {
        assert!(readiness_view("{}", false).is_err());
        assert!(readiness_view(r#"{"status":"UP","components":{}}"#, false).is_err());
    }

    #[test]
    fn the_public_readiness_body_renders_states_without_detail() {
        // The CDR's public `/health/readiness` body: names and states, no
        // detail.
        let body = serde_json::json!({
            "state": "up",
            "phase": "serving",
            "indicators": [
                { "name": "db", "state": "up" },
                { "name": "migrations", "state": "up" }
            ]
        })
        .to_string();
        let view = readiness_view(&body, false).expect("the readiness document");
        assert!(!view.detailed);
        assert_eq!(view.readiness.indicators.len(), 2);
        assert!(view.readiness.indicators.iter().all(|c| c.detail.is_none()));
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
        let BuildInfoView { facts, sections } = build_info_view(&body);
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
        // The nested object is NOT a fact row, and its own leaves land in the
        // `spec` section, key-sorted.
        assert_eq!(
            sections,
            vec![InfoSection {
                key: "spec".to_owned(),
                rows: vec![
                    ("am".to_owned(), "1.4.0 + 2.4.0".to_owned()),
                    ("its_rest".to_owned(), "1.1.0".to_owned()),
                    ("rm".to_owned(), "1.2.0".to_owned()),
                ],
            }]
        );
    }

    /// Every nested object of the body becomes its own key-sorted section,
    /// one level deep: the manufacturer the CDR serializes (the shared
    /// `manufacturer.rs` value, so the fixture is the server's shape), the
    /// support statement with its `null` dates, and a deeper object shown as
    /// compact JSON rather than dropped.
    #[test]
    fn build_info_renders_every_nested_object_one_level_deep() {
        let body = serde_json::json!({
            "version": "4.3.6",
            "features": ["fhir", "events"],
            "manufacturer": serde_json::to_value(crate::manufacturer::MANUFACTURER)
                .expect("the manufacturer serializes"),
            "support": {"release_date": null, "support_ends": null, "status": "unreleased"},
            "audit": {"enabled": true, "category_map": {"sha256": "ab12"}},
            "spec": {"rm": "1.2.0"}
        });
        let BuildInfoView { facts, sections } = build_info_view(&body);
        assert_eq!(
            facts,
            vec![
                ("features".to_owned(), r#"["fhir","events"]"#.to_owned()),
                ("version".to_owned(), "4.3.6".to_owned()),
            ]
        );
        let keys: Vec<&str> = sections.iter().map(|s| s.key.as_str()).collect();
        assert_eq!(keys, ["audit", "manufacturer", "spec", "support"]);
        let section = |key: &str| {
            sections
                .iter()
                .find(|s| s.key == key)
                .expect("the section is present")
        };
        assert_eq!(
            section("manufacturer").rows,
            vec![
                ("email".to_owned(), "info@cadasto.com".to_owned()),
                ("name".to_owned(), "Cadasto B.V.".to_owned()),
                (
                    "postal_address".to_owned(),
                    "Comeniusstraat 2d, 1817 MS Alkmaar, The Netherlands".to_owned()
                ),
                (
                    "website".to_owned(),
                    "https://www.cadasto.com/contact/".to_owned()
                ),
            ]
        );
        assert_eq!(
            section("support").rows,
            vec![
                ("release_date".to_owned(), tr(Msg::NoValue)),
                ("status".to_owned(), "unreleased".to_owned()),
                ("support_ends".to_owned(), tr(Msg::NoValue)),
            ]
        );
        assert_eq!(
            section("audit").rows,
            vec![
                ("category_map".to_owned(), r#"{"sha256":"ab12"}"#.to_owned()),
                ("enabled".to_owned(), "true".to_owned()),
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
