// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The domain pools a process opens, inside the instance's connection budget
//! (`[database] connection_budget`, #5845). No openEHR spec governs
//! connection handling: our own design.

use anyhow::Context as _;
use ferroehr::db;
use ferroehr::db::domain::{Domain, DomainPools};

/// What the pools are opened for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Opening {
    /// The server: the pools sized for its concurrency, with the connections
    /// each keeps open before it answers ready.
    Serving,
    /// A one-shot command: the pools as configured, opening only what the
    /// command uses, so it fits beside the instances in the connection budget.
    OneShot,
}

/// Connects the domain pools and prepares every database they reach.
///
/// Each pool carries its own domain's `search_path`, and a domain a deployment
/// gave `[database.<domain>] url` carries its own credential — which is what
/// makes the pseudonymisation boundary a role boundary rather than only a
/// schema one. The domain split is our own deployment extension; no openEHR
/// spec governs it.
///
/// # Errors
/// A connection, migration or domain-isolation failure, contextualized for the
/// operator.
pub(crate) async fn connect(
    config: &ferroehr::config::FerroEhrConfig,
    opening: Opening,
) -> anyhow::Result<DomainPools> {
    let database = match opening {
        Opening::Serving => {
            let concurrency = u32::try_from(config.server.shared.max_concurrent_requests.get())
                .unwrap_or(u32::MAX);
            config.database.for_serving(concurrency)
        }
        Opening::OneShot => config.database.clone(),
    };
    let pools = db::connect_domains(&database)
        .await
        .context("connecting to PostgreSQL")?;
    db::prepare(&config.database, &pools, config.deployment.profile)
        .await
        .context("preparing the database schema")?;
    db::pool::open_kept(&database, &pools)
        .await
        .context("opening the pooled connections")?;
    tracing::info!(
        pool_size = database.pool_size(),
        kept_open = database.pool_kept(),
        connections = database.connections(),
        budget = database.connection_budget,
        "each domain pool is open"
    );
    if opening == Opening::Serving {
        warn_over_server_slots(&database, &pools).await;
    }
    for domain in Domain::ALL {
        if config.database.is_separated(domain) {
            tracing::info!(
                %domain,
                "the domain connects on its own DSN: its credential is a database role of its own"
            );
        }
    }
    Ok(pools)
}

/// Warns when the connections this instance may open to one `PostgreSQL`
/// exceed what that server admits to a role without superuser rights, read
/// through each pool (`max_connections` less `superuser_reserved_connections`
/// and `reserved_connections`). A warning, never a refusal: behind a
/// transaction-mode pooler the pools count against the pooler, and the limit
/// read through it is the server's. No openEHR spec governs this: our own
/// design (#5845).
async fn warn_over_server_slots(database: &db::DbConfig, pools: &DomainPools) {
    match db::pool::server_slots(database, pools).await {
        Ok(over) => {
            for slots in over {
                tracing::warn!(
                    domains = %slots.domains,
                    connections = slots.planned,
                    admitted = slots.admitted,
                    budget = database.connection_budget,
                    "this instance may open more connections to PostgreSQL than it admits; \
                     lower database.connection_budget or raise PostgreSQL's max_connections"
                );
            }
        }
        Err(error) => tracing::warn!(
            error = %ferroehr::error::ErrorChain::new(&error),
            "PostgreSQL's connection limit could not be read"
        ),
    }
}
