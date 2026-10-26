// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Concrete health indicators: the dependency-touching checks (DB pool,
//! migrations, audit sender, event publisher) implementing the family's
//! [`HealthIndicator`] trait.
//!
//! The binary constructs them over the live handles and registers them into
//! a [`ferrohealth_health::registry::Registry`] at boot; the HTTP probes live
//! in the protocol adapter (`ferroehr-rest`). `db` and `migrations` read
//! `down` when they fail, which withdraws readiness; every other indicator
//! reads at worst `degraded`, so it reports without taking the instance out
//! of rotation.
//!
//! No openEHR spec governs health probes — our own operational design.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::db::acquire::acquire;
use crate::system_log::chain_check::{ChainCheck, ChainCheckState};
use crate::system_log::config::AuditPosture;
use crate::system_log::sender::{AuditSender, DeliveryStatus};
use ferrohealth_health::registry::{Check, HealthIndicator, Outcome};
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

impl HealthIndicator for DbHealth {
    fn name(&self) -> &'static str {
        "db"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move {
            let mut conn = match acquire(&self.pool).await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::error!(error = %e.driver(), "health: no database connection");
                    return Outcome::down("database ping failed");
                }
            };
            match sqlx::query("SELECT 1").execute(&mut *conn).await {
                Ok(_) => Outcome::up(),
                Err(e) => {
                    // The health family is UNAUTHENTICATED by design (orchestrator
                    // probes), and an `sqlx::Error` Display can name the DSN host,
                    // the database, the role, or SQL text. The operator reads the
                    // cause here; the wire gets the fact alone.
                    tracing::error!(error = %e, "health: database ping failed");
                    Outcome::down("database ping failed")
                }
            }
        })
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

impl HealthIndicator for MigrationsHealth {
    fn name(&self) -> &'static str {
        "migrations"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move {
            let query = "SELECT (to_regclass('clinical.node') IS NOT NULL \
                 AND to_regclass('clinical.version') IS NOT NULL \
                 AND to_regclass('clinical.vo_head') IS NOT NULL) AS applied";
            let mut conn = match acquire(&self.pool).await {
                Ok(conn) => conn,
                Err(e) => {
                    tracing::error!(error = %e.driver(), "health: no database connection");
                    return Outcome::down("migration check failed");
                }
            };
            match sqlx::query_scalar::<_, bool>(query)
                .fetch_one(&mut *conn)
                .await
            {
                Ok(true) => Outcome::up(),
                Ok(false) => Outcome::down("core schema tables missing (migrations not applied)"),
                Err(e) => {
                    tracing::error!(error = %e, "health: migration check failed");
                    Outcome::down("migration check failed")
                }
            }
        })
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

impl HealthIndicator for AuditHealth {
    fn name(&self) -> &'static str {
        "audit_sender"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move {
            let live = self.sender.as_ref().is_some_and(AuditSender::enabled);
            let cautions = self.posture.cautions();
            if !live || !self.posture.enabled {
                return Outcome::degraded(cautions.first().copied().unwrap_or(
                    "auditing is disabled: no access log is written and the EHDS logging \
                     component is off (set [audit] enabled = true)",
                ));
            }
            Outcome::up()
        })
    }
}

/// `audit_delivery` — reports each forwarder delivering to an external ARR.
///
/// Each `ihe-atna` forwarder names the records waiting, those the repository
/// refused, and whether the last attempt reached it.
///
/// A repository that cannot be reached, or a refused record, reads
/// `DEGRADED`: the records wait in their store and nothing is lost yet, so
/// readiness never flips. Registered only when a repository is configured.
#[derive(Debug)]
pub struct AuditDeliveryHealth {
    delivery: DeliveryStatus,
}

impl AuditDeliveryHealth {
    /// Constructs over the forwarders' shared status.
    #[must_use]
    pub fn new(delivery: DeliveryStatus) -> Self {
        Self { delivery }
    }
}

impl HealthIndicator for AuditDeliveryHealth {
    fn name(&self) -> &'static str {
        "audit_delivery"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move {
            let mut degraded = false;
            let parts: Vec<String> = self
                .delivery
                .statuses()
                .into_iter()
                .map(|(sink, status)| {
                    if !status.reachable || status.depth.quarantined > 0 {
                        degraded = true;
                    }
                    format!(
                        "{sink}: {} waiting, {} refused, {} delivered, {}{}",
                        status.depth.waiting(),
                        status.depth.quarantined,
                        status.delivered,
                        if status.reachable {
                            "reachable"
                        } else {
                            "unreachable"
                        },
                        if status.durable {
                            ""
                        } else {
                            ", held in memory"
                        },
                    )
                })
                .collect();
            let detail = parts.join("; ");
            if degraded {
                Outcome::degraded(detail)
            } else {
                Outcome::up()
            }
        })
    }
}

/// `audit_chain` — reports the latest scheduled verification of the local
/// audit store's hash chain.
///
/// Damage and an unverifiable trail both read `DEGRADED`, each naming the
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

impl HealthIndicator for AuditChainHealth {
    fn name(&self) -> &'static str {
        "audit_chain"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move {
            match self.check.state() {
                ChainCheckState::Pending | ChainCheckState::Intact { .. } => Outcome::up(),
                ChainCheckState::Damaged {
                    at,
                    total,
                    findings,
                } => {
                    let first = findings.first().map_or_else(String::new, |finding| {
                        let position = finding.chain_seq.map_or_else(
                            || "chain state".to_owned(),
                            |seq| format!("position {seq}"),
                        );
                        format!("; first at {position}: {}", finding.finding)
                    });
                    Outcome::degraded(format!(
                        "hash chain DAMAGED at {at}: {total} finding(s){first}; see \
                         audit.verify_audit_chain()"
                    ))
                }
                ChainCheckState::Unverifiable { at, reason } => Outcome::degraded(format!(
                    "the hash chain could not be verified at {at}: {reason}"
                )),
            }
        })
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

impl HealthIndicator for EventsHealth {
    fn name(&self) -> &'static str {
        "events"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move {
            if self.healthy.load(Ordering::Relaxed) {
                Outcome::up()
            } else {
                Outcome::degraded("event broker unavailable; outbox buffering")
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AuditHealth, DbHealth, EventsHealth, MigrationsHealth};
    use crate::system_log::config::{AuditConfig, AuditPosture, FailMode};
    use ferrohealth_health::document::State;
    use ferrohealth_health::registry::HealthIndicator;
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
        let health = indicator.check().await;
        assert_eq!(
            health.state(),
            State::Degraded,
            "a silent trail must not take the instance out of rotation"
        );
        let detail = health.detail().unwrap_or_default();
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
        assert_eq!(closed.cautions(), Vec::<&str>::new());
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
        assert_eq!(indicator.check().await.state(), State::Up);

        healthy.store(false, Ordering::Relaxed);
        let degraded = indicator.check().await;
        assert_eq!(
            degraded.state(),
            State::Degraded,
            "delivery lag must never take the instance out of rotation"
        );
        assert!(
            degraded.detail().is_some_and(|d| d.contains("outbox")),
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
            assert_eq!(health.state(), State::Down);
            let detail = health.detail().unwrap_or_default();
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
