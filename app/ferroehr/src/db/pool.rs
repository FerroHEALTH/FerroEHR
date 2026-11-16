// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The size of each domain's pool, and the connections the server opens
//! before it answers ready (#5774).
//!
//! A request holds a connection of a pool from its first statement until its
//! handler has answered ([`crate::db::acquire::release_held`]); the pool
//! needs about the arrival rate times that hold, and room for a burst. No
//! openEHR spec governs connection handling: our own design.

use crate::db::acquire::acquire;
use crate::db::domain::{Domain, DomainPools};
use crate::db::{DbConfig, DbError};

/// The pool connections per core an unset [`DbConfig::max_connections`]
/// gives.
///
/// A request holds a connection about as long as it runs on a core (1 to 3
/// ms at the measured rates), so this is about what every core busy at once
/// keeps in use, and the burst above it waits in the pool.
pub const POOL_PER_CORE: u32 = 4;

/// The smallest pool an unset [`DbConfig::max_connections`] gives.
pub const POOL_FLOOR: u32 = 10;

/// The largest pool an unset [`DbConfig::max_connections`] gives: three
/// pools of 30 and the four listeners fit one instance in `PostgreSQL`'s
/// default `max_connections` of 100, less its three reserved slots.
pub const POOL_CEILING: u32 = 30;

/// The fewest connections an unset [`DbConfig::min_connections`] keeps open
/// in a server's pool.
pub const POOL_KEPT_FLOOR: u32 = 2;

/// The cores this process may run on.
fn cores() -> u32 {
    // NOTE: our own design (#5774); a host that cannot tell its cores is sized as one core.
    let cores = std::thread::available_parallelism().map_or(1, std::num::NonZero::get);
    u32::try_from(cores).unwrap_or(u32::MAX)
}

impl DbConfig {
    /// The size of each domain's pool: [`Self::max_connections`], or else
    /// [`POOL_PER_CORE`] per core the process may run on, within
    /// [`POOL_FLOOR`] and [`POOL_CEILING`].
    #[must_use]
    pub fn pool_size(&self) -> u32 {
        self.max_connections.unwrap_or_else(|| {
            cores()
                .saturating_mul(POOL_PER_CORE)
                .clamp(POOL_FLOOR, POOL_CEILING)
        })
    }

    /// The connections each pool keeps open while idle:
    /// [`Self::min_connections`], none when unset, never more than the pool
    /// size.
    #[must_use]
    pub fn pool_kept(&self) -> u32 {
        self.min_connections.unwrap_or(0).min(self.pool_size())
    }

    /// These settings as the server runs them, for one that admits at most
    /// `max_concurrent_requests` at once: an unset pool size is bounded by
    /// that, since a request holds at most one connection of a pool, and an
    /// unset [`Self::min_connections`] keeps one connection per core, at
    /// least [`POOL_KEPT_FLOOR`] and at most the pool size.
    #[must_use]
    pub fn for_serving(&self, max_concurrent_requests: u32) -> Self {
        let mut serving = self.clone();
        let size = self
            .max_connections
            .unwrap_or_else(|| self.pool_size().min(max_concurrent_requests.max(1)));
        serving.max_connections = Some(size);
        serving.min_connections = Some(
            self.min_connections
                .unwrap_or_else(|| cores().max(POOL_KEPT_FLOOR).min(size)),
        );
        serving
    }
}

/// Opens the connections every pool of `pools` keeps ([`DbConfig::pool_kept`]
/// of `settings`) and returns them to their pools, so they are open before
/// the server answers ready; the pools keep them while idle.
///
/// The pools open their kept connections in the background after they
/// connect; the server waits for them here, holding each until all are open
/// so none is handed out twice. No openEHR spec governs this: our own design
/// (#5774).
///
/// # Errors
/// [`DbError::Acquire`] for the first connection that cannot be opened.
pub async fn open_kept(settings: &DbConfig, pools: &DomainPools) -> Result<(), DbError> {
    let kept = settings.pool_kept();
    let mut opened = Vec::new();
    for domain in Domain::ALL {
        for _ in 0..kept {
            opened.push(acquire(pools.get(domain)).await?);
        }
    }
    drop(opened);
    Ok(())
}
