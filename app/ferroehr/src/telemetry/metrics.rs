// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The metrics system: ONE `OpenTelemetry` `MeterProvider` feeding both
//! surfaces.
//!
//! **No openEHR spec governs metrics — our own design/extension.**
//!
//! # Why one provider
//!
//! One [`SdkMeterProvider`] drives up to two readers: a Prometheus reader
//! serving the pull surface and, when `telemetry.metrics_push` is on, a periodic
//! OTLP reader. Every instrument reaches both by construction, so a family
//! cannot exist on one surface and not the other.
//!
//! # Naming
//!
//! Instrument names carry NO Prometheus suffix, and units are declared instead:
//! the exporter derives `_total` for a counter and `_seconds`/`_bytes` from the
//! unit, per the `OpenTelemetry`-to-Prometheus conventions. Writing
//! `auth_failures_total` here would render `auth_failures_total_total`.
//!
//! The `OpenTelemetry` metrics API and SDK are Stable upstream — a firmer
//! footing than the trace path this server already depends on, which is Beta
//! (<https://github.com/open-telemetry/opentelemetry-rust>).

use std::sync::OnceLock;

use opentelemetry::KeyValue;
use opentelemetry::metrics::{Counter, Histogram, Meter, UpDownCounter};
use opentelemetry_sdk::metrics::reader::MetricReader;
use opentelemetry_sdk::metrics::{Instrument, SdkMeterProvider, Stream, Temporality};

use crate::telemetry::build_info::BuildInfo;

/// The instrumentation scope every instrument is created under.
pub const SCOPE: &str = "ferroehr";

// ── Instrument names (no Prometheus suffix — the exporter adds it) ───────────

/// HTTP request duration (`http_route`, `http_request_method`, `status_class`).
pub const HTTP_REQUEST_DURATION: &str = "http_server_request_duration";
/// In-flight HTTP requests (`http_route`).
pub const HTTP_ACTIVE_REQUESTS: &str = "http_server_active_requests";
/// HTTP request body size (`http_route`).
pub const HTTP_REQUEST_BODY_SIZE: &str = "http_server_request_body_size";
/// HTTP response body size (`http_route`).
pub const HTTP_RESPONSE_BODY_SIZE: &str = "http_server_response_body_size";
/// Authentication failures (`mechanism`, `reason`).
pub const AUTH_FAILURES: &str = "auth_failures";
/// Cedar authorization decisions (`result`).
pub const AUTHZ_CEDAR_DECISIONS: &str = "authz_cedar_decisions";
/// Remote-PDP authorization calls (`result`).
pub const AUTHZ_REMOTE_PDP_CALLS: &str = "authz_remote_pdp_calls";
/// Database pool connections (`state`), sampled as a gauge.
pub const DB_POOL_CONNECTIONS: &str = "db_pool_connections";
/// Database connection-acquire latency.
pub const DB_POOL_ACQUIRE_DURATION: &str = "db_pool_acquire_duration";
/// Service transactions (`outcome`).
pub const DB_TRANSACTIONS: &str = "db_transactions";
/// AQL queries (`outcome`).
pub const AQL_QUERIES: &str = "aql_queries";
/// AQL query phase latency (`phase`).
pub const AQL_QUERY_DURATION: &str = "aql_query_duration";
/// AQL plan-cache events (`event`).
pub const AQL_PLAN_CACHE_EVENTS: &str = "aql_plan_cache_events";
/// Committed compositions (`change_type`, the numeric openEHR audit group code).
pub const COMPOSITIONS_COMMITTED: &str = "compositions_committed";
/// Validation failures (`pass`).
pub const VALIDATION_FAILURES: &str = "validation_failures";
/// Version-signature verification failures (`verdict`).
pub const VERSION_SIGNATURE_INVALID: &str = "version_signature_invalid";
/// `WebTemplate` cache events (`event`).
pub const WEBTEMPLATE_CACHE_EVENTS: &str = "webtemplate_cache_events";
/// Contribution-outbox events published to the broker.
pub const EVENTS_PUBLISHED: &str = "events_published";
/// ATNA audit records emitted.
pub const ATNA_AUDIT_EMITTED: &str = "atna_audit_emitted";
/// ATNA audit records dropped because the queue was full.
pub const ATNA_AUDIT_DROPPED: &str = "atna_audit_dropped";
/// ATNA audit records rejected as malformed.
pub const ATNA_AUDIT_REJECTED: &str = "atna_audit_rejected";
/// ATNA audit records that failed to serialize.
pub const ATNA_AUDIT_SERIALIZE_FAILED: &str = "atna_audit_serialize_failed";
/// ATNA audit records delivered to a sink (`sink`).
pub const ATNA_AUDIT_SENT: &str = "atna_audit_sent";
/// ATNA audit deliveries that failed (`sink`).
pub const ATNA_AUDIT_SEND_FAILED: &str = "atna_audit_send_failed";
/// ATNA audit records reaped by retention.
pub const ATNA_AUDIT_REAPED: &str = "atna_audit_reaped";
/// Process start time, unix seconds.
pub const PROCESS_START_TIME: &str = "process_start_time";
/// Build identity, always `1`, carrying `version`/`git_sha`/`rm_version`.
pub const BUILD_INFO: &str = "ferroehr_build_info";
/// Tokio worker threads.
pub const TOKIO_WORKERS: &str = "tokio_workers";
/// Tokio global queue depth.
pub const TOKIO_GLOBAL_QUEUE_DEPTH: &str = "tokio_global_queue_depth";
/// Tokio alive tasks.
pub const TOKIO_ALIVE_TASKS: &str = "tokio_alive_tasks";
/// Process resident set size (unit: bytes — the exporter appends `_bytes`,
/// yielding the standard Prometheus `process_resident_memory_bytes`).
pub const PROCESS_RESIDENT_MEMORY: &str = "process_resident_memory";

