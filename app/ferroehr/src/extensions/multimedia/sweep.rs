// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The orphan sweep: deletes every blob no stored version references and
//! that is older than the grace period (#5957).
//!
//! A commit uploads its blobs before it takes a connection, so a commit that
//! fails after the upload leaves a blob nothing references. The sweep lists
//! the store, keeps every blob younger than `[record.multimedia] orphan_grace`
//! (an upload whose commit may still be running) and every blob `blob_ref`
//! names in either pseudonymisation domain, and deletes the rest. A write of
//! bytes already stored renews a blob older than half the grace period, so a
//! commit that deduplicates onto an old blob never loses it to the sweep. No
//! openEHR spec governs blob storage: our own design.

use std::collections::BTreeSet;
use std::time::Duration;

use ferroehr_ext::multimedia::blob::{BlobError, BlobKey, BlobStore, older_than};
use sqlx::PgPool;

use crate::db::domain::DomainPools;
use crate::storage::erasure;
use crate::storage::error::StorageError;

/// How many URIs one reference query asks about.
const REFERENCE_BATCH: usize = 1000;

/// Why a sweep stopped.
#[derive(Debug, thiserror::Error)]
pub enum SweepError {
    /// The blob store failed to list or delete.
    #[error("the multimedia blob store failed during the orphan sweep")]
    Store(#[from] BlobError),
    /// A reference read failed, so nothing was deleted on incomplete
    /// information.
    #[error("the blob references could not be read; nothing was deleted")]
    References(#[from] StorageError),
}

/// What a sweep found and, when it ran, deleted.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SweepReport {
    /// Every blob of ours the store listed.
    pub listed: usize,
    /// Blobs kept because they are younger than the grace period, or their
    /// backend records no write time.
    pub too_young: usize,
    /// Blobs kept because a stored version references them.
    pub referenced: usize,
    /// The unreferenced blobs past the grace period, sorted.
    pub orphans: Vec<BlobKey>,
    /// How many of `orphans` the sweep deleted; zero on a dry run.
    pub deleted: usize,
}

/// One sweep over a store and the two domains whose versions reference it.
#[derive(Debug, Clone)]
pub struct Sweep {
    store: BlobStore,
    clinical: PgPool,
    party: PgPool,
    grace: Duration,
}

impl Sweep {
    /// The sweep of `store`, judged against the references in `pools`.
    #[must_use]
    pub fn new(store: BlobStore, pools: &DomainPools, grace: Duration) -> Self {
        Self {
            store,
            clinical: pools.clinical.clone(),
            party: pools.party.clone(),
            grace,
        }
    }

    /// What a sweep would delete; deletes nothing.
    ///
    /// # Errors
    /// [`SweepError`] when the listing or a reference read fails.
    pub async fn plan(&self) -> Result<SweepReport, SweepError> {
        let listing = self.store.content_listing().await?;
        let mut report = SweepReport {
            listed: listing.len(),
            ..SweepReport::default()
        };
        let mut aged = Vec::new();
        for (key, written) in listing {
            // NOTE: a backend with no write time cannot prove an upload finished; only a zero grace takes it.
            let old = match written {
                Some(written) => older_than(written, self.grace),
                None => self.grace.is_zero(),
            };
            if old {
                aged.push(key);
            } else {
                report.too_young += 1;
            }
        }
        let referenced = self.referenced(&aged).await?;
        for key in aged {
            if referenced.contains(&key) {
                report.referenced += 1;
            } else {
                report.orphans.push(key);
            }
        }
        Ok(report)
    }

    /// Deletes every orphan [`Self::plan`] finds and reports them.
    ///
    /// # Errors
    /// [`SweepError`] when the listing, a reference read or a delete fails;
    /// the blobs deleted before a failed delete stay deleted, and a second
    /// run finishes the rest.
    pub async fn run(&self) -> Result<SweepReport, SweepError> {
        let mut report = self.plan().await?;
        for key in &report.orphans {
            self.store.delete(key).await?;
            report.deleted += 1;
        }
        tracing::info!(
            listed = report.listed,
            referenced = report.referenced,
            too_young = report.too_young,
            deleted = report.deleted,
            "multimedia orphan sweep"
        );
        Ok(report)
    }

    /// The keys of `keys` a stored version references, in either domain.
    async fn referenced(&self, keys: &[BlobKey]) -> Result<BTreeSet<BlobKey>, SweepError> {
        let mut found = BTreeSet::new();
        for batch in keys.chunks(REFERENCE_BATCH) {
            let uris: Vec<String> = batch.iter().map(|key| self.store.uri_for(key)).collect();
            for pool in [&self.clinical, &self.party] {
                for uri in erasure::referenced_blob_uris(pool, &uris).await? {
                    if let Some(key) = self.store.key_from_uri(&uri) {
                        found.insert(key);
                    }
                }
            }
        }
        Ok(found)
    }
}
