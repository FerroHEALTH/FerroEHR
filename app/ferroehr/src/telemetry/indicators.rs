// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Concrete health indicators: the dependency-touching checks (DB pool,
//! migrations, audit sender, event publisher) implementing the
//! [`HealthIndicator`] trait from [`crate::telemetry::health`].
//!
//! The binary constructs them over the live handles and registers them into
//! the [`HealthRegistry`](crate::telemetry::health::HealthRegistry) at boot;
//! the HTTP probes live in the protocol adapter (`ferroehr-rest`).
//!
//! No openEHR spec governs health probes — our own operational design.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::system_log::chain_check::{ChainCheck, ChainCheckState};
use crate::system_log::config::AuditPosture;
use crate::system_log::sender::AuditSender;
use crate::telemetry::health::{Health, HealthIndicator, HealthStatus};
use async_trait::async_trait;
use sqlx::PgPool;

/// `db` — a bounded `SELECT 1` liveness ping. Required for readiness.
#[derive(Debug)]
pub struct DbHealth {
    pool: PgPool,
}

impl DbHealth {
    /// Construct over the application pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl HealthIndicator for DbHealth {
    fn name(&self) -> &'static str {
        "db"
    }

    async fn check(&self) -> Health {
        match sqlx::query("SELECT 1").execute(&self.pool).await {
            Ok(_) => Health::up(),
            Err(e) => {
                // The health family is UNAUTHENTICATED by design (orchestrator
                // probes), and an `sqlx::Error` Display can name the DSN host,
                // the database, the role, or SQL text. The operator reads the
                // cause here; the wire gets the fact alone.
                tracing::error!(error = %e, "health: database ping failed");
                Health::down("database ping failed")
            }
        }
    }
}

/// `migrations` — verifies the greenfield schema is present (the
/// `clinical.node`, `clinical.version` and `clinical.vo_head` core relations
/// exist). Required for readiness.
#[derive(Debug)]
pub struct MigrationsHealth {
    pool: PgPool,
}

impl MigrationsHealth {
    /// Construct over the application pool.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self { pool }
    }
}

#[async_trait]
impl HealthIndicator for MigrationsHealth {
    fn name(&self) -> &'static str {
        "migrations"
    }

    async fn check(&self) -> Health {
        let query = "SELECT (to_regclass('clinical.node') IS NOT NULL \
             AND to_regclass('clinical.version') IS NOT NULL \
             AND to_regclass('clinical.vo_head') IS NOT NULL) AS applied";
        match sqlx::query_scalar::<_, bool>(query)
            .fetch_one(&self.pool)
            .await
        {
            Ok(true) => Health::up(),
            Ok(false) => Health::down("core schema tables missing (migrations not applied)"),
            Err(e) => {
                tracing::error!(error = %e, "health: migration check failed");
                Health::down("migration check failed")
            }
        }
    }
}

/// `audit_sender` — reports the audit posture.
///
/// The sender master switch, the fail mode and the local store. Registered
/// whether or not auditing is on (#3238): a deployment that writes no access
/// log reads `DEGRADED` with the consequence spelled out, so monitoring can
/// see the silence instead of a missing row. Degraded-tolerable (a missing or
/// fail-open trail must not take the instance out of rotation), so it never
/// flips readiness — only the aggregate to `DEGRADED`.
#[derive(Debug)]
pub struct AuditHealth {
    sender: Option<AuditSender>,
    posture: AuditPosture,
}

impl AuditHealth {
    /// Construct over the live audit sender and the configured posture.
    #[must_use]
    pub fn new(sender: AuditSender, posture: AuditPosture) -> Self {
        Self {
            sender: Some(sender),
            posture,
        }
    }

    /// The indicator for a deployment that wired no sender: auditing is off.
    #[must_use]
    pub fn disabled(posture: AuditPosture) -> Self {
        Self {
            sender: None,
            posture,
        }
    }
}