// ── Bucket ladders ───────────────────────────────────────────────────────────

/// HTTP request duration: 5 ms … 10 s.
pub const HTTP_DURATION_BUCKETS: &[f64] = &[
    0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0,
];
/// DB connection-acquire duration: 100 µs … 1 s.
pub const DB_ACQUIRE_BUCKETS: &[f64] = &[
    0.0001, 0.00025, 0.0005, 0.001, 0.0025, 0.005, 0.01, 0.025, 0.05, 0.1, 0.25, 0.5, 1.0,
];
/// AQL query duration: 1 ms … 30 s.
pub const AQL_DURATION_BUCKETS: &[f64] = &[
    0.001, 0.005, 0.01, 0.05, 0.1, 0.25, 0.5, 1.0, 2.5, 5.0, 10.0, 30.0,
];
/// HTTP body sizes: 64 B … 16 MiB.
pub const BODY_SIZE_BUCKETS: &[f64] = &[
    64.0,
    256.0,
    1024.0,
    4096.0,
    16384.0,
    65536.0,
    262_144.0,
    1_048_576.0,
    4_194_304.0,
    16_777_216.0,
];

/// Every histogram and its bucket ladder, in one place.
///
/// The ladders are attached as SDK views rather than left to the default
/// boundaries: `OTel`'s defaults are not ours, and a silently re-bucketed
/// latency histogram invalidates every dashboard and alert built on it.
#[must_use]
pub fn histogram_views() -> Vec<(&'static str, &'static [f64])> {
    vec![
        (HTTP_REQUEST_DURATION, HTTP_DURATION_BUCKETS),
        (HTTP_REQUEST_BODY_SIZE, BODY_SIZE_BUCKETS),
        (HTTP_RESPONSE_BODY_SIZE, BODY_SIZE_BUCKETS),
        (DB_POOL_ACQUIRE_DURATION, DB_ACQUIRE_BUCKETS),
        (AQL_QUERY_DURATION, AQL_DURATION_BUCKETS),
    ]
}

