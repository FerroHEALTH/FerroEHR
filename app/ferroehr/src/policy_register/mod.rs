// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operational-policy admin registers: the audit category map and the
//! EHDS Art. 3(3) access delay.
//!
//! Each register is versioned in the clinical database
//! ([`crate::storage::policy_register`]): a write through the admin API
//! appends one version with its committer, and the request path reads the
//! newest version of every register ([`LiveRegisters::refresh`]) before it
//! decides anything, so a change applies on the next request on every replica
//! with no restart. A register with no version holds its default: the map
//! FerroEHR ships and no delay. A document is checked when it is written and refused
//! whole when it does not hold; a stored version that no longer compiles
//! leaves the version in force and is logged.
//!
//! No openEHR spec governs operational policy or its registers: our own
//! design/extension.

pub mod access_delay;
pub mod listen;

use std::fmt;
use std::str::FromStr;
use std::sync::{Arc, PoisonError, RwLock};

use sqlx::PgPool;

use crate::policy_register::access_delay::{AccessDelayPolicy, AccessDelayRegister};
use crate::policy_register::listen::{CHANNEL, Listened};
use crate::storage::error::StorageError;
use crate::storage::policy_register as store;
use crate::system_log::categories::{CategoryMapConfig, Classifier};
use crate::system_log::sender::AuditSender;

/// One operational-policy register, by its wire name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Register {
    /// The EHDS category map every access is classified through (Annex II
    /// 3.2(c)).
    AuditCategories,
    /// The EHDS Art. 3(3) delay of the person's own access.
    AccessDelay,
}

impl Register {
    /// Every register, in wire order.
    pub const ALL: [Register; 2] = [Register::AuditCategories, Register::AccessDelay];

    /// The register's wire and storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Register::AuditCategories => "audit-categories",
            Register::AccessDelay => "access-delay",
        }
    }
}

impl fmt::Display for Register {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A name that is no register.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0:?} is not a register: audit-categories or access-delay")]
pub struct UnknownRegister(pub String);

impl FromStr for Register {
    type Err = UnknownRegister;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Register::ALL
            .into_iter()
            .find(|register| register.as_str() == name)
            .ok_or_else(|| UnknownRegister(name.to_owned()))
    }
}

/// Why the registers could not be read.
#[derive(Debug, thiserror::Error)]
pub enum RegisterError {
    /// The register store did not answer.
    #[error("the policy register store did not answer")]
    Store(#[from] StorageError),
}

/// The audit category register in force.
#[derive(Debug, Clone)]
pub struct CategoriesInForce {
    /// Its version; `0` is the default.
    pub version: i64,
    /// The document.
    pub map: CategoryMapConfig,
    /// The document compiled.
    pub classifier: Classifier,
}

/// The access-delay register in force.
#[derive(Debug, Clone)]
pub struct DelayInForce {
    /// Its version; `0` is the default.
    pub version: i64,
    /// The document.
    pub register: AccessDelayRegister,
    /// The policy, when the delay is on.
    pub policy: Option<Arc<AccessDelayPolicy>>,
}

/// The registers in force on this replica, read again whenever a register
/// gains a version.
pub struct LiveRegisters {
    pool: PgPool,
    categories: RwLock<Arc<CategoriesInForce>>,
    access_delay: RwLock<Arc<DelayInForce>>,
    /// The audit sender the category map in force is handed to.
    audit: RwLock<Option<AuditSender>>,
    /// Serializes the reads that install a new version.
    installing: tokio::sync::Mutex<()>,
    /// The listener on [`CHANNEL`], which says when the registers in force
    /// must be read again.
    listened: Arc<Listened>,
}

impl fmt::Debug for LiveRegisters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveRegisters")
            .field("categories", &self.categories().version)
            .field("access_delay", &self.access_delay().version)
            .finish_non_exhaustive()
    }
}

/// The value behind `lock`, which a panicked writer cannot leave torn: every
/// writer swaps one `Arc` whole.
fn read<T: Clone>(lock: &RwLock<T>) -> T {
    lock.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Replace the value behind `lock` with `value`.
fn write<T>(lock: &RwLock<T>, value: T) {
    *lock.write().unwrap_or_else(PoisonError::into_inner) = value;
}

/// The shipped category map compiled, the default of the category register.
#[expect(
    clippy::expect_used,
    reason = "the shipped map is compiled into the binary and its unit test compiles it"
)]
fn shipped_categories() -> Classifier {
    CategoryMapConfig::default()
        .compile()
        .expect("the shipped category map should compile")
}

