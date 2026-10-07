// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Decommissioning: the one operation that removes every piece of data and
//! every setting an instance stores, behind an explicit confirmation.
//!
//! No openEHR spec governs this — our own design, for the possibility "to
//! securely and easily remove on a permanent basis all data and settings"
//! (`docs/law/eu/cra/text.html` Annex I Part I(2)(m)). [`Decommission::inventory`]
//! reads what an erasure would remove and changes nothing;
//! [`Decommission::erase`] deletes the multimedia blobs first and then drops
//! every schema ([`crate::db::erase_schema`]), so a failure while deleting
//! blobs leaves the database whole and the command can run again.
//!
//! The confirmation names the instance: its stored instance id, or, for a
//! database that stores none (the usage report never ran), the name of the
//! database the clinical domain lives in.

use crate::config::FerroEhrConfig;
use crate::db::domain::StorageConfig;
use crate::db::{DatabaseErasure, DbConfig, DbError, EraseReport};

/// What an erasure would remove, read without changing anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Inventory {
    /// The databases the domains reach and the schemas of this build in each.
    pub databases: EraseReport,
    /// The configured blob store, when one is configured.
    pub blobs: Option<BlobInventory>,
}

/// The configured multimedia blob store and how many blobs it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobInventory {
    /// The bucket.
    pub bucket: String,
    /// How many blobs this server wrote to it.
    pub objects: usize,
}

impl Inventory {
    /// Whether there is nothing to erase: no schema of this build in any
    /// database and no blob in the store.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.databases.is_empty() && self.blobs.as_ref().is_none_or(|b| b.objects == 0)
    }

    /// The value `--confirm` must carry: the stored instance id, else the name
    /// of the database the clinical domain lives in; `None` only when no
    /// database holds the clinical domain, which no layout produces.
    #[must_use]
    pub fn confirmation(&self) -> Option<String> {
        self.databases
            .instance_id
            .map(|id| id.to_string())
            .or_else(|| self.databases.clinical_database().map(str::to_owned))
    }
}

/// What [`Decommission::erase`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Erasure {
    /// No database held a schema of this build and the store held no blob;
    /// nothing was touched.
    NothingToErase,
    /// The instance was erased.
    Erased {
        /// Per database, the schemas dropped.
        databases: Vec<DatabaseErasure>,
        /// How many blobs were deleted; `None` when no blob store is
        /// configured.
        blobs_deleted: Option<usize>,
    },
}

/// Why a decommissioning step failed.
#[derive(Debug, thiserror::Error)]
pub enum DecommissionError {
    /// A database could not be read or erased.
    #[error("the database step failed")]
    Db(#[from] DbError),
    /// The blob store could not be built, listed, or emptied.
    #[cfg(feature = "multimedia")]
    #[error("the multimedia blob store failed; nothing in the database was touched")]
    Blobs(#[from] ferroehr_ext::multimedia::MultimediaError),
    /// A blob store is configured, but this binary was built without the
    /// `multimedia` feature and cannot reach it.
    #[error(
        "[multimedia] names a blob store, but this binary was built without the `multimedia` \
         feature and cannot delete its blobs; run the erase with a build that has it"
    )]
    BlobStoreUnsupported,
    /// The confirmation does not name the instance this configuration reaches.
    #[error(
        "--confirm does not name the instance this configuration reaches; nothing was erased. \
         Run `ferroehr db erase` without --confirm to see what it names"
    )]
    NotConfirmed,
}

/// The decommissioning of one instance: its databases and its blob store.
#[derive(Debug, Clone)]
pub struct Decommission {
    db: DbConfig,
    storage: StorageConfig,
    #[cfg(feature = "multimedia")]
    blobs: Option<ferroehr_ext::multimedia::store::BlobStore>,
}

impl Decommission {
    /// Creates a decommissioning over the databases alone, with no blob store.
    #[must_use]
    pub fn new(db: DbConfig, storage: StorageConfig) -> Self {
        Self {
            db,
            storage,
            #[cfg(feature = "multimedia")]
            blobs: None,
        }
    }

    /// Adds the blob store whose blobs an erasure deletes.
    #[cfg(feature = "multimedia")]
    #[must_use]
    pub fn with_blob_store(mut self, store: ferroehr_ext::multimedia::store::BlobStore) -> Self {
        self.blobs = Some(store);
        self
    }