/// The instruments, built once and reachable from every emitting site.
///
/// A struct of typed handles rather than macro call sites: an instrument is
/// created once and recorded through a real type, so a misspelled metric name
/// is a compile error instead of a silently-new time series.
#[derive(Debug)]
pub struct Metrics {
    /// HTTP request duration, seconds.
    pub http_request_duration: Histogram<f64>,
    /// In-flight HTTP requests.
    pub http_active_requests: UpDownCounter<i64>,
    /// HTTP request body size, bytes.
    pub http_request_body_size: Histogram<u64>,
    /// HTTP response body size, bytes.
    pub http_response_body_size: Histogram<u64>,
    /// Authentication failures.
    pub auth_failures: Counter<u64>,
    /// Cedar authorization decisions.
    pub authz_cedar_decisions: Counter<u64>,
    /// Remote-PDP authorization calls.
    pub authz_remote_pdp_calls: Counter<u64>,
    /// Database connection-acquire latency, seconds.
    pub db_pool_acquire_duration: Histogram<f64>,
    /// Service transactions by outcome.
    pub db_transactions: Counter<u64>,
    /// AQL queries by outcome.
    pub aql_queries: Counter<u64>,
    /// AQL query phase latency, seconds.
    pub aql_query_duration: Histogram<f64>,
    /// AQL plan-cache events.
    pub aql_plan_cache_events: Counter<u64>,
    /// Committed compositions.
    pub compositions_committed: Counter<u64>,
    /// Validation failures by pass.
    pub validation_failures: Counter<u64>,
    /// Version-signature verification failures.
    pub version_signature_invalid: Counter<u64>,
    /// `WebTemplate` cache events.
    pub webtemplate_cache_events: Counter<u64>,
    /// Events published to the broker.
    pub events_published: Counter<u64>,
    /// ATNA audit records emitted.
    pub atna_audit_emitted: Counter<u64>,
    /// ATNA audit records dropped because the queue was full.
    pub atna_audit_dropped: Counter<u64>,
    /// ATNA audit records rejected as malformed.
    pub atna_audit_rejected: Counter<u64>,
    /// ATNA audit records that failed to serialize.
    pub atna_audit_serialize_failed: Counter<u64>,
    /// ATNA audit records delivered to a sink.
    pub atna_audit_sent: Counter<u64>,
    /// ATNA audit deliveries that failed.
    pub atna_audit_send_failed: Counter<u64>,
    /// ATNA audit records reaped by retention.
    pub atna_audit_reaped: Counter<u64>,
}

