// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The per-request HTTP observability middleware.
//!
//! Two `axum` middlewares, applied to the API router beside the ATNA audit
//! layer:
//!
//! - [`root_span`] — the root span-maker installed over `tower-http`'s default.
//!   Names the root span by route template (`MatchedPath`), records the
//!   `OTel` HTTP semantic-convention attributes plus `request_id`, extracts the
//!   W3C `traceparent` on ingress, and (only when the `OTel` export layer is
//!   installed) records `trace_id`/`span_id` on the span and returns the
//!   `x-trace-id` response header. Cardinality-safe: the raw path with ids
//!   never enters the span *name*.
//! - [`http_metrics`] — records the HTTP metric family over the `metrics`
//!   facade: request duration by `(http_route, http_request_method,
//!   status_class)`, an active-requests gauge, and request/response body sizes,
//!   all keyed by the route *template* only. The same template, duration and
//!   status feed the usage report's window
//!   ([`ferroehr::usage_report::window::record_request`]).
//!
//! **PHI rule:** every label value here is a closed set — route
//! templates, method, status class. No ids ever become a label.

use std::net::SocketAddr;
use std::time::Instant;

use axum::extract::{ConnectInfo, MatchedPath, Request};
use axum::middleware::Next;
use axum::response::Response;
use http::{HeaderMap, HeaderValue, StatusCode, header};
use opentelemetry::propagation::Extractor;
use opentelemetry::trace::TraceContextExt;
use tracing::Instrument;
use tracing_opentelemetry::OpenTelemetrySpanExt;

// ── Metric names — the single source of truth for the emitters here and
//    the bucket-ladder / description registration in `ferroehr::telemetry`. ────

/// The label value used when a request did not match any route template (the
/// fallback that keeps the `http_route` label a closed set).
const UNMATCHED_ROUTE: &str = "unmatched";

/// The response header carrying the current trace id for support correlation.
const X_TRACE_ID: &str = "x-trace-id";

/// Root-span middleware. Creates the per-request span, propagates W3C
/// trace context, and stamps `x-trace-id` when tracing is active.
pub async fn root_span(req: Request, next: Next) -> Response {
    let method = req.method().clone();
    let route = matched_route(&req);
    let user_agent = header_str(req.headers(), header::USER_AGENT.as_str()).unwrap_or_default();
    let request_id = header_str(req.headers(), "x-request-id").unwrap_or_default();
    let client_address = client_address(&req).unwrap_or_default();

    // Extract any inbound W3C trace context (the propagator is installed by the
    // binary only when OTel is enabled; absent it is a no-op empty context).
    let parent_cx = opentelemetry::global::get_text_map_propagator(|propagator| {
        propagator.extract(&HeaderExtractor(req.headers()))
    });

    let span = tracing::info_span!(
        "http_request",
        otel.name = tracing::field::Empty,
        "http.request.method" = %method,
        "http.route" = %route,
        "user_agent.original" = %user_agent,
        "client.address" = %client_address,
        "http.response.status_code" = tracing::field::Empty,
        request_id = %request_id,
        trace_id = tracing::field::Empty,
        span_id = tracing::field::Empty,
    );
    // The low-cardinality OTel span display name: method + route template.
    span.record("otel.name", format!("{method} {route}").as_str());
    if let Err(e) = span.set_parent(parent_cx) {
        tracing::trace!("could not attach inbound trace context: {e}");
    }

    let recorder = span.clone();
    async move {
        // Read the OTel span context assigned by the export layer. When the
        // layer is not installed the context is invalid (all-zero) → no header,
        // no trace_id field.
        let cx = recorder.context();
        let span_context = cx.span().span_context().clone();
        let tracing_active = span_context.is_valid();
        if tracing_active {
            recorder.record("trace_id", span_context.trace_id().to_string().as_str());
            recorder.record("span_id", span_context.span_id().to_string().as_str());
        }

        let mut resp = next.run(req).await;
        recorder.record("http.response.status_code", resp.status().as_u16());

        if tracing_active
            && let Ok(value) = HeaderValue::from_str(&span_context.trace_id().to_string())
        {
            resp.headers_mut().insert(X_TRACE_ID, value);
        }
        resp
    }
    .instrument(span)
    .await
}

