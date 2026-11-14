// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The instances of one deployment against a real `PostgreSQL` 18 (shared
//! testkit harness): each compares the fingerprint of its signing key and
//! its configuration with every live instance's, and the `instances` health
//! indicator names an instance that differs from the start. No openEHR spec
//! governs the process model: our own design (#5589).

use std::sync::Arc;

use ferroehr::instance::{Agreement, Fingerprint, Instances};
use ferroehr::telemetry::indicators::InstancesHealth;
use ferrohealth_health::document::State;
use ferrohealth_health::registry::HealthIndicator;

/// A fingerprint signing with `key` over the configuration `configuration`.
fn print(key: &str, configuration: &str) -> Fingerprint {
    Fingerprint {
        signing_key: key.to_owned(),
        configuration: configuration.to_owned(),
    }
}

/// Two instances that share key and configuration agree; a third with
/// another key is named by both sides and in health until it leaves; a row
/// no instance touched recently is not compared.
#[tokio::test]
async fn an_instance_with_another_key_or_configuration_is_named_in_health() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let first = Arc::new(Instances::new(pool.clone(), print("AB12", "c0ffee")));
    let health = InstancesHealth::new(Arc::clone(&first));
    assert_eq!(
        health.check().await.state(),
        State::Degraded,
        "not compared yet"
    );
    assert_eq!(
        first.compare().await.expect("the rows are read"),
        Agreement::Agrees { others: 0 }
    );
    let second = Instances::new(pool.clone(), print("AB12", "c0ffee"));
    assert_eq!(
        second.compare().await.expect("the rows are read"),
        Agreement::Agrees { others: 1 }
    );
    assert_eq!(health.check().await.state(), State::Up);

    let third = Instances::new(pool.clone(), print("CD34", "decade"));
    let Agreement::Differs(differences) = third.compare().await.expect("the rows are read") else {
        panic!("the third instance differs from both");
    };
    assert_eq!(differences.len(), 2);
    assert!(
        differences
            .iter()
            .all(|difference| difference.signing_key && difference.configuration)
    );
    let Agreement::Differs(seen) = first.compare().await.expect("the rows are read") else {
        panic!("the first instance sees the third");
    };
    assert_eq!(seen.len(), 1);
    assert_eq!(seen.first().map(|d| d.instance), Some(third.id()));
    let reported = health.check().await;
    assert_eq!(reported.state(), State::Degraded);
    let detail = reported.detail().unwrap_or_default().to_owned();
    assert!(
        detail.contains(&third.id().to_string())
            && detail.contains("another signing key and configuration"),
        "{detail}"
    );

    let telemetry = ferrohealth_telemetry::telemetry::Telemetry::new(
        &ferrohealth_telemetry::section::TelemetrySection::default(),
        ferrohealth_telemetry::telemetry::Service::new("ferroehr", "test"),
    )
    .expect("the Prometheus reader builds");
    ferroehr::telemetry::access_path::observe_instances(
        &telemetry.meter(ferroehr::telemetry::metrics::SCOPE),
        &first,
    );
    let rendered = telemetry
        .render_prometheus()
        .expect("the exposition encodes");
    for line in [
        "instance_mismatches{differs=\"signing_key\"} 1",
        "instance_mismatches{differs=\"configuration\"} 1",
    ] {
        assert!(
            rendered.lines().any(|rendered| rendered.starts_with(line)),
            "the exposition carries no line starting {line}: {rendered}"
        );
    }

    third.leave().await.expect("the third instance leaves");
    sqlx::query("UPDATE clinical.instance SET seen_at = now() - interval '5 minutes' WHERE instance_id = $1")
        .bind(second.id())
        .execute(&pool)
        .await
        .expect("the second instance goes quiet");
    assert_eq!(
        first.compare().await.expect("the rows are read"),
        Agreement::Agrees { others: 0 },
        "an instance that left or went quiet is not compared"
    );
    assert_eq!(health.check().await.state(), State::Up);
}
