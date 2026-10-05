// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Health indicators + registry + the readiness-aggregation semantics.
//!
//! A [`HealthIndicator`] is a named, async, bounded check (e.g. "ping the DB").
//! This module owns the trait, the registry, and the aggregation rules; the
//! concrete indicators live in [`crate::telemetry::indicators`] and the HTTP
//! probe handlers in the protocol adapter (`ferroehr-rest`, the always-on
//! public `/health` family). The registry runs every indicator concurrently,
//! each bounded to [`CHECK_TIMEOUT`], so a wedged dependency cannot hang a
//! probe.
//!
//! No openEHR spec governs health probes — our own operational design.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use http::StatusCode;
use serde::Serialize;
use tokio::task::JoinSet;

/// The upper bound on a single indicator check: a probe must answer promptly
/// even when a dependency is wedged. A check that exceeds it is reported
/// [`HealthStatus::Down`].
pub const CHECK_TIMEOUT: Duration = Duration::from_secs(1);

/// The status of a single component or of the aggregate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HealthStatus {
    /// Fully operational.
    Up,
    /// Operational but with a caveat; does not flip readiness (a
    /// degraded-tolerable indicator, e.g. the audit sender in fail-open mode).
    Degraded,
    /// Not operational.
    Down,
}

/// The outcome of one indicator check.
#[derive(Debug, Clone, Serialize)]
pub struct Health {
    /// The component status.
    pub status: HealthStatus,
    /// Optional human-readable detail (never PHI).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

impl Health {
    /// A healthy result with no detail.
    #[must_use]
    pub const fn up() -> Self {
        Self {
            status: HealthStatus::Up,
            detail: None,
        }
    }

    /// A down result with a reason.
    #[must_use]
    pub fn down(detail: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Down,
            detail: Some(detail.into()),
        }
    }

    /// A degraded result with a reason.
    #[must_use]
    pub fn degraded(detail: impl Into<String>) -> Self {
        Self {
            status: HealthStatus::Degraded,
            detail: Some(detail.into()),
        }
    }
}

/// A named, bounded health check. Implemented by the binary for each dependency
/// (DB, migrations-applied, audit sender).
#[async_trait]
pub trait HealthIndicator: Send + Sync + std::fmt::Debug {
    /// The component name (the key under `components` in the aggregate body).
    fn name(&self) -> &'static str;

    /// Run the check. Should return promptly; the registry additionally bounds
    /// it to [`CHECK_TIMEOUT`].
    async fn check(&self) -> Health;

    /// Whether a [`HealthStatus::Down`] of this indicator flips overall
    /// readiness to `DOWN`. Degraded-tolerable indicators (fail-open audit)
    /// override this to `false`: they report detail but never block readiness.
    fn required(&self) -> bool {
        true
    }
}

/// The set of indicators evaluated by the public `/health/readiness` probe.
/// Cheaply cloneable (`Arc` of indicator handles).
#[derive(Clone, Default, Debug)]
pub struct HealthRegistry {
    indicators: Arc<[Arc<dyn HealthIndicator>]>,
}

impl HealthRegistry {
    /// Build a registry from a set of indicators.
    #[must_use]
    pub fn new(indicators: Vec<Arc<dyn HealthIndicator>>) -> Self {
        Self {
            indicators: indicators.into(),
        }
    }

    /// Whether any indicators are registered.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.indicators.is_empty()
    }

    /// Evaluate every indicator concurrently (each bounded to
    /// [`CHECK_TIMEOUT`]) and aggregate.
    pub async fn evaluate(&self) -> AggregateHealth {
        let mut set: JoinSet<(&'static str, bool, Health)> = JoinSet::new();
        for indicator in self.indicators.iter().map(Arc::clone) {
            set.spawn(async move {
                let name = indicator.name();
                let required = indicator.required();
                let health = match tokio::time::timeout(CHECK_TIMEOUT, indicator.check()).await {
                    Ok(h) => h,
                    Err(_) => Health::down(format!(
                        "health check timed out after {}ms",
                        CHECK_TIMEOUT.as_millis()
                    )),
                };
                (name, required, health)
            });
        }

        let mut components = BTreeMap::new();
        let mut overall = HealthStatus::Up;
        while let Some(joined) = set.join_next().await {
            let (name, required, health) = match joined {
                Ok(v) => v,
                Err(join_err) => {
                    // A panicked check counts as a required DOWN. The join
                    // error's Display can carry the panic payload, and this
                    // surface is unauthenticated — so it is logged, not served.
                    tracing::error!(error = %join_err, "health: check task failed");
                    overall = HealthStatus::Down;
                    components.insert("unknown", Health::down("health check task failed"));
                    continue;
                }
            };
            overall = combine(overall, health.status, required);
            components.insert(name, health);
        }

        AggregateHealth {
            status: overall,
            components,
        }
    }
}

