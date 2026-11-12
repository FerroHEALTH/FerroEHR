// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operational-policy admin registers: the audit category map, the EHDS
//! Art. 3(3) access delay and the Cedar policy set.
//!
//! Each register is versioned in the clinical database
//! ([`crate::storage::policy_register`]): a write through the admin API
//! appends one version with its committer, and the request path reads the
//! newest version of every register ([`LiveRegisters::refresh`]) before it
//! decides anything, so a change applies on the next request on every replica
//! with no restart. A register with no version holds its default: the map
//! FerroEHR ships, no delay, and no Cedar policy (so the embedded engine
//! permits nothing). A document is checked when it is written and refused
//! whole when it does not hold; a stored version that no longer compiles
//! leaves the version in force and is logged.
//!
//! No openEHR spec governs operational policy or its registers: our own
//! design/extension.

pub mod access_delay;
pub mod listen;

use std::fmt;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};

use serde::{Deserialize, Serialize};
use sqlx::PgPool;

use crate::policy_register::access_delay::{AccessDelayPolicy, AccessDelayRegister};
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
    /// The policy set of the embedded Cedar engine.
    CedarPolicies,
}

impl Register {
    /// Every register, in wire order.
    pub const ALL: [Register; 3] = [
        Register::AuditCategories,
        Register::AccessDelay,
        Register::CedarPolicies,
    ];

    /// The register's wire and storage name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Register::AuditCategories => "audit-categories",
            Register::AccessDelay => "access-delay",
            Register::CedarPolicies => "cedar-policies",
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
#[error("{0:?} is not a register: audit-categories, access-delay or cedar-policies")]
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

/// The Cedar register document: the policy set as Cedar text.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CedarPolicies {
    /// The policies, in the Cedar policy language; empty permits nothing.
    pub policies: String,
}

/// The check a Cedar policy set passes before it is stored: it parses and
/// validates against the schema of the engine the REST layer builds.
pub type CedarCheck = Arc<dyn Fn(&str) -> Result<(), String> + Send + Sync>;

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

/// The Cedar register in force.
#[derive(Debug, Clone)]
pub struct CedarInForce {
    /// Its version; `0` is the default.
    pub version: i64,
    /// The document.
    pub policies: CedarPolicies,
}

/// The registers in force on this replica, read again whenever a register
/// gains a version.
pub struct LiveRegisters {
    pool: PgPool,
    categories: RwLock<Arc<CategoriesInForce>>,
    access_delay: RwLock<Arc<DelayInForce>>,
    cedar: RwLock<Arc<CedarInForce>>,
    /// The audit sender the category map in force is handed to.
    audit: RwLock<Option<AuditSender>>,
    /// The check a Cedar policy set passes before it is stored.
    cedar_check: RwLock<Option<CedarCheck>>,
    /// Serializes the reads that install a new version.
    installing: tokio::sync::Mutex<()>,
    /// Whether the listener ([`listen`]) is connected, so a request may trust
    /// the registers in force until a notification marks them stale.
    listening: AtomicBool,
    /// Whether a notification arrived that no read has made up yet.
    stale: AtomicBool,
    /// The notifications received, and the wake of a task waiting on one.
    notified: AtomicU64,
    wake: tokio::sync::Notify,
}

impl fmt::Debug for LiveRegisters {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("LiveRegisters")
            .field("categories", &self.categories().version)
            .field("access_delay", &self.access_delay().version)
            .field("cedar", &self.cedar().version)
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
            cedar: RwLock::new(Arc::new(CedarInForce {
                version: 0,
                policies: CedarPolicies::default(),
            })),
            audit: RwLock::new(None),
            cedar_check: RwLock::new(None),
            installing: tokio::sync::Mutex::new(()),
            listening: AtomicBool::new(false),
            stale: AtomicBool::new(false),
            notified: AtomicU64::new(0),
            wake: tokio::sync::Notify::new(),
        }
    }

    /// Hand the category map in force to `sender` now and on every change.
    pub fn attach_audit(&self, sender: AuditSender) {
        let categories = self.categories();
        sender.set_categories(&categories.map, categories.classifier.clone());
        write(&self.audit, Some(sender));
    }

    /// Install the check a Cedar policy set passes before it is stored.
    pub fn set_cedar_check(&self, check: CedarCheck) {
        write(&self.cedar_check, Some(check));
    }

    /// The check a Cedar policy set passes before it is stored, when one is
    /// installed.
    #[must_use]
    pub fn cedar_check(&self) -> Option<CedarCheck> {
        read(&self.cedar_check)
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

    /// The Cedar register in force.
    #[must_use]
    pub fn cedar(&self) -> Arc<CedarInForce> {
        read(&self.cedar)
    }

    /// The version of `register` in force on this replica.
    #[must_use]
    pub fn version(&self, register: Register) -> i64 {
        match register {
            Register::AuditCategories => self.categories().version,
            Register::AccessDelay => self.access_delay().version,
            Register::CedarPolicies => self.cedar().version,
        }
    }

    /// Whether the listener is connected; while it is not, every request reads
    /// the registers.
    #[must_use]
    pub fn is_listening(&self) -> bool {
        self.listening.load(Ordering::Acquire)
    }

    /// Read the registers when a notification marked them stale or the
    /// listener is down; otherwise the registers in force stand, and the
    /// request touches no database.
    ///
    /// # Errors
    /// As [`Self::refresh`].
    pub async fn refresh_if_stale(&self) -> Result<(), RegisterError> {
        if self.must_read() {
            self.refresh().await?;
        }
        Ok(())
    }

    /// The pool the registers are stored in.
    #[must_use]
    pub(crate) fn pool(&self) -> &PgPool {
        &self.pool
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
        let cedar = newest(Register::CedarPolicies);
        let held = self.cedar();
        if cedar != held.version {
            let policies = self
                .load::<CedarPolicies>(Register::CedarPolicies, cedar)
                .await?
                .unwrap_or_else(|| held.policies.clone());
            write(
                &self.cedar,
                Arc::new(CedarInForce {
                    version: cedar,
                    policies,
                }),
            );
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
