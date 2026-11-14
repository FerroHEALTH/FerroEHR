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

use crate::db::acquire::acquire;
use crate::instance::{Agreement, Instances};
use crate::policy_register::listen::Listened;
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
                        format!(
                            "; first in the {} chain at {position}: {}",
                            finding.chain.as_str(),
                            finding.finding
                        )
                    });
                    Outcome::degraded(format!(
                        "hash chain DAMAGED at {at}: {total} finding(s){first}; see \
                         audit.verify_audit_chain() and audit.verify_access_log_chain()"
                    ))
                }
                ChainCheckState::Unverifiable { at, reason } => Outcome::degraded(format!(
                    "the hash chain could not be verified at {at}: {reason}"
                )),
            }
        })
    }
}

/// `policy_listeners` — reports whether each policy copy's LISTEN connection
/// is up.
///
/// A replica holds the `EHR_ACCESS` settings, the policy grants, the
/// operational-policy registers and the access policy in memory, each kept
/// fresh by a notification channel. While a listener is down, every request
/// reads that policy from the store, so every decision stays correct and only
/// latency changes. The indicator therefore reads `DEGRADED`, naming each lost
/// listener, and never takes the instance out of rotation. No openEHR spec
/// governs this: our own design (#5594).
#[derive(Debug)]
pub struct PolicyListenersHealth {
    listeners: Vec<(&'static str, Arc<Listened>)>,
}

impl PolicyListenersHealth {
    /// Constructs over each policy copy's listener, by the name the
    /// indicator reports it under.
    #[must_use]
    pub fn new(listeners: Vec<(&'static str, Arc<Listened>)>) -> Self {
        Self { listeners }
    }
}

impl HealthIndicator for PolicyListenersHealth {
    fn name(&self) -> &'static str {
        "policy_listeners"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move {
            let lost: Vec<&str> = self
                .listeners
                .iter()
                .filter(|(_, listened)| !listened.is_listening())
                .map(|(name, _)| *name)
                .collect();
            if lost.is_empty() {
                Outcome::up()
            } else {
                Outcome::degraded(format!(
                    "listener lost: {}; every request reads that policy from the database \
                     until the listener connects again",
                    lost.join(", ")
                ))
            }
        })
    }
}

/// `instances` — whether every live instance of the deployment signs with
/// this instance's key and runs its configuration ([`Instances`]).
///
/// An instance with another key writes access-log links the others cannot
/// vouch for, so a difference is shown from the start rather than found when
/// a chain fails to verify. Each instance keeps serving, so the indicator
/// reads `DEGRADED` and never takes the instance out of rotation. No openEHR
/// spec governs the process model: our own design (#5589).
#[derive(Debug)]
pub struct InstancesHealth {
    instances: Arc<Instances>,
}

impl InstancesHealth {
    /// Constructs over this instance among the deployment's instances.
    #[must_use]
    pub fn new(instances: Arc<Instances>) -> Self {
        Self { instances }
    }
}

impl HealthIndicator for InstancesHealth {
    fn name(&self) -> &'static str {
        "instances"
    }

    fn check(&self) -> Check<'_> {
        Box::pin(async move {
            match self.instances.agreement() {
                Agreement::Agrees { .. } => Outcome::up(),
                Agreement::Unread => {
                    Outcome::degraded("this instance has not compared itself with the others")
                }
                Agreement::Unknown => Outcome::degraded(
                    "the instances could not be compared: the instance table did not answer",
                ),
                Agreement::Differs(differences) => {
                    let named: Vec<String> = differences
                        .iter()
                        .map(|difference| {
                            let what = match (difference.signing_key, difference.configuration) {
                                (true, true) => "another signing key and configuration",
                                (true, false) => "another signing key",
                                _ => "another configuration",
                            };
                            format!(
                                "instance {} (FerroEHR {}) runs {what}",
                                difference.instance, difference.version
                            )
                        })
                        .collect();
                    Outcome::degraded(format!(
                        "{}; every instance must share the signing key and the configuration",
                        named.join("; ")
                    ))
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{AuditHealth, DbHealth, MigrationsHealth};
    use crate::system_log::config::{AuditConfig, AuditPosture, FailMode};
    use ferrohealth_health::document::State;
    use ferrohealth_health::registry::HealthIndicator;

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