impl Metrics {
    /// Create every instrument on `meter`.
    ///
    /// One flat struct literal on purpose: the whole instrument set is visible
    /// in one place, which is what makes an omission obvious. Splitting it into
    /// themed halves would hide exactly that.
    #[expect(
        clippy::too_many_lines,
        reason = "a single struct literal enumerating every instrument; splitting it \
                  would scatter the set this module exists to keep in one place"
    )]
    #[must_use]
    pub fn new(meter: &Meter) -> Self {
        Self {
            http_request_duration: meter
                .f64_histogram(HTTP_REQUEST_DURATION)
                .with_description("HTTP server request duration")
                .with_unit("s")
                .build(),
            http_active_requests: meter
                .i64_up_down_counter(HTTP_ACTIVE_REQUESTS)
                .with_description("In-flight HTTP requests")
                .build(),
            http_request_body_size: meter
                .u64_histogram(HTTP_REQUEST_BODY_SIZE)
                .with_description("HTTP request body size")
                .with_unit("By")
                .build(),
            http_response_body_size: meter
                .u64_histogram(HTTP_RESPONSE_BODY_SIZE)
                .with_description("HTTP response body size")
                .with_unit("By")
                .build(),
            auth_failures: meter
                .u64_counter(AUTH_FAILURES)
                .with_description("Authentication failures by mechanism and reason")
                .build(),
            authz_cedar_decisions: meter
                .u64_counter(AUTHZ_CEDAR_DECISIONS)
                .with_description("Cedar authorization decisions by result")
                .build(),
            authz_remote_pdp_calls: meter
                .u64_counter(AUTHZ_REMOTE_PDP_CALLS)
                .with_description("Remote-PDP authorization calls by result")
                .build(),
            db_pool_acquire_duration: meter
                .f64_histogram(DB_POOL_ACQUIRE_DURATION)
                .with_description("Database connection acquire latency")
                .with_unit("s")
                .build(),
            db_transactions: meter
                .u64_counter(DB_TRANSACTIONS)
                .with_description("Service transactions by outcome")
                .build(),
            aql_queries: meter
                .u64_counter(AQL_QUERIES)
                .with_description("AQL queries by outcome")
                .build(),
            aql_query_duration: meter
                .f64_histogram(AQL_QUERY_DURATION)
                .with_description("AQL query phase latency")
                .with_unit("s")
                .build(),
            aql_plan_cache_events: meter
                .u64_counter(AQL_PLAN_CACHE_EVENTS)
                .with_description("AQL plan-cache events by result")
                .build(),
            compositions_committed: meter
                .u64_counter(COMPOSITIONS_COMMITTED)
                .with_description("Committed compositions by openEHR audit change type")
                .build(),
            validation_failures: meter
                .u64_counter(VALIDATION_FAILURES)
                .with_description("Validation failures by pass")
                .build(),
            version_signature_invalid: meter
                .u64_counter(VERSION_SIGNATURE_INVALID)
                .with_description("Version-signature verification failures by verdict")
                .build(),
            webtemplate_cache_events: meter
                .u64_counter(WEBTEMPLATE_CACHE_EVENTS)
                .with_description("WebTemplate cache events")
                .build(),
            events_published: meter
                .u64_counter(EVENTS_PUBLISHED)
                .with_description("Contribution-outbox events confirmed by the broker")
                .build(),
            atna_audit_emitted: meter
                .u64_counter(ATNA_AUDIT_EMITTED)
                .with_description("ATNA audit records emitted")
                .build(),
            atna_audit_dropped: meter
                .u64_counter(ATNA_AUDIT_DROPPED)
                .with_description("ATNA audit records dropped because the queue was full")
                .build(),
            atna_audit_rejected: meter
                .u64_counter(ATNA_AUDIT_REJECTED)
                .with_description("ATNA audit records rejected as malformed")
                .build(),
            atna_audit_serialize_failed: meter
                .u64_counter(ATNA_AUDIT_SERIALIZE_FAILED)
                .with_description("ATNA audit records that failed to serialize")
                .build(),
            atna_audit_sent: meter
                .u64_counter(ATNA_AUDIT_SENT)
                .with_description("ATNA audit records delivered to a sink")
                .build(),
            atna_audit_send_failed: meter
                .u64_counter(ATNA_AUDIT_SEND_FAILED)
                .with_description("ATNA audit deliveries that failed")
                .build(),
            atna_audit_reaped: meter
                .u64_counter(ATNA_AUDIT_REAPED)
                .with_description("ATNA audit records reaped by retention")
                .build(),
        }
    }
}

/// The process-wide instruments.
static METRICS: OnceLock<Metrics> = OnceLock::new();

/// The instruments.
///
/// Infallible on purpose: before [`init`] runs, this builds them on the global
/// no-op meter, so every recording site is a plain `metrics().x.add(..)` with
/// no `Option` dance and no chance that a metric is worth a panic. A test
/// binary that never initialises telemetry records into the void, which is the
/// correct behaviour there.
#[must_use]
pub fn metrics() -> &'static Metrics {
    METRICS.get_or_init(|| Metrics::new(&opentelemetry::global::meter(SCOPE)))
}

/// Install the instruments for the process.
///
/// # Errors
/// Never fails; returns the existing set if called twice, so a second call in a
/// test binary is harmless.
pub fn init(meter: &Meter) -> &'static Metrics {
    METRICS.get_or_init(|| Metrics::new(meter))
}

/// A view applying `buckets` to the histogram named `name`.
///
/// `SdkMeterProvider::with_view` takes the closure directly
/// (`Fn(&Instrument) -> Option<Stream>`), so no trait import is involved.
fn bucket_view(
    name: &'static str,
    buckets: &'static [f64],
) -> impl Fn(&Instrument) -> Option<Stream> + Send + Sync + 'static {
    move |instrument: &Instrument| {
        (instrument.name() == name).then(|| {
            Stream::builder()
                .with_aggregation(
                    opentelemetry_sdk::metrics::Aggregation::ExplicitBucketHistogram {
                        boundaries: buckets.to_vec(),
                        record_min_max: true,
                    },
                )
                .build()
                .ok()
        })?
    }
}

