// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A lost policy listener, against a real `PostgreSQL` 18 (shared testkit
//! harness): the database ends each listener's session, the
//! `policy_listeners` health indicator names the lost listeners while they
//! are down, and each reconnects. While a listener is down its copy reads the
//! store on every request, so the indicator degrades and never withdraws
//! readiness. Every channel of one database is held on one session, apart
//! from the request pools, and a domain listening on a DSN of its own takes a
//! second. No openEHR spec governs the process model: our own design (#5594,
//! #5589, #5845).

use std::sync::Arc;
use std::time::Duration;

use ferroehr::db::DbConfig;
use ferroehr::policy_register::listen::{Listened, Listeners, SESSION_NAME};
use ferroehr::telemetry::indicators::PolicyListenersHealth;
use ferrohealth_health::document::State;
use ferrohealth_health::registry::HealthIndicator;
use ferrohealth_secret::secret_url::SecretUrl;

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
    let listeners = svc.policy_listeners();
    assert_eq!(listeners.len(), 7, "every policy copy is listed");
    let health = PolicyListenersHealth::new(listeners.clone());
    let down = health.check().await;
    assert_eq!(
        down.state(),
        State::Degraded,
        "nothing listens before the spawn"
    );
    let copies: Vec<Arc<Listened>> = listeners.iter().map(|(_, l)| Arc::clone(l)).collect();
    crate::policy_fixtures::listen(&db, &copies);
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
        named, 1,
        "every channel of the database is held on one named session, apart from the pools"
    );

    let killed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM (SELECT pg_terminate_backend(pid) FROM pg_stat_activity \
         WHERE datname = current_database() AND query LIKE 'LISTEN %') k",
    )
    .fetch_one(&db.pool())
    .await
    .expect("the listener sessions end");
    assert_eq!(killed, 1, "one session for every listener");
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

/// The sessions one replica takes are one per listen DSN, as
/// [`DbConfig::listen_sessions`] counts them: both domains on one DSN share a
/// session, and the party domain on a DSN of its own takes a second (#5845).
#[tokio::test]
async fn one_session_per_listen_dsn() {
    let (db, svc) = crate::fixtures::service().await;
    let copies: Vec<Arc<Listened>> = svc
        .policy_listeners()
        .into_iter()
        .map(|(_, l)| l)
        .chain([Arc::clone(svc.pmir_listened())])
        .collect();
    let mut settings = DbConfig::new(db.url());
    let separator = if db.url().contains('?') { '&' } else { '?' };
    settings.party.listen_url = Some(SecretUrl::new(format!(
        "{}{separator}connect_timeout=10",
        db.url()
    )));
    let listeners = Listeners::new(&settings, &copies).expect("the DSNs parse");
    assert_eq!(listeners.sessions(), 2);
    assert_eq!(settings.listen_sessions(), 2);
    listeners.spawn();
    wait_until(
        || copies.iter().all(|l| l.is_listening()),
        "every copy listens",
    )
    .await;
    let sessions: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM pg_stat_activity \
         WHERE datname = current_database() AND query LIKE 'LISTEN %' AND application_name = $1",
    )
    .bind(SESSION_NAME)
    .fetch_one(&db.pool())
    .await
    .expect("the listener sessions are read");
    assert_eq!(sessions, 2, "one session per listen DSN");
}
