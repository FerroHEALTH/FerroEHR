// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! This instance among the deployment's instances: compared with the others
//! at boot, every heartbeat after, and removed when it stops (#5589). No
//! openEHR spec governs the process model: our own design.

use std::sync::Arc;

use anyhow::Context as _;
use ferroehr::config::FerroEhrConfig;
use ferroehr::instance::{Fingerprint, Instances};
use ferroehr::telemetry::indicators::InstancesHealth;
use ferroehr::versioning::signature::signer::Signer;
use ferrohealth_health::registry::HealthIndicator;
use sqlx::PgPool;

/// Writes this instance's row over the clinical `pool`, compares it with the
/// others, starts its heartbeat and pushes its health indicator.
///
/// # Errors
/// The configuration does not render, or the instance table does not answer.
pub async fn join(
    pool: PgPool,
    config: &FerroEhrConfig,
    signer: &Signer,
    indicators: &mut Vec<Arc<dyn HealthIndicator>>,
) -> anyhow::Result<Arc<Instances>> {
    let instances = Arc::new(Instances::new(
        pool,
        Fingerprint::of(config, signer).context("fingerprinting the configuration")?,
    ));
    instances
        .compare()
        .await
        .context("comparing this instance with the deployment's others")?;
    instances.spawn_heartbeat();
    indicators.push(Arc::new(InstancesHealth::new(Arc::clone(&instances))));
    Ok(instances)
}

/// Removes this instance's row as it stops.
pub async fn leave(instances: &Instances) {
    if let Err(error) = instances.leave().await {
        tracing::warn!(error = %error, "this instance's row could not be deleted");
    }
}
