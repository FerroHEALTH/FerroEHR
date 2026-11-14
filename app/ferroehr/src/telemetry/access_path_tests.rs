// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

use super::*;

/// A provider built exactly as the server builds it, with no export.
fn provider() -> ferrohealth_telemetry::telemetry::Telemetry {
    ferrohealth_telemetry::telemetry::Telemetry::new(
        &ferrohealth_telemetry::section::TelemetrySection::default(),
        ferrohealth_telemetry::telemetry::Service::new(super::super::SERVICE, "test"),
    )
    .expect("the Prometheus reader should build")
}

/// A listener that never connects, so it reads as down.
fn listener() -> Arc<Listened> {
    Arc::new(Listened::new("ferroehr_test_channel"))
}

/// The exporter derives `_total`, `_seconds` and `_bytes`, so a name that
/// carries one renders it twice.
#[test]
fn instrument_names_carry_no_prometheus_suffix() {
    for name in NAMES {
        for suffix in ["_total", "_seconds", "_bytes"] {
            assert!(
                !name.ends_with(suffix),
                "{name} ends with {suffix}; the exporter adds it"
            );
        }
    }
}

/// Every histogram carries an explicit ladder, sorted and without repeats,
/// and no name is used twice.
#[test]
fn every_histogram_has_a_sorted_ladder_and_names_are_unique() {
    let views = histogram_views();
    for name in [
        GRANTS_REFRESH_DURATION,
        ACCESS_LOG_APPEND_DURATION,
        ACCESS_LOG_SIGN_DURATION,
        AUDIT_SEAL_DURATION,
        ACCESS_LOG_RETENTION_DURATION,
        AUDIT_CHAIN_VERIFICATION_DURATION,
    ] {
        assert!(
            views.iter().any(|(view, _)| *view == name),
            "{name} has no bucket view"
        );
    }
    for (name, buckets) in views {
        assert!(
            buckets.windows(2).all(|pair| pair[0] < pair[1]),
            "{name} bucket boundaries are not strictly ascending"
        );
    }
    let mut names = NAMES.to_vec();
    names.sort_unstable();
    names.dedup();
    assert_eq!(names.len(), NAMES.len(), "an instrument name repeats");
    let theirs = super::super::metrics::histogram_views();
    assert!(
        NAMES
            .iter()
            .all(|name| theirs.iter().all(|(other, _)| other != name)),
        "an access-path instrument reuses a name of the server's own set"
    );
}

/// The exposition carries the decisions by outcome and code, the seal
/// histogram on its own ladder, and each listener's state by its name.
#[tokio::test]
async fn the_exporter_renders_the_access_path_families() {
    let telemetry = provider();
    let meter = telemetry.meter(super::super::metrics::SCOPE);
    let instruments = AccessPathMetrics::new(&meter);
    instruments.decisions.add(
        1,
        &[
            KeyValue::new("outcome", "refused"),
            KeyValue::new("code", "refused-opt-out"),
        ],
    );
    instruments.seal_duration.record(
        0.02,
        &[
            KeyValue::new("chain", "access_log"),
            KeyValue::new("outcome", "sealed"),
        ],
    );
    instruments
        .seal_last_success
        .record(1.0, &[KeyValue::new("chain", "system")]);
    instruments.chain_intact.record(0, &[]);
    let grants = listener();
    assert!(grants.must_read(), "a listener that never connected reads");
    observe_listeners(&meter, &[("grants", Arc::clone(&grants))]);

    let rendered = telemetry
        .render_prometheus()
        .expect("the exposition should encode");
    for line in [
        "access_decisions_total{code=\"refused-opt-out\",outcome=\"refused\"",
        "audit_seal_duration_seconds_bucket{chain=\"access_log\",outcome=\"sealed\",le=\"0.001\"}",
        "policy_listener_up{listener=\"grants\"} 0",
        "policy_listener_losses_total{listener=\"grants\"} 0",
        "policy_copy_reads_total{listener=\"grants\",reason=\"unlistened\"} 1",
        "policy_copy_reads_total{listener=\"grants\",reason=\"notified\"} 0",
        "audit_seal_last_success_seconds{chain=\"system\"} 1",
        "audit_chain_intact 0",
    ] {
        assert!(
            rendered.lines().any(|rendered| rendered.starts_with(line)),
            "the exposition carries no line starting {line}: {rendered}"
        );
    }
    assert!(
        !rendered.contains("policy_notification_age"),
        "a listener that heard nothing reports no notification age: {rendered}"
    );
}

/// The mismatch gauge reports nothing before this instance compared itself
/// with the others, and the instances that differ once it has.
#[tokio::test]
async fn the_exporter_renders_the_instances_that_differ() {
    let telemetry = provider();
    let meter = telemetry.meter(super::super::metrics::SCOPE);
    let pool = sqlx::PgPool::connect_lazy("postgres://ferroehr@localhost/none")
        .expect("a lazy pool should build without a connection");
    let instances = Arc::new(Instances::new(
        pool,
        crate::instance::Fingerprint {
            signing_key: "digest".to_owned(),
            configuration: "c0ffee".to_owned(),
        },
    ));
    observe_instances(&meter, &instances);
    let rendered = telemetry
        .render_prometheus()
        .expect("the exposition should encode");
    assert!(
        !rendered.contains(INSTANCE_MISMATCHES),
        "nothing before the first comparison: {rendered}"
    );
}
