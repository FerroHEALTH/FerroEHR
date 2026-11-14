// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A lost policy listener, against a real `PostgreSQL` 18 (shared testkit
//! harness): the database ends each listener's session, the
//! `policy_listeners` health indicator names the lost listeners while they
//! are down, and each reconnects. While a listener is down its copy reads the
//! store on every request, so the indicator degrades and never withdraws
//! readiness. Each listener holds a session of its own, apart from the
//! request pools. No openEHR spec governs the process model: our own design
//! (#5594, #5589).

use std::sync::Arc;
use std::time::Duration;

use ferroehr::policy_register::listen::{Listened, SESSION_NAME};
use ferroehr::telemetry::indicators::PolicyListenersHealth;
use ferrohealth_health::document::State;
use ferrohealth_health::registry::HealthIndicator;

/// Waits until `holds`, at most ten seconds, naming `what` on a timeout.
async fn wait_until(holds: impl Fn() -> bool, what: &str) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !holds() {
        assert!(std::time::Instant::now() < deadline, "{what}: timed out");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
}

#[tokio::test]
async fn a_killed_listener_degrades_health_until_it_reconnects() {
    let (db, svc) = crate::fixtures::service().await;
    let listeners: Vec<(&'static str, Arc<Listened>)> = vec![
        ("ehr_access", Arc::clone(svc.ehr_access_listened())),
        ("grants", Arc::clone(svc.grants().listened())),
        (
            "policy_registers",
            Arc::clone(svc.policy_registers().listened()),
        ),
        ("access_policy", Arc::clone(svc.access_policy().listened())),
    ];
    let health = PolicyListenersHealth::new(listeners.clone());
    let down = health.check().await;
    assert_eq!(
        down.state(),
        State::Degraded,
        "nothing listens before the spawn"
    );
    for (_, listened) in &listeners {
        listened.spawn_listener((*db.pool().connect_options()).clone());
    }
    let every = || listeners.iter().all(|(_, l)| l.is_listening());
    wait_until(every, "every listener connects").await;
    assert_eq!(health.check().await.state(), State::Up);
    let named: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND query LIKE 'LISTEN %' AND application_name = $1",
    )
    .bind(SESSION_NAME)
    .fetch_one(&db.pool())
    .await
    .expect("the listener sessions are read");
    assert_eq!(
        named, 4,
        "each listener holds a named session of its own, apart from the pools"
    );

    let killed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = current_database() AND query LIKE 'LISTEN %') k",
    )
    .fetch_one(&db.pool())
    .await
    .expect("the listener sessions end");
    assert_eq!(killed, 4, "one session per listener");
    wait_until(
        || listeners.iter().any(|(_, l)| !l.is_listening()),
        "a listener sees its session end",
    )
    .await;
    let lost = health.check().await;
    assert_eq!(lost.state(), State::Degraded, "{:?}", lost.detail());
    assert!(
        lost.detail().is_some_and(|d| d.contains("listener lost")),
        "{:?}",
        lost.detail()
    );
    wait_until(every, "every listener connects again").await;
    assert_eq!(health.check().await.state(), State::Up);
    // NOTE: our own design (#5591), the `policy_listener_losses` counter reads these counts.
    for (name, listened) in &listeners {
        assert!(listened.losses() >= 1, "{name} counts its lost session");
    }
}
