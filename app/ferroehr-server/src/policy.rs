// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operational policy a replica holds in memory from boot, and the
//! listener sessions that keep every copy fresh.
//!
//! The policy registers and the controller's access policy are read before
//! the listeners take traffic; until a session connects, every request reads
//! the store. No openEHR spec governs operational policy: our own design.

use std::sync::Arc;

use anyhow::Context as _;
use ferroehr::config::FerroEhrConfig;
use ferroehr::policy_register::listen::{Listened, Listeners};
use ferroehr::service::FerroEhrService;

/// Reads the operational-policy registers and the access policy, and starts
/// the access policy's refresher.
///
/// # Errors
/// The policy store did not answer.
pub async fn read(service: &FerroEhrService) -> anyhow::Result<()> {
    service
        .read_policy_registers()
        .await
        .context("reading the operational-policy registers")?;
    service
        .read_access_policy()
        .await
        .context("reading the access policy")?;
    service.access_policy().spawn_refresher();
    Ok(())
}

/// Starts one `LISTEN` session per database for every copy this replica
/// holds, and the ITI-93 applier's channel when `pmir` is set, so a write on
/// any replica reaches this one; how many sessions that took.
///
/// # Errors
/// A listener DSN does not parse.
pub fn listen(
    service: &FerroEhrService,
    config: &FerroEhrConfig,
    pmir: bool,
) -> anyhow::Result<u32> {
    let mut listened: Vec<Arc<Listened>> = service
        .policy_listeners()
        .into_iter()
        .map(|(_, listened)| listened)
        .collect();
    if pmir {
        listened.push(Arc::clone(service.pmir_listened()));
    }
    let listeners =
        Listeners::new(&config.database, &listened).context("reading the listener DSNs")?;
    let sessions = u32::try_from(listeners.sessions()).unwrap_or(u32::MAX);
    listeners.spawn();
    Ok(sessions)
}
