// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Which pool of an instance a [`PgPool`] handle is: the `pool` label of the
//! pool metrics (#5972), and the domain the order of acquisition is checked
//! by.
//!
//! A pool is named where it is built ([`crate::db::connect_domain`],
//! [`crate::db::domain_pool_from`], the audit runtime's own pool) and found
//! again by the connect options it holds, which the register keeps alive, so
//! no later pool can take its address. An entry whose pool is gone is dropped
//! at the next naming. No openEHR spec governs this: our own design.

use std::sync::{Arc, PoisonError, RwLock};

use sqlx::PgPool;
use sqlx::postgres::PgConnectOptions;

use crate::db::domain::Domain;

/// One pool of an instance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PoolName {
    /// The pool of one storage domain.
    Domain(Domain),
    /// The audit runtime's own pool over the audit store
    /// ([`crate::db::pool::AUDIT_RUNTIME_POOL`]).
    AuditRuntime,
}

impl PoolName {
    /// The `pool` label value: the domain's name, or `audit_runtime`.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Domain(domain) => domain.as_str(),
            Self::AuditRuntime => "audit_runtime",
        }
    }

    /// The storage domain the pool serves.
    #[must_use]
    pub const fn domain(self) -> Domain {
        match self {
            Self::Domain(domain) => domain,
            Self::AuditRuntime => Domain::Audit,
        }
    }
}

/// The `pool` label value of a pool no instance named, such as a test's own.
pub const UNNAMED: &str = "unnamed";

/// The named pools, by the connect options each holds.
static NAMES: RwLock<Vec<(Arc<PgConnectOptions>, PoolName)>> = RwLock::new(Vec::new());

/// Names `pool` as `name`.
pub fn name(pool: &PgPool, name: PoolName) {
    let options = pool.connect_options();
    let mut names = NAMES.write().unwrap_or_else(PoisonError::into_inner);
    // NOTE: our own design (#5972), an entry only the register holds belongs to a pool that is gone.
    names.retain(|(held, _)| Arc::strong_count(held) > 1);
    names.push((options, name));
}

/// The name of `pool`; `None` for a pool no instance named.
#[must_use]
pub fn name_of(pool: &PgPool) -> Option<PoolName> {
    let options = pool.connect_options();
    NAMES
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .find(|(held, _)| Arc::ptr_eq(held, &options))
        .map(|(_, name)| *name)
}

/// The `pool` label value of `pool`.
#[must_use]
pub fn label_of(pool: &PgPool) -> &'static str {
    name_of(pool).map_or(UNNAMED, PoolName::as_str)
}