#[async_trait]
impl HealthIndicator for AuditHealth {
    fn name(&self) -> &'static str {
        "audit_sender"
    }

    async fn check(&self) -> Health {
        let live = self.sender.as_ref().is_some_and(AuditSender::enabled);
        let cautions = self.posture.cautions();
        if !live || !self.posture.enabled {
            return Health::degraded(cautions.first().copied().unwrap_or(
                "auditing is disabled: no access log is written and the EHDS logging \
                 component is off (set [audit] enabled = true)",
            ));
        }
        let mut detail = self.posture.summary();
        for caution in cautions {
            detail.push_str("; ");
            detail.push_str(caution);
        }
        Health {
            status: HealthStatus::Up,
            detail: Some(detail),
        }
    }

    fn required(&self) -> bool {
        false
    }
}

/// `audit_chain` — reports the latest scheduled verification of the local
/// audit store's hash chain.
///
/// Damage reads `DOWN` and an unverifiable trail `DEGRADED`, each naming the
/// finding; neither flips readiness, because the trail still records while the
/// damage is investigated. Registered only when the scheduled verification
/// runs.
#[derive(Debug)]
pub struct AuditChainHealth {
    check: ChainCheck,
}

impl AuditChainHealth {
    /// Constructs over the scheduled verification's shared result.
    #[must_use]
    pub fn new(check: ChainCheck) -> Self {
        Self { check }
    }
}

#[async_trait]
impl HealthIndicator for AuditChainHealth {
    fn name(&self) -> &'static str {
        "audit_chain"
    }

    async fn check(&self) -> Health {
        match self.check.state() {
            ChainCheckState::Pending => Health {
                status: HealthStatus::Up,
                detail: Some("the first scheduled verification has not run yet".to_owned()),
            },
            ChainCheckState::Intact { at } => Health {
                status: HealthStatus::Up,
                detail: Some(format!("hash chain verified intact at {at}")),
            },
            ChainCheckState::Damaged {
                at,
                total,
                findings,
            } => {
                let first = findings.first().map_or_else(String::new, |finding| {
                    let position = finding
                        .chain_seq
                        .map_or_else(|| "chain state".to_owned(), |seq| format!("position {seq}"));
                    format!("; first at {position}: {}", finding.finding)
                });
                Health::down(format!(
                    "hash chain DAMAGED at {at}: {total} finding(s){first}; see \
                     audit.verify_audit_chain()"
                ))
            }
            ChainCheckState::Unverifiable { at, reason } => Health::degraded(format!(
                "the hash chain could not be verified at {at}: {reason}"
            )),
        }
    }

    fn required(&self) -> bool {
        false
    }
}

/// `events` — reports the contribution-outbox publisher's broker-delivery
/// health.
///
/// Degraded-tolerable (the outbox buffers while the broker is down, so
/// delivery lag must not block readiness): a broker outage flips only the
/// aggregate to `DEGRADED`, never readiness. Registered only when eventing is
/// enabled.
#[derive(Debug)]
pub struct EventsHealth {
    healthy: Arc<AtomicBool>,
}

impl EventsHealth {
    /// Construct over the publisher's shared health flag
    /// (`EventsHandle::healthy` — a feature-gated path an intra-doc link
    /// would break on under `--no-default-features`).
    #[must_use]
    pub fn new(healthy: Arc<AtomicBool>) -> Self {
        Self { healthy }
    }
}