    /// Creates the decommissioning a configuration describes: its databases,
    /// and its blob store when `[multimedia]` enables one or names an endpoint.
    ///
    /// # Errors
    /// [`DecommissionError::Blobs`] when the blob store client cannot be built,
    /// and [`DecommissionError::BlobStoreUnsupported`] when a store is
    /// configured on a build without the `multimedia` feature.
    pub fn from_config(config: &FerroEhrConfig) -> Result<Self, DecommissionError> {
        let decommission = Self::new(config.db.clone(), config.storage.clone());
        #[cfg(feature = "multimedia")]
        {
            let engine = crate::extensions::multimedia::engine_from_config(&config.multimedia)?;
            Ok(match engine {
                Some(engine) => decommission.with_blob_store(engine.store().clone()),
                None => decommission,
            })
        }
        #[cfg(not(feature = "multimedia"))]
        {
            if config.multimedia.enabled || config.multimedia.endpoint.is_some() {
                return Err(DecommissionError::BlobStoreUnsupported);
            }
            Ok(decommission)
        }
    }

    /// Reads what [`Self::erase`] would remove, changing nothing.
    ///
    /// The blob store is listed first, so an unreachable store fails here
    /// before any database is read.
    ///
    /// # Errors
    /// [`DecommissionError::Blobs`] when the store cannot be listed, and
    /// [`DecommissionError::Db`] when a database cannot be read.
    pub async fn inventory(&self) -> Result<Inventory, DecommissionError> {
        let blobs = self.blob_inventory().await?;
        let databases = crate::db::erase_inventory(&self.db, &self.storage).await?;
        Ok(Inventory { databases, blobs })
    }

    /// Erases the instance when `confirmation` matches
    /// [`Inventory::confirmation`]: the blobs first, then every schema.
    ///
    /// A run with nothing left to erase touches nothing and succeeds whatever
    /// the confirmation, so a second run after a successful one is a no-op.
    ///
    /// # Errors
    /// [`DecommissionError::NotConfirmed`] when the confirmation does not match
    /// (nothing is touched), [`DecommissionError::Blobs`] when the store cannot
    /// be listed or emptied (the database is not touched), and
    /// [`DecommissionError::Db`] when a database cannot be read or erased.
    pub async fn erase(&self, confirmation: &str) -> Result<Erasure, DecommissionError> {
        let inventory = self.inventory().await?;
        if inventory.is_empty() {
            return Ok(Erasure::NothingToErase);
        }
        if inventory.confirmation().as_deref() != Some(confirmation.trim()) {
            return Err(DecommissionError::NotConfirmed);
        }
        let blobs_deleted = self.delete_blobs().await?;
        let report = crate::db::erase_schema(&self.db, &self.storage).await?;
        Ok(Erasure::Erased {
            databases: report.databases,
            blobs_deleted,
        })
    }

    /// The configured store's bucket and blob count.
    #[cfg(feature = "multimedia")]
    async fn blob_inventory(&self) -> Result<Option<BlobInventory>, DecommissionError> {
        let Some(store) = &self.blobs else {
            return Ok(None);
        };
        Ok(Some(BlobInventory {
            bucket: store.bucket().to_owned(),
            objects: store.content_keys().await?.len(),
        }))
    }

    /// No store can be configured on this build.
    #[cfg(not(feature = "multimedia"))]
    #[expect(
        clippy::unused_self,
        reason = "the signature matches the `multimedia` build's method, which reads the store"
    )]
    fn blob_inventory(
        &self,
    ) -> impl Future<Output = Result<Option<BlobInventory>, DecommissionError>> {
        std::future::ready(Ok(None))
    }

    /// Deletes every blob in the configured store.
    #[cfg(feature = "multimedia")]
    async fn delete_blobs(&self) -> Result<Option<usize>, DecommissionError> {
        match &self.blobs {
            Some(store) => Ok(Some(store.delete_all_content().await?)),
            None => Ok(None),
        }
    }

    /// No store can be configured on this build.
    #[cfg(not(feature = "multimedia"))]
    #[expect(
        clippy::unused_self,
        reason = "the signature matches the `multimedia` build's method, which reads the store"
    )]
    fn delete_blobs(&self) -> impl Future<Output = Result<Option<usize>, DecommissionError>> {
        std::future::ready(Ok(None))
    }
}