/// HTTP metrics middleware.
pub async fn http_metrics(req: Request, next: Next) -> Response {
    let route = matched_route(&req);
    let method = req.method().as_str().to_owned();
    let request_body = content_length(req.headers());

    let route_kv = [opentelemetry::KeyValue::new("http_route", route.clone())];
    ferroehr::telemetry::metrics::metrics()
        .http_active_requests
        .add(1, &route_kv);
    let started = Instant::now();

    let resp = next.run(req).await;

    let elapsed = started.elapsed();
    ferroehr::usage_report::window::record_request(&route, elapsed, resp.status());
    let elapsed = elapsed.as_secs_f64();
    ferroehr::telemetry::metrics::metrics()
        .http_active_requests
        .add(-1, &route_kv);

    ferroehr::telemetry::metrics::metrics()
        .http_request_duration
        .record(
            elapsed,
            &[
                opentelemetry::KeyValue::new("http_route", route.clone()),
                opentelemetry::KeyValue::new("http_request_method", method),
                opentelemetry::KeyValue::new("status_class", status_class(resp.status())),
            ],
        );

    if let Some(size) = request_body {
        ferroehr::telemetry::metrics::metrics()
            .http_request_body_size
            .record(size, &route_kv);
    }
    if let Some(size) = content_length(resp.headers()) {
        ferroehr::telemetry::metrics::metrics()
            .http_response_body_size
            .record(size, &route_kv);
    }
    resp
}

/// The matched route template (`MatchedPath`), or [`UNMATCHED_ROUTE`].
fn matched_route(req: &Request) -> String {
    req.extensions()
        .get::<MatchedPath>()
        .map_or_else(|| UNMATCHED_ROUTE.to_owned(), |m| m.as_str().to_owned())
}

/// The HTTP status class label value (`2xx`…`5xx`).
#[expect(
    clippy::integer_division,
    reason = "the fold IS the truncating divide: 100..=199 → 1, 200..=299 → 2, … is \
              exactly the HTTP status-class label"
)]
fn status_class(status: StatusCode) -> &'static str {
    match status.as_u16() / 100 {
        1 => "1xx",
        2 => "2xx",
        3 => "3xx",
        4 => "4xx",
        _ => "5xx",
    }
}

/// The `Content-Length` of a message, in bytes, if declared.
///
/// A byte count is integral, and the histogram takes `u64`, so there is no cast
/// and nothing to lose.
fn content_length(headers: &HeaderMap) -> Option<u64> {
    headers
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
}

/// A header value as an owned string, if present and valid UTF-8.
fn header_str(headers: &HeaderMap, name: &str) -> Option<String> {
    headers
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned)
}

/// The client network address: `X-Forwarded-For` first hop, else the TCP peer.
fn client_address(req: &Request) -> Option<String> {
    if let Some(first) = req
        .headers()
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|xff| xff.split(',').next())
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(first.to_owned());
    }
    req.extensions()
        .get::<ConnectInfo<SocketAddr>>()
        .map(|ci| ci.0.ip().to_string())
}

/// Adapts an HTTP [`HeaderMap`] to the `OTel` propagation [`Extractor`] trait so
/// the `TraceContextPropagator` can read `traceparent`/`tracestate` on ingress.
struct HeaderExtractor<'a>(&'a HeaderMap);

impl Extractor for HeaderExtractor<'_> {
    fn get(&self, key: &str) -> Option<&str> {
        self.0.get(key).and_then(|v| v.to_str().ok())
    }

    fn keys(&self) -> Vec<&str> {
        self.0.keys().map(http::HeaderName::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_classes() {
        assert_eq!(status_class(StatusCode::OK), "2xx");
        assert_eq!(status_class(StatusCode::NOT_FOUND), "4xx");
        assert_eq!(status_class(StatusCode::INTERNAL_SERVER_ERROR), "5xx");
        assert_eq!(status_class(StatusCode::MOVED_PERMANENTLY), "3xx");
        assert_eq!(status_class(StatusCode::CONTINUE), "1xx");
    }

    #[test]
    fn content_length_parses() {
        let mut h = HeaderMap::new();
        h.insert(header::CONTENT_LENGTH, HeaderValue::from_static("1234"));
        assert_eq!(content_length(&h), Some(1234));
        assert_eq!(content_length(&HeaderMap::new()), None);
    }

    #[test]
    fn xff_first_hop_wins() {
        let mut req = Request::new(axum::body::Body::empty());
        req.headers_mut().insert(
            "x-forwarded-for",
            HeaderValue::from_static("203.0.113.7, 10.0.0.1"),
        );
        assert_eq!(client_address(&req).as_deref(), Some("203.0.113.7"));
    }
}
