// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Telemetry tests: the metric-name + bucket registry snapshot
//! (renames become deliberate), the OTLP export smokes for traces AND metrics
//! push (against in-memory exporters — the export pipeline is exercised without
//! a live collector), and readiness reflecting a DOWN database.

use ferroehr::telemetry::health::{Health, HealthIndicator, HealthStatus};
use ferroehr::telemetry::metrics::{DB_POOL_CONNECTIONS, histogram_views};

/// The histogram bucket ladders as a stable text table, so a re-bucketing is
/// reviewed deliberately: `OTel`'s defaults are not ours, and a silently changed
/// ladder invalidates every dashboard and alert built on the histogram.
#[test]
fn histogram_bucket_ladders_snapshot() {
    let mut lines = Vec::new();
    for (name, buckets) in histogram_views() {
        lines.push(format!("{name} buckets={buckets:?}"));
    }
    insta::assert_snapshot!(lines.join("\n"));
}

// ── OTLP export smokes (in-memory exporters) ────────────────────────────────

#[tokio::test(flavor = "multi_thread")]
async fn otlp_traces_export_pipeline_smoke() {
    use opentelemetry::trace::TracerProvider as _;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::trace::{InMemorySpanExporterBuilder, SdkTracerProvider};
    use tracing_subscriber::prelude::*;

    let exporter = InMemorySpanExporterBuilder::new().build();
    let provider = SdkTracerProvider::builder()
        .with_simple_exporter(exporter.clone())
        .with_resource(
            Resource::builder()
                .with_service_name("ferroehr-test")
                .build(),
        )
        .build();
    let tracer = provider.tracer("ferroehr");
    let layer = tracing_opentelemetry::layer().with_tracer(tracer);
    let subscriber = tracing_subscriber::registry().with(layer);

    tracing::subscriber::with_default(subscriber, || {
        let span = tracing::info_span!("smoke_span", "http.route" = "/x");
        let _enter = span.enter();
        tracing::info!("inside the smoke span");
    });

    provider.force_flush().expect("flush");
    let spans = exporter.get_finished_spans().expect("finished spans");
    assert!(
        spans.iter().any(|s| s.name == "smoke_span"),
        "the batch/export pipeline did not deliver the span: {:?}",
        spans.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn otlp_metrics_push_pipeline_smoke() {
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::MeterProvider as _;
    use opentelemetry_sdk::Resource;
    use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

    let exporter = InMemoryMetricExporter::default();
    let reader = PeriodicReader::builder(exporter.clone()).build();
    let provider = SdkMeterProvider::builder()
        .with_reader(reader)
        .with_resource(
            Resource::builder()
                .with_service_name("ferroehr-test")
                .build(),
        )
        .build();

    let meter = provider.meter("ferroehr");
    let gauge = meter.u64_gauge(DB_POOL_CONNECTIONS).build();
    gauge.record(3, &[KeyValue::new("state", "idle")]);

    provider.force_flush().expect("flush");
    let metrics = exporter.get_finished_metrics().expect("finished metrics");
    assert!(
        !metrics.is_empty(),
        "the metrics push pipeline exported nothing"
    );
    let has_gauge = metrics.iter().any(|rm| {
        rm.scope_metrics()
            .any(|sm| sm.metrics().any(|m| m.name() == DB_POOL_CONNECTIONS))
    });
    assert!(has_gauge, "db_pool_connections not present in the export");
}

// ── Readiness reflects DB state ─────────────────────────────────────────────

#[tokio::test]
async fn db_health_is_down_when_database_unreachable() {
    // A lazily-connected pool to an unreachable DSN: the DB ping fails, so the
    // readiness indicator reports DOWN (→ aggregate 503). Deterministic and
    // Docker-free; the healthy direction is covered by the persistence suite.
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(300))
        .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/nonexistent")
        .expect("lazy pool");

    let indicator = ferroehr::telemetry::indicators::DbHealth::new(pool);
    assert_eq!(indicator.name(), "db");
    let Health { status, .. } = indicator.check().await;
    assert_eq!(status, HealthStatus::Down);
    assert!(indicator.required(), "db is required for readiness");
}
