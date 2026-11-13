// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's `ACCESS_POLICY` in force on this replica.
//!
//! The policy is the one `VERSIONED_ACCESS_POLICY` of the instance
//! ([`crate::storage::access_policy`]): which kinds of data carry which
//! sensitivity, whether clinical access needs a care relationship and a
//! declared purpose, and the catalogue of named directives a subject accepts
//! by reference. Each replica holds the newest version in memory and listens
//! on [`CHANNEL`]; a write notifies it when it commits, and the next request
//! reads the newest version again ([`LivePolicy::refresh_if_stale`]). While
//! the listener is down every request reads, so a lost notification never
//! leaves an outdated policy in force, and with nothing stale a request
//! touches no database. With no version committed nothing is tightened: no
//! classification, no care relationship or purpose required, an empty
//! catalogue.
//!
//! `ACCESS_POLICY` is a class of FerroEHR's draft access control package and
//! ITS-REST defines no resource for it, so the store, the copy and its
//! invalidation are our own design/extension.

pub mod check;

use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use sqlx::PgPool;

use crate::policy_register::listen::Listened;
use crate::storage::access_policy as store;
use crate::storage::error::StorageError;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;

/// The archetype of the default access policy definition.
pub const ARCHETYPE: &str = "openEHR-EHR-ACCESS_POLICY.controller.v1";

/// The default definition of the access policy archetype, which every boot
/// stores when no definition of its id is stored
/// ([`crate::service::demographic::default_definitions`]).
pub const DEFINITIONS: [(&str, &str); 1] = [(
    "openEHR-EHR-ACCESS_POLICY.controller.v1.0.0.adls",
    include_str!(
        "../../assets/definitions/access_policy/openEHR-EHR-ACCESS_POLICY.controller.v1.0.0.adls"
    ),
)];

/// The channel a policy write notifies and every replica listens on.
pub const CHANNEL: &str = "ferroehr_access_policy";

/// The access policy in force.
#[derive(Debug, Clone, Default)]
pub struct PolicyInForce {
    /// Its trunk version; `0` when no version is committed.
    pub version: i32,
    /// The policy; `None` when no version is committed, which tightens
    /// nothing.
    pub policy: Option<Arc<AccessPolicy>>,
}

/// Why the policy could not be read.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// The policy store did not answer.
    #[error("the access policy store did not answer")]
    Store(#[from] StorageError),
}

/// The access policy in force on this replica, read again whenever it gains a
/// version.
pub struct LivePolicy {
    pool: PgPool,
    in_force: RwLock<Arc<PolicyInForce>>,
    /// Serializes the reads that install a new version.
    installing: tokio::sync::Mutex<()>,
    /// The listener on [`CHANNEL`], which says when the policy in force must
    /// be read again.
    listened: Arc<Listened>,
}

impl fmt::Debug for LivePolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LivePolicy")
            .field("version", &self.in_force().version)
            .finish_non_exhaustive()
    }
}

impl LivePolicy {
    /// The policy over `pool`, at no version until [`Self::refresh`] reads the
    /// stored one.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            listened: Arc::new(Listened::new(pool.clone(), CHANNEL)),
            pool,
            in_force: RwLock::new(Arc::default()),
            installing: tokio::sync::Mutex::new(()),
        }
    }

    /// The policy in force.
    #[must_use]
    pub fn in_force(&self) -> Arc<PolicyInForce> {
        // NOTE: a writer swaps one `Arc` whole, so a poisoned lock holds no torn value.
        self.in_force
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// The pool the policy is stored in.
    #[must_use]
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The listener that says when the policy in force must be read again;
    /// [`Listened::spawn_listener`] starts it.
    #[must_use]
    pub fn listened(&self) -> &Arc<Listened> {
        &self.listened
    }

    /// Read the policy when a notification marked it stale or the listener is
    /// down; otherwise the policy in force stands, and the request touches no
    /// database.
    ///
    /// # Errors
    /// As [`Self::refresh`].
    pub async fn refresh_if_stale(&self) -> Result<(), PolicyError> {
        if self.listened.must_read() {
            self.refresh().await?;
        }
        Ok(())
    }

    /// Read the newest stored version and install it when this replica does
    /// not hold it yet: one indexed read when nothing changed.
    ///
    /// # Errors
    /// [`PolicyError::Store`] when the store does not answer; the policy in
    /// force stays.
    pub async fn refresh(&self) -> Result<(), PolicyError> {
        if store::head(&self.pool).await? == self.in_force().version {
            return Ok(());
        }
        let _installing = self.installing.lock().await;
        let Some(row) = store::newest(&self.pool).await? else {
            return Ok(());
        };
        if row.version == self.in_force().version {
            return Ok(());
        }
        let policy = match openehr_its::json::from_canonical_json::<AccessPolicy>(&row.data) {
            Ok(policy) => Some(Arc::new(policy)),
            Err(error) => {
                tracing::error!(
                    version = row.version,
                    error = %error,
                    "a stored access policy version does not read; the policy in force stays"
                );
                self.in_force().policy.clone()
            }
        };
        let installed = Arc::new(PolicyInForce {
            version: row.version,
            policy,
        });
        *self
            .in_force
            .write()
            .unwrap_or_else(PoisonError::into_inner) = installed;
        Ok(())
    }
}
