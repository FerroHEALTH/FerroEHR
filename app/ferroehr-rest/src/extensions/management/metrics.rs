// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `GET /management/prometheus` and `GET /management/metrics[/{name}]`.
//!
//! `prometheus` renders the exposition text scraped by Prometheus.
//! `metrics` gives an actuator-style JSON view (the registry's metric names)
//! and `metrics/{name}` the per-metric samples — parsed from the same
//! exposition text so the two views can never disagree.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 8): genuinely open operational JSON (config \
              dump, management env, validity-checker input, OpenAPI schema literals)"
)]

use std::collections::BTreeMap;

use axum::Json;
use axum::extract::Path;
use axum::response::{IntoResponse, Response};
use http::{HeaderValue, StatusCode, header};
use serde::Serialize;

/// The Prometheus text-exposition content type.
const PROM_CONTENT_TYPE: &str = "text/plain; version=0.0.4; charset=utf-8";

/// `GET /management/prometheus`.
pub(super) fn prometheus(registry: &::prometheus::Registry) -> Response {
    let Ok(body) = ferroehr::telemetry::metrics::render(registry) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "metrics could not be rendered",
        )
            .into_response();
    };
    (
        StatusCode::OK,
        [(
            header::CONTENT_TYPE,
            HeaderValue::from_static(PROM_CONTENT_TYPE),
        )],
        body,
    )
        .into_response()
}

/// The `GET /management/metrics` body: the set of registered metric names.
#[derive(Debug, Serialize)]
pub(super) struct MetricNames {
    names: Vec<String>,
}

/// The `GET /management/metrics/{name}` body: a metric's samples.
#[derive(Debug, Serialize)]
pub(super) struct MetricDetail {
    name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    metric_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    help: Option<String>,
    measurements: Vec<Measurement>,
}

/// One sample line (labels + value).
#[derive(Debug, Serialize)]
pub(super) struct Measurement {
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    labels: BTreeMap<String, String>,
    value: f64,
}

/// `GET /management/metrics`.
pub(super) fn list(registry: &::prometheus::Registry) -> Json<MetricNames> {
    let text = ferroehr::telemetry::metrics::render(registry).unwrap_or_default();
    let mut names: Vec<String> = base_names(&text).into_iter().collect();
    names.sort();
    Json(MetricNames { names })
}

/// `GET /management/metrics/{name}`.
pub(super) fn detail(registry: &::prometheus::Registry, Path(name): Path<String>) -> Response {
    let text = ferroehr::telemetry::metrics::render(registry).unwrap_or_default();
    let (metric_type, help, measurements) = samples_for(&text, &name);
    if measurements.is_empty() && metric_type.is_none() && help.is_none() {
        return (
            StatusCode::NOT_FOUND,
            Json(serde_json::json!({ "error": format!("no such metric: {name}") })),
        )
            .into_response();
    }
    Json(MetricDetail {
        name,
        metric_type,
        help,
        measurements,
    })
    .into_response()
}

/// The logical (base) metric names present in exposition `text`: the names on
/// `# TYPE` lines, plus histogram/summary base names with their `_bucket`/
/// `_sum`/`_count` suffixes folded away.
fn base_names(text: &str) -> std::collections::BTreeSet<String> {
    let mut names = std::collections::BTreeSet::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# TYPE ") {
            if let Some(name) = rest.split_whitespace().next() {
                names.insert(name.to_owned());
            }
        } else if !line.starts_with('#')
            && !line.trim().is_empty()
            && let Some(name) = metric_name(line)
        {
            names.insert(fold_suffix(name).to_owned());
        }
    }
    names
}

/// Extract the `(type, help, samples)` for a base metric name.
fn samples_for(text: &str, base: &str) -> (Option<String>, Option<String>, Vec<Measurement>) {
    let mut metric_type = None;
    let mut help = None;
    let mut measurements = Vec::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("# TYPE ") {
            let mut it = rest.split_whitespace();
            if it.next() == Some(base) {
                metric_type = it.next().map(str::to_owned);
            }
        } else if let Some(rest) = line.strip_prefix("# HELP ") {
            let mut it = rest.splitn(2, char::is_whitespace);
            if it.next() == Some(base) {
                help = it.next().map(str::to_owned);
            }
        } else if !line.starts_with('#')
            && !line.trim().is_empty()
            && let Some(name) = metric_name(line)
            && fold_suffix(name) == base
            && let Some(m) = parse_sample(line)
        {
            measurements.push(m);
        }
    }
    (metric_type, help, measurements)
}

/// The metric name at the start of a sample line (up to `{` or whitespace).
fn metric_name(line: &str) -> Option<&str> {
    let name = line
        .find(['{', ' '])
        .and_then(|end| line.get(..end))
        .unwrap_or(line);
    (!name.is_empty()).then_some(name)
}

/// Fold a histogram/summary sample-name suffix to its base name.
fn fold_suffix(name: &str) -> &str {
    for suffix in ["_bucket", "_sum", "_count"] {
        if let Some(base) = name.strip_suffix(suffix) {
            return base;
        }
    }
    name
}

/// Parse one exposition sample line into labels + value.
fn parse_sample(line: &str) -> Option<Measurement> {
    let (name_labels, value) = line.rsplit_once(char::is_whitespace)?;
    let value: f64 = value.trim().parse().ok()?;
    let labels = name_labels
        .find('{')
        .and_then(|open| {
            let close = name_labels.rfind('}')?;
            name_labels.get(open + 1..close).map(parse_labels)
        })
        .unwrap_or_default();
    Some(Measurement { labels, value })
}

/// Parse a `k="v",k2="v2"` label block into a map.
fn parse_labels(block: &str) -> BTreeMap<String, String> {
    let mut labels = BTreeMap::new();
    for pair in block.split(',') {
        if let Some((k, v)) = pair.split_once('=') {
            let v = v.trim().trim_matches('"');
            labels.insert(k.trim().to_owned(), v.to_owned());
        }
    }
    labels
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"# HELP http_server_request_duration_seconds request duration
# TYPE http_server_request_duration_seconds histogram
http_server_request_duration_seconds_bucket{http_route="/ehr",le="0.1"} 3
http_server_request_duration_seconds_sum{http_route="/ehr"} 0.25
http_server_request_duration_seconds_count{http_route="/ehr"} 4
# TYPE db_pool_connections gauge
db_pool_connections{state="idle"} 2
"#;

    #[test]
    fn base_names_fold_histogram_suffixes() {
        let names = base_names(SAMPLE);
        assert!(names.contains("http_server_request_duration_seconds"));
        assert!(names.contains("db_pool_connections"));
        assert!(!names.contains("http_server_request_duration_seconds_bucket"));
    }

    #[test]
    fn samples_for_gauge_parses_labels_and_value() {
        let (ty, _help, ms) = samples_for(SAMPLE, "db_pool_connections");
        assert_eq!(ty.as_deref(), Some("gauge"));
        assert_eq!(ms.len(), 1);
        assert_eq!(ms[0].labels.get("state").map(String::as_str), Some("idle"));
        assert!((ms[0].value - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn samples_for_histogram_collects_all_lines() {
        let (ty, help, ms) = samples_for(SAMPLE, "http_server_request_duration_seconds");
        assert_eq!(ty.as_deref(), Some("histogram"));
        assert!(help.is_some());
        assert_eq!(ms.len(), 3); // bucket + sum + count
    }
}