/// Fold a component result into the running aggregate. A required `DOWN` forces
/// the aggregate `DOWN`; a `DEGRADED` (or a non-required `DOWN`) only downgrades
/// an otherwise-`UP` aggregate to `DEGRADED`.
fn combine(current: HealthStatus, component: HealthStatus, required: bool) -> HealthStatus {
    if current == HealthStatus::Down || (component == HealthStatus::Down && required) {
        HealthStatus::Down
    } else if current == HealthStatus::Degraded || component != HealthStatus::Up {
        // A non-required DOWN or any DEGRADED downgrades an otherwise-UP aggregate.
        HealthStatus::Degraded
    } else {
        HealthStatus::Up
    }
}

/// The aggregate `/health` body.
#[derive(Debug, Clone, Serialize)]
pub struct AggregateHealth {
    /// The overall status.
    pub status: HealthStatus,
    /// Per-component results.
    pub components: BTreeMap<&'static str, Health>,
}

impl AggregateHealth {
    /// The HTTP status for this aggregate: `503` when `DOWN`, else `200`
    /// (`DEGRADED` is still served up — the surface is reachable).
    #[must_use]
    pub fn http_status(&self) -> StatusCode {
        match self.status {
            HealthStatus::Down => StatusCode::SERVICE_UNAVAILABLE,
            HealthStatus::Up | HealthStatus::Degraded => StatusCode::OK,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug)]
    struct Fixed {
        name: &'static str,
        health: Health,
        required: bool,
    }

    #[async_trait]
    impl HealthIndicator for Fixed {
        fn name(&self) -> &'static str {
            self.name
        }
        async fn check(&self) -> Health {
            self.health.clone()
        }
        fn required(&self) -> bool {
            self.required
        }
    }

    fn reg(indicators: Vec<Fixed>) -> HealthRegistry {
        HealthRegistry::new(
            indicators
                .into_iter()
                .map(|i| -> Arc<dyn HealthIndicator> { Arc::new(i) })
                .collect(),
        )
    }

    #[tokio::test]
    async fn all_up_is_up_200() {
        let r = reg(vec![
            Fixed {
                name: "db",
                health: Health::up(),
                required: true,
            },
            Fixed {
                name: "audit",
                health: Health::up(),
                required: false,
            },
        ]);
        let agg = r.evaluate().await;
        assert_eq!(agg.status, HealthStatus::Up);
        assert_eq!(agg.http_status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn required_down_is_down_503() {
        let r = reg(vec![Fixed {
            name: "db",
            health: Health::down("no connection"),
            required: true,
        }]);
        let agg = r.evaluate().await;
        assert_eq!(agg.status, HealthStatus::Down);
        assert_eq!(agg.http_status(), StatusCode::SERVICE_UNAVAILABLE);
    }

    #[tokio::test]
    async fn degraded_tolerable_down_does_not_flip_readiness() {
        // A non-required indicator that is DOWN degrades but does not fail.
        let r = reg(vec![
            Fixed {
                name: "db",
                health: Health::up(),
                required: true,
            },
            Fixed {
                name: "audit",
                health: Health::down("transport unreachable (fail-open)"),
                required: false,
            },
        ]);
        let agg = r.evaluate().await;
        assert_eq!(agg.status, HealthStatus::Degraded);
        assert_eq!(agg.http_status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn explicit_degraded_reports_degraded() {
        let r = reg(vec![Fixed {
            name: "audit",
            health: Health::degraded("queue backpressure"),
            required: false,
        }]);
        let agg = r.evaluate().await;
        assert_eq!(agg.status, HealthStatus::Degraded);
    }
}