/// Build the meter provider with the Prometheus reader always attached, and the
/// OTLP periodic reader when `otlp_reader` is supplied.
///
/// Returns the provider and the Prometheus registry the pull surface renders.
///
/// # Errors
/// Returns the exporter's error if the Prometheus reader cannot be built.
pub fn build_provider<R: MetricReader>(
    resource: opentelemetry_sdk::Resource,
    otlp_reader: Option<R>,
) -> Result<(SdkMeterProvider, prometheus::Registry), Box<dyn std::error::Error + Send + Sync>> {
    let registry = prometheus::Registry::new();
    let prometheus_reader = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .build()?;

    let mut builder = SdkMeterProvider::builder()
        .with_resource(resource)
        .with_reader(prometheus_reader);
    if let Some(reader) = otlp_reader {
        builder = builder.with_reader(reader);
    }
    for (name, buckets) in histogram_views() {
        builder = builder.with_view(bucket_view(name, buckets));
    }
    Ok((builder.build(), registry))
}

/// Render the Prometheus text exposition for `/management/prometheus`.
///
/// # Errors
/// Returns the encoder's error if the gathered families cannot be encoded.
pub fn render(registry: &prometheus::Registry) -> Result<String, prometheus::Error> {
    use prometheus::Encoder as _;
    let encoder = prometheus::TextEncoder::new();
    let mut buf = Vec::new();
    encoder.encode(&registry.gather(), &mut buf)?;
    // NOTE: `prometheus::Error` is the registry's own error space and carries
    // no source-bearing variant, so the cause can only travel as its message.
    String::from_utf8(buf).map_err(|e| prometheus::Error::Msg(e.to_string()))
}

/// Record the always-`1` build-identity gauge and the process start time.
///
/// Both are observable gauges: their value is a fact about the process rather
/// than an event, so they are read at collection time instead of pushed.
pub fn register_static_gauges(meter: &Meter, build: &BuildInfo) {
    let labels = [
        KeyValue::new("version", build.version),
        KeyValue::new("git_sha", build.git_sha),
        KeyValue::new("rm_version", build.spec.rm),
    ];
    let _build_info = meter
        .u64_observable_gauge(BUILD_INFO)
        .with_description("Build identity; always 1, carrying the identity as labels")
        .with_callback(move |observer| observer.observe(1, &labels))
        .build();

    // Wall clock comes from jiff, the workspace's one pinned time library
    // (`std::time::SystemTime::now` is a disallowed method); `as_duration` is
    // the signed span since the Unix epoch, so no fallible `duration_since`.
    let start = jiff::Timestamp::now().as_duration().as_secs_f64();
    let _start_time = meter
        .f64_observable_gauge(PROCESS_START_TIME)
        .with_description("Process start time")
        .with_unit("s")
        .with_callback(move |observer| observer.observe(start, &[]))
        .build();
}

/// The temporality the OTLP push uses.
///
/// Cumulative, matching what the Prometheus surface exposes, so the two
/// surfaces cannot disagree about the same counter.
#[must_use]
pub const fn otlp_temporality() -> Temporality {
    Temporality::Cumulative
}

#[cfg(test)]
mod tests {
    use super::*;