#[async_trait]
impl HealthIndicator for EventsHealth {
    fn name(&self) -> &'static str {
        "events"
    }

    async fn check(&self) -> Health {
        if self.healthy.load(Ordering::Relaxed) {
            Health::up()
        } else {
            Health::degraded("event broker unavailable; outbox buffering")
        }
    }

    fn required(&self) -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::{AuditHealth, DbHealth, EventsHealth, MigrationsHealth};
    use crate::system_log::config::{AuditConfig, AuditPosture, FailMode};
    use crate::telemetry::health::{HealthIndicator, HealthStatus};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A deployment with auditing off still carries the indicator, and it
    /// reads DEGRADED with the consequence stated, never a missing row (#3238).
    #[tokio::test]
    async fn a_disabled_audit_trail_reads_degraded_with_its_consequence() {
        let posture = AuditPosture::of(&AuditConfig {
            enabled: false,
            ..AuditConfig::default()
        });
        let indicator = AuditHealth::disabled(posture);
        assert_eq!(indicator.name(), "audit_sender");
        assert!(
            !indicator.required(),
            "a silent trail must not take the instance out of rotation"
        );
        let health = indicator.check().await;
        assert_eq!(health.status, HealthStatus::Degraded);
        let detail = health.detail.unwrap_or_default();
        assert!(
            detail.contains("disabled") && detail.contains("EHDS"),
            "{detail}"
        );
    }

    /// The fail-open default is a caution, not a health failure: the trail is
    /// UP, and the detail says what a full queue does to a record.
    #[test]
    fn the_fail_open_default_is_a_stated_caution_and_fail_closed_is_not() {
        let open = AuditPosture::of(&AuditConfig::default());
        assert_eq!(open.fail_mode, FailMode::Open, "the shipped default");
        let cautions = open.cautions();
        assert_eq!(cautions.len(), 1);
        assert!(cautions[0].contains("fail_mode is open"), "{}", cautions[0]);

        let closed = AuditPosture::of(&AuditConfig {
            fail_mode: FailMode::Closed,
            ..AuditConfig::default()
        });
        assert!(closed.cautions().is_empty());
        assert!(
            closed.summary().starts_with("fail_mode=closed"),
            "{}",
            closed.summary()
        );

        let off = AuditPosture::of(&AuditConfig {
            enabled: false,
            fail_mode: FailMode::Closed,
            ..AuditConfig::default()
        });
        assert_eq!(
            off.cautions().len(),
            1,
            "a disabled trail makes the fail mode moot"
        );
    }

    /// The publisher's shared flag IS the indicator's answer: healthy reads UP,
    /// a broker failure reads DEGRADED with a detail, and the indicator never
    /// blocks readiness (the outbox buffers the rows).
    #[tokio::test]
    async fn events_health_follows_the_shared_flag() {
        let healthy = Arc::new(AtomicBool::new(true));
        let indicator = EventsHealth::new(Arc::clone(&healthy));
        assert_eq!(indicator.name(), "events");
        assert!(
            !indicator.required(),
            "delivery lag must never take the instance out of rotation"
        );
        assert_eq!(indicator.check().await.status, HealthStatus::Up);

        healthy.store(false, Ordering::Relaxed);
        let degraded = indicator.check().await;
        assert_eq!(degraded.status, HealthStatus::Degraded);
        assert!(
            degraded.detail.is_some_and(|d| d.contains("outbox")),
            "the degraded detail must say the outbox buffers"
        );
    }

    /// A failing database probe must report the FACT and nothing else.
    ///
    /// The health family is always-on and unauthenticated (orchestrator probes),
    /// and an `sqlx::Error` Display can carry the DSN host, the database name and
    /// the connecting role. This forces a real connection failure — a pool
    /// pointed at a port nothing listens on — and asserts the served detail
    /// carries none of it.
    #[tokio::test]
    async fn a_failing_db_probe_discloses_no_connection_detail() {
        // Deliberately unreachable, and deliberately full of things that must not
        // appear on the wire: a role, a password, a host and a database name.
        let dsn = "postgres://secret_role:secret_password@127.0.0.1:1/secret_database";
        let pool = sqlx::postgres::PgPoolOptions::new()
            .acquire_timeout(std::time::Duration::from_millis(500))
            .connect_lazy(dsn)
            .expect("a lazy pool over a syntactically valid DSN should build");

        for health in [
            DbHealth::new(pool.clone()).check().await,
            MigrationsHealth::new(pool).check().await,
        ] {
            assert_eq!(health.status, HealthStatus::Down);
            let detail = health.detail.unwrap_or_default();
            for leaked in [
                "secret_role",
                "secret_password",
                "secret_database",
                "127.0.0.1",
                "postgres://",
                "SELECT",
                "to_regclass",
            ] {
                assert!(
                    !detail.contains(leaked),
                    "the unauthenticated health detail must not contain {leaked:?}: {detail:?}"
                );
            }
        }
    }
}
