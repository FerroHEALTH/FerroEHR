// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operational policy a replica holds in memory from boot.
//!
//! The policy registers and the controller's access policy are each read
//! before their listener takes traffic. No openEHR spec governs operational
//! policy: our own design.

use anyhow::Context as _;
use ferroehr::service::FerroEhrService;

/// Read the policy registers and the access policy in force, and start the
/// listener of each, so a write on any replica reaches this one.
///
/// # Errors
/// The policy store did not answer.
pub async fn read_and_listen(service: &FerroEhrService) -> anyhow::Result<()> {
    service
        .read_policy_registers()
        .await
        .context("reading the operational-policy registers")?;
    service.policy_registers().listened().spawn_listener();
    service
        .read_access_policy()
        .await
        .context("reading the access policy")?;
    service.access_policy().listened().spawn_listener();
    Ok(())
}