    /// No instrument name may carry a Prometheus suffix: the exporter derives
    /// `_total` from the instrument kind and `_seconds`/`_bytes` from the unit,
    /// so a manual suffix here renders `auth_failures_total_total`.
    #[test]
    fn instrument_names_carry_no_prometheus_suffix() {
        let names = [
            HTTP_REQUEST_DURATION,
            HTTP_ACTIVE_REQUESTS,
            HTTP_REQUEST_BODY_SIZE,
            HTTP_RESPONSE_BODY_SIZE,
            AUTH_FAILURES,
            AUTHZ_CEDAR_DECISIONS,
            AUTHZ_REMOTE_PDP_CALLS,
            DB_POOL_CONNECTIONS,
            DB_POOL_ACQUIRE_DURATION,
            DB_TRANSACTIONS,
            AQL_QUERIES,
            AQL_QUERY_DURATION,
            AQL_PLAN_CACHE_EVENTS,
            COMPOSITIONS_COMMITTED,
            VALIDATION_FAILURES,
            VERSION_SIGNATURE_INVALID,
            WEBTEMPLATE_CACHE_EVENTS,
            EVENTS_PUBLISHED,
            ATNA_AUDIT_EMITTED,
            ATNA_AUDIT_SENT,
            PROCESS_START_TIME,
            TOKIO_WORKERS,
            TOKIO_GLOBAL_QUEUE_DEPTH,
            TOKIO_ALIVE_TASKS,
            PROCESS_RESIDENT_MEMORY,
        ];
        for name in names {
            for suffix in ["_total", "_seconds", "_bytes"] {
                assert!(
                    !name.ends_with(suffix),
                    "{name} ends with {suffix}; the exporter adds it — drop it and set a unit"
                );
            }
        }
    }

    /// Every histogram instrument has an explicit bucket ladder. `OTel`'s default
    /// boundaries are not ours, and a silently re-bucketed latency histogram
    /// invalidates every dashboard built on it.
    #[test]
    fn every_histogram_has_an_explicit_bucket_ladder() {
        let with_views: Vec<&str> = histogram_views().iter().map(|(name, _)| *name).collect();
        for name in [
            HTTP_REQUEST_DURATION,
            HTTP_REQUEST_BODY_SIZE,
            HTTP_RESPONSE_BODY_SIZE,
            DB_POOL_ACQUIRE_DURATION,
            AQL_QUERY_DURATION,
        ] {
            assert!(
                with_views.contains(&name),
                "histogram {name} has no bucket view — it would take OTel's defaults"
            );
        }
    }

    /// The viewer's headline tiles and metric deep links, as the pairs
    /// `(instrument name, the name the Prometheus exporter renders it under)`.
    ///
    /// The viewer reads the rendered view over `/management/metrics`, so the
    /// right-hand column is the wire contract; it cannot import a constant from
    /// here (the viewer never depends on this crate).
    const VIEWER_RENDERED_NAMES: [(&str, &str); 4] = [
        (HTTP_ACTIVE_REQUESTS, "http_server_active_requests"),
        (COMPOSITIONS_COMMITTED, "compositions_committed_total"),
        (AQL_QUERIES, "aql_queries_total"),
        (DB_POOL_CONNECTIONS, "db_pool_connections"),
    ];

    /// Pins the exporter-derived name of every instrument the viewer
    /// names as a literal.
    ///
    /// The viewer's `HEADLINE_METRICS` (and its `/operations?metric=…` deep
    /// links) speak the exporter's rendered name space — `_total` is derived
    /// from the counter kind, not written on the instrument — and the two name
    /// spaces meet only here: a renamed instrument or a changed derivation would
    /// otherwise degrade a viewer tile to an em-dash in silence.
    #[test]
    fn exporter_renders_the_viewer_metric_names() {
        use opentelemetry::metrics::MeterProvider as _;

        let (provider, registry) = build_provider(
            opentelemetry_sdk::Resource::builder().build(),
            None::<opentelemetry_sdk::metrics::PeriodicReader<opentelemetry_otlp::MetricExporter>>,
        )
        .expect("the Prometheus reader should build");
        let meter = provider.meter(SCOPE);
        let instruments = Metrics::new(&meter);
        let pool = crate::telemetry::samplers::pool_gauge(&meter);

        // A family reaches the exposition only once it carries a measurement.
        instruments.http_active_requests.add(1, &[]);
        instruments
            .compositions_committed
            .add(1, &[KeyValue::new("change_type", "249")]);
        instruments
            .aql_queries
            .add(1, &[KeyValue::new("outcome", "ok")]);
        pool.record(2, &[KeyValue::new("state", "idle")]);
        pool.record(1, &[KeyValue::new("state", "in_use")]);

        let rendered = render(&registry).expect("the exposition should encode");
        for (instrument, exported) in VIEWER_RENDERED_NAMES {
            assert!(
                exported.starts_with(instrument),
                "{exported} is not {instrument} plus an exporter-derived suffix"
            );
            assert!(
                rendered.contains(&format!("# TYPE {exported} ")),
                "the exporter renders no family named {exported} (instrument \
                 {instrument}); the viewer's HEADLINE_METRICS and metric deep \
                 links consume these rendered spellings verbatim, so a tile \
                 would silently degrade to an em-dash: {rendered}"
            );
            assert!(
                rendered
                    .lines()
                    .any(|line| line.starts_with(&format!("{exported}{{"))),
                "the exporter renders no sample line for {exported}: {rendered}"
            );
        }
    }

