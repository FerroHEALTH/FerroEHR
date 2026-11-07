// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The shared party mapping on the server.
//!
//! It is installed on the service and judged against its templates at boot;
//! the binding side table is filled or checked under its digest, with this
//! replica's lease renewed while it runs; and `ferroehr party-binding refill`
//! is the one path by which the mapping of a deployment changes.
//!
//! A mapping change is a stop-the-world step: every replica stops, the
//! refill runs, and replicas with the new mapping start. No openEHR spec
//! governs the binding: our own design.

use std::path::Path;
use std::sync::Arc;

use anyhow::Context as _;
use clap::Subcommand;
use ferroehr::config::FerroEhrConfig;
use ferroehr::party_binding::fill::{LEASE_RENEWAL, LEASE_TTL, Lease};
use ferroehr::service::FerroEhrService;

/// `ferroehr party-binding …` subcommands.
#[derive(Debug, Subcommand)]
pub enum PartyBindingCmd {
    /// Rebuild the binding side table from the party versions under the
    /// configured mapping and store its digest. Run it with every replica
    /// stopped: it refuses while a replica's lease is live.
    Refill,
}

/// Returns `service` with the configured party mapping installed: the
/// templates a mapped party is validated against and the mapping the side
/// table is derived under.
///
/// # Errors
/// A selector path that does not compile, which the configuration check
/// reports first.
pub fn install(
    service: FerroEhrService,
    config: &FerroEhrConfig,
) -> anyhow::Result<FerroEhrService> {
    let sections = config.authz.party_binding.sections();
    if sections.is_empty() {
        return Ok(service);
    }
    let key_issuers = match ferroehr::config::auth::resolve(&config.auth.shared) {
        Ok(resolved) => ferroehr::config::binding::key_issuers(&resolved),
        Err(e) => return Err(e).context("resolving [auth] for the party mapping"),
    };
    let mapping = sections
        .binding_mapping(&key_issuers, &config.authz.rbac.admin_role)
        .context("compiling [authz.party_binding]")?;
    Ok(service
        .with_mapped_party_templates(sections.mapped_templates())
        .with_binding_mapping(mapping))
}

/// Judges the mapping against its templates, prepares the side table and
/// takes this replica's lease, renewed in the background until it is
/// released.
///
/// # Errors
/// A template the mapping cannot use, a table filled under another mapping,
/// a value two parties carry, or a party store failure.
pub async fn prepare(
    service: &Arc<FerroEhrService>,
    config: &FerroEhrConfig,
) -> anyhow::Result<Option<Held>> {
    let errors = service
        .party_mapping_errors(&config.authz.party_binding.sections())
        .await?;
    anyhow::ensure!(
        errors.is_empty(),
        "[authz.party_binding] is refused: {}",
        errors.join("; ")
    );
    let Some(lease) = service
        .take_binding_lease(uuid::Uuid::now_v7())
        .await
        .context("preparing the party binding side table")?
    else {
        return Ok(None);
    };
    let lease = Arc::new(lease);
    let renewing = Arc::clone(&lease);
    let renewal = tokio::spawn(async move {
        let mut tick = tokio::time::interval(LEASE_RENEWAL);
        loop {
            tick.tick().await;
            if let Err(e) = renewing.renew().await {
                tracing::warn!(
                    error = %ferroehr::error::ErrorChain::new(&e),
                    "the party binding lease could not be renewed"
                );
            }
        }
    });
    Ok(Some(Held { lease, renewal }))
}

/// This replica's lease, renewed in the background.
#[derive(Debug)]
pub struct Held {
    /// The lease.
    lease: Arc<Lease>,
    /// The renewal task.
    renewal: tokio::task::JoinHandle<()>,
}

impl Held {
    /// Stops renewing and releases the lease, so a refill need not wait for
    /// it to lapse.
    pub async fn release(self) {
        let Self { lease, renewal } = self;
        renewal.abort();
        // NOTE: tokio JoinHandle::abort, the aborted task drops its clone of the
        // lease, so this one is the last; the join error of the abort is expected.
        let lease = match renewal.await {
            Ok(()) | Err(_) => Arc::try_unwrap(lease),
        };
        if let Ok(lease) = lease
            && let Err(e) = lease.release().await
        {
            tracing::warn!(
                error = %ferroehr::error::ErrorChain::new(&e),
                "the party binding lease could not be released; it lapses unrenewed"
            );
        }
    }
}

/// `ferroehr party-binding refill`.
///
/// # Errors
/// The configuration does not load or validate, no mapping is configured, a
/// replica's lease is live, a value two parties carry, or the party store
/// fails.
#[expect(
    clippy::print_stdout,
    reason = "the party-binding subcommand's PURPOSE is console output \
              (.claude/rules/reliability.md §tools)"
)]
pub async fn run(
    cmd: &PartyBindingCmd,
    file: Option<&Path>,
    sets: &[(String, String)],
) -> anyhow::Result<()> {
    match cmd {
        PartyBindingCmd::Refill => {
            let config = ferroehr::config::load(file, sets)?;
            config.validate()?;
            let pools = crate::connect_pool(&config).await?;
            let service = install(FerroEhrService::new(&pools), &config)?;
            let errors = service
                .party_mapping_errors(&config.authz.party_binding.sections())
                .await?;
            anyhow::ensure!(
                errors.is_empty(),
                "[authz.party_binding] is refused: {}",
                errors.join("; ")
            );
            service
                .refill_binding(LEASE_TTL)
                .await
                .context("refilling the party binding side table")?;
            println!("the party binding side table is refilled under the configured mapping");
            Ok(())
        }
    }
}