impl LiveRegisters {
    /// The registers over `pool`, every one at its default until
    /// [`Self::refresh`] reads the stored versions.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            listened: Arc::new(Listened::new(CHANNEL)),
            pool,
            categories: RwLock::new(Arc::new(CategoriesInForce {
                version: 0,
                map: CategoryMapConfig::default(),
                classifier: shipped_categories(),
            })),
            access_delay: RwLock::new(Arc::new(DelayInForce {
                version: 0,
                register: AccessDelayRegister::default(),
                policy: None,
            })),
            audit: RwLock::new(None),
            installing: tokio::sync::Mutex::new(()),
        }
    }

    /// Hand the category map in force to `sender` now and on every change.
    pub fn attach_audit(&self, sender: AuditSender) {
        let categories = self.categories();
        sender.set_categories(&categories.map, categories.classifier.clone());
        write(&self.audit, Some(sender));
    }

    /// The audit category register in force.
    #[must_use]
    pub fn categories(&self) -> Arc<CategoriesInForce> {
        read(&self.categories)
    }

    /// The access-delay register in force.
    #[must_use]
    pub fn access_delay(&self) -> Arc<DelayInForce> {
        read(&self.access_delay)
    }

    /// The version of `register` in force on this replica.
    #[must_use]
    pub fn version(&self, register: Register) -> i64 {
        match register {
            Register::AuditCategories => self.categories().version,
            Register::AccessDelay => self.access_delay().version,
        }
    }

    /// The pool the registers are stored in.
    #[must_use]
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
    }

    /// The listener that says when the registers in force must be read
    /// again; [`Listened::spawn_listener`] starts it.
    #[must_use]
    pub fn listened(&self) -> &Arc<Listened> {
        &self.listened
    }

    /// Read the registers when a notification marked them stale or the
    /// listener is down; otherwise the registers in force stand, and the
    /// request touches no database.
    ///
    /// # Errors
    /// As [`Self::refresh`].
    pub async fn refresh_if_stale(&self) -> Result<(), RegisterError> {
        if self.listened.must_read() {
            self.refresh().await?;
        }
        Ok(())
    }

    /// Read the newest stored version of every register, and install each one
    /// this replica does not hold yet: one indexed read when nothing changed.
    ///
    /// # Errors
    /// [`RegisterError::Store`] when the store does not answer; the registers
    /// in force stay.
    pub async fn refresh(&self) -> Result<(), RegisterError> {
        let heads = store::heads(&self.pool).await?;
        let newest = |register: Register| {
            heads
                .iter()
                .find(|(name, _)| name == register.as_str())
                .map_or(0, |(_, version)| *version)
        };
        if Register::ALL
            .into_iter()
            .all(|register| newest(register) == self.version(register))
        {
            return Ok(());
        }
        let _installing = self.installing.lock().await;
        let categories = newest(Register::AuditCategories);
        let held = self.categories();
        let categories_changed = categories != held.version;
        if categories_changed {
            let map = self
                .load::<CategoryMapConfig>(Register::AuditCategories, categories)
                .await?;
            self.install_categories(categories, map.unwrap_or_else(|| held.map.clone()));
        }
        let delay = newest(Register::AccessDelay);
        let held = self.access_delay();
        if delay != held.version || categories_changed {
            let register = if delay == held.version {
                None
            } else {
                self.load::<AccessDelayRegister>(Register::AccessDelay, delay)
                    .await?
            };
            self.install_delay(delay, register.unwrap_or_else(|| held.register.clone()));
        }
        Ok(())
    }

    /// The stored document of `version` of `register`, or `None` when it
    /// does not read; the document in force then stays, and the failure is
    /// logged.
    async fn load<T: serde::de::DeserializeOwned + Default>(
        &self,
        register: Register,
        version: i64,
    ) -> Result<Option<T>, RegisterError> {
        if version == 0 {
            return Ok(Some(T::default()));
        }
        let Some(row) = store::at(&self.pool, register.as_str(), version).await? else {
            return Ok(None);
        };
        match serde_json::from_str(&row.document) {
            Ok(document) => Ok(Some(document)),
            Err(error) => {
                tracing::error!(
                    %register,
                    version,
                    error = %error,
                    "a stored register version does not read; the version in force stays"
                );
                Ok(None)
            }
        }
    }

    /// Install `map` as version `version` of the category register; a map
    /// that does not compile leaves the map in force under the new version.
    fn install_categories(&self, version: i64, map: CategoryMapConfig) {
        let (map, classifier) = match map.compile() {
            Ok(classifier) => (map, classifier),
            Err(errors) => {
                tracing::error!(
                    register = %Register::AuditCategories,
                    version,
                    errors = errors.len(),
                    "a stored category map does not compile; the map in force stays"
                );
                let held = self.categories();
                (held.map.clone(), held.classifier.clone())
            }
        };
        if let Some(sender) = read(&self.audit) {
            sender.set_categories(&map, classifier.clone());
        }
        write(
            &self.categories,
            Arc::new(CategoriesInForce {
                version,
                map,
                classifier,
            }),
        );
    }

    /// Install `register` as version `version` of the access-delay register,
    /// its categories resolved through the category map in force.
    fn install_delay(&self, version: i64, register: AccessDelayRegister) {
        let policy = AccessDelayPolicy::compile(&register, &self.categories().classifier);
        write(
            &self.access_delay,
            Arc::new(DelayInForce {
                version,
                register,
                policy: policy.map(Arc::new),
            }),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_register_reads_back_from_its_name() {
        for register in Register::ALL {
            assert_eq!(register.as_str().parse::<Register>(), Ok(register));
        }
        assert!("categories".parse::<Register>().is_err());
    }

    #[test]
    fn the_shipped_map_is_the_category_default() {
        assert!(!shipped_categories().is_empty());
    }
}