    /// One instrument, one kind.
    ///
    /// `db_pool_connections` is created solely by the sampler, as a gauge. A
    /// second instrument of a different kind under the same name on the same
    /// meter is an `OpenTelemetry` duplicate-instrument conflict, and the pull
    /// surface would then expose whichever stream won — the viewer's
    /// `db_pool_connections{state="in_use"}` tile reads that family verbatim.
    #[test]
    fn the_pool_gauge_is_the_only_db_pool_connections_stream() {
        use opentelemetry::metrics::MeterProvider as _;

        let (provider, registry) = build_provider(
            opentelemetry_sdk::Resource::builder().build(),
            None::<opentelemetry_sdk::metrics::PeriodicReader<opentelemetry_otlp::MetricExporter>>,
        )
        .expect("the Prometheus reader should build");
        let meter = provider.meter(SCOPE);
        // The whole shipped instrument set plus the sampler's gauge, on one
        // meter, exactly as the server creates them.
        let _instruments = Metrics::new(&meter);
        let pool = crate::telemetry::samplers::pool_gauge(&meter);
        pool.record(2, &[KeyValue::new("state", "idle")]);
        pool.record(1, &[KeyValue::new("state", "in_use")]);

        let rendered = render(&registry).expect("the exposition should encode");
        assert_eq!(
            rendered.matches("# TYPE db_pool_connections ").count(),
            1,
            "db_pool_connections must render as exactly one family: {rendered}"
        );
        assert!(
            rendered.contains("# TYPE db_pool_connections gauge"),
            "db_pool_connections must render as a gauge: {rendered}"
        );
        for state in ["idle", "in_use"] {
            let label = format!("state=\"{state}\"");
            assert!(
                rendered.lines().any(|line| {
                    line.starts_with("db_pool_connections{") && line.contains(&label)
                }),
                "db_pool_connections carries no {label} sample: {rendered}"
            );
        }

        // A second instrument kind under this name folds its stream into the
        // same family, so one label set renders twice — which Prometheus
        // rejects as a duplicate sample, losing the whole scrape.
        let mut label_sets: Vec<&str> = rendered
            .lines()
            .filter(|line| line.starts_with("db_pool_connections{"))
            .filter_map(|line| line.split_once('}').map(|(labels, _)| labels))
            .collect();
        let emitted = label_sets.len();
        label_sets.sort_unstable();
        label_sets.dedup();
        assert_eq!(
            label_sets.len(),
            emitted,
            "db_pool_connections renders a label set more than once: {rendered}"
        );
    }

    /// The bucket ladders must be sorted and free of duplicates, or the
    /// exporter's cumulative buckets are nonsense.
    #[test]
    fn bucket_ladders_are_sorted_and_unique() {
        for (name, buckets) in histogram_views() {
            let mut sorted = buckets.to_vec();
            sorted.sort_by(f64::total_cmp);
            sorted.dedup();
            assert_eq!(
                sorted.len(),
                buckets.len(),
                "{name} has duplicate bucket boundaries"
            );
            assert!(
                sorted.iter().eq(buckets.iter()),
                "{name} bucket boundaries are not ascending"
            );
        }
    }
}
