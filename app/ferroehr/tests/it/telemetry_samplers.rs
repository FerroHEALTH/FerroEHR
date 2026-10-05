// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The background metric samplers (`ferroehr::telemetry::samplers`): the
//! instrumented pool acquisition and the periodic pool/runtime gauge task.
//!
//! Both are exercised against real objects — a migrated `PostgreSQL` 18 from the
//! shared testkit harness and an in-memory `OTel` metric exporter — so no OTLP
//! collector is involved. No openEHR spec governs telemetry — our own
//! design/extension.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::time::Duration;

use opentelemetry::metrics::MeterProvider;
use opentelemetry_sdk::Resource;
use opentelemetry_sdk::metrics::{InMemoryMetricExporter, PeriodicReader, SdkMeterProvider};

use ferroehr::telemetry::metrics::{
    DB_POOL_CONNECTIONS, TOKIO_ALIVE_TASKS, TOKIO_GLOBAL_QUEUE_DEPTH, TOKIO_WORKERS,
};
use ferroehr::telemetry::samplers;

/// The instrument names the exporter has collected so far.
fn exported_names(exporter: &InMemoryMetricExporter) -> Vec<String> {
    let mut names: Vec<String> = exporter
        .get_finished_metrics()
        .expect("finished metrics")
        .iter()
        .flat_map(|rm| {
            rm.scope_metrics()
                .flat_map(|sm| sm.metrics().map(|m| m.name().to_string()))
                .collect::<Vec<_>>()
        })
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// `samplers::acquire` hands back a genuine pooled connection — the recording
/// wrapper must not swap in anything the caller cannot query on, and it must
/// leave the pool usable for the next acquisition.
#[tokio::test]
async fn acquire_returns_a_usable_pooled_connection() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    {
        let mut conn = samplers::acquire(&pool).await.expect("acquire");
        let one: i32 = sqlx::query_scalar("SELECT 1")
            .fetch_one(&mut *conn)
            .await
            .expect("query on the acquired connection");
        assert_eq!(one, 1, "the acquired connection serves queries");
        assert!(pool.size() >= 1, "a real checkout is held on the pool");
    }

    // The guard was a real pool checkout, so the pool keeps serving after it is
    // dropped rather than leaking the connection.
    let mut again = samplers::acquire(&pool).await.expect("second acquire");
    let two: i32 = sqlx::query_scalar("SELECT 2")
        .fetch_one(&mut *again)
        .await
        .expect("query on the re-acquired connection");
    assert_eq!(two, 2);
}

/// The wrapper PROPAGATES the pool's failure instead of masking it as a
/// telemetry concern: a pool that cannot connect times out, and the caller sees
/// that `sqlx` error.
#[tokio::test]
async fn acquire_propagates_the_pool_error() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(300))
        .connect_lazy("postgres://nobody:nobody@127.0.0.1:1/nonexistent")
        .expect("lazy pool");

    let error = samplers::acquire(&pool)
        .await
        .expect_err("an unreachable database cannot be acquired");
    assert!(
        matches!(error, sqlx::Error::PoolTimedOut),
        "the pool's own error must reach the caller, got {error:?}"
    );
}

/// The spawned sampler records ALL FOUR gauges — the pool state plus the three
/// stable `tokio` runtime counters — on the meter it was handed, and it keeps
/// running until the telemetry guard aborts its handle.
#[tokio::test(flavor = "multi_thread")]
async fn the_sampler_records_the_pool_and_runtime_gauges_until_aborted() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();

    let exporter = InMemoryMetricExporter::default();
    let provider = SdkMeterProvider::builder()
        .with_reader(PeriodicReader::builder(exporter.clone()).build())
        .with_resource(
            Resource::builder()
                .with_service_name("ferroehr-test")
                .build(),
        )
        .build();

    let handle = samplers::spawn(pool, provider.meter("ferroehr"));

    // The interval fires its first tick immediately, so one sample is due at
    // once; poll rather than assume a scheduling order.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut names = Vec::new();
    while std::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(50)).await;
        provider.force_flush().expect("flush");
        names = exported_names(&exporter);
        if names.iter().any(|n| n == DB_POOL_CONNECTIONS) {
            break;
        }
    }

    for instrument in [
        DB_POOL_CONNECTIONS,
        TOKIO_WORKERS,
        TOKIO_GLOBAL_QUEUE_DEPTH,
        TOKIO_ALIVE_TASKS,
    ] {
        assert!(
            names.iter().any(|n| n == instrument),
            "the sampler must record {instrument}, exported: {names:?}"
        );
    }

    // The RSS gauge reads /proc/self/status, so it exists exactly where procfs
    // does: present on Linux (what every deployment runs), absent elsewhere.
    #[cfg(target_os = "linux")]
    assert!(
        names
            .iter()
            .any(|n| n == ferroehr::telemetry::metrics::PROCESS_RESIDENT_MEMORY),
        "the sampler must record the resident-set gauge on Linux, exported: {names:?}"
    );

    // The task is a loop, not a one-shot: it is still running, and the handle
    // the telemetry guard holds is what ends it.
    assert!(!handle.is_finished(), "the sampler runs until aborted");
    handle.abort();
    assert!(
        handle.await.expect_err("aborted").is_cancelled(),
        "aborting the handle stops the sampler"
    );
}
