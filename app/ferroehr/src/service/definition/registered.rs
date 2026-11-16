// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The template addresses `template_ref` registers, held in memory, so a
//! COMPOSITION commit learns whether its template is registered with no
//! statement (#5775).
//!
//! Every replica holds the registered ids, compared case-insensitively, and
//! listens on [`CHANNEL`]; a registration or a release notifies it when it
//! commits, and the next commit reads the set again. An id the set does not
//! hold is asked of the store, so a template registered a moment ago is never
//! refused, and while the listener is down every commit asks the store. A
//! template released at the same moment is still refused by the template
//! delete guard and the `version.template_id` foreign key. No openEHR spec
//! governs the copy: our own design.

use crate::db::domain::Domain;
use std::collections::BTreeSet;
use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};

use sqlx::PgPool;

use crate::policy_register::listen::Listened;
use crate::storage::error::StorageError;
use crate::storage::template_repo::{self, CHANNEL};

/// The registered template addresses this replica holds.
pub struct RegisteredTemplates {
    pool: PgPool,
    /// The registered ids, lower-cased.
    ids: RwLock<Arc<BTreeSet<String>>>,
    /// Serializes the reads that install a new set.
    installing: tokio::sync::Mutex<()>,
    /// The listener on [`CHANNEL`].
    listened: Arc<Listened>,
}

impl fmt::Debug for RegisteredTemplates {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("RegisteredTemplates")
            .field("held", &self.held().len())
            .field("listened", &self.listened)
            .finish_non_exhaustive()
    }
}

impl RegisteredTemplates {
    /// An empty copy over `pool`, not yet listening, so every commit asks the
    /// store until [`crate::policy_register::listen::Listeners::spawn`] connects.
    #[must_use]
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            ids: RwLock::new(Arc::new(BTreeSet::new())),
            installing: tokio::sync::Mutex::new(()),
            listened: Arc::new(Listened::new(CHANNEL, Domain::Clinical)),
        }
    }

    /// The listener on [`CHANNEL`].
    #[must_use]
    pub fn listened(&self) -> &Arc<Listened> {
        &self.listened
    }

    /// Whether `template_id` is a registered address, compared
    /// case-insensitively: from the copy, and from the store for an id the
    /// copy does not hold or while the listener is down.
    ///
    /// # Errors
    /// [`StorageError::Database`] when the store cannot answer.
    pub async fn holds(&self, template_id: &str) -> Result<bool, StorageError> {
        if self.listened.unlistened() {
            return template_repo::template_ref_exists(&self.pool, template_id).await;
        }
        if self.listened.must_read()
            && let Err(error) = self.read().await
        {
            self.listened.mark_stale();
            return Err(error);
        }
        let key = template_id.to_lowercase();
        if self.held().contains(&key) {
            return Ok(true);
        }
        let registered = template_repo::template_ref_exists(&self.pool, template_id).await?;
        if registered {
            let mut ids = BTreeSet::clone(&self.held());
            ids.insert(key);
            *self.ids.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(ids);
        }
        Ok(registered)
    }

    /// Marks the copy stale after this replica's own registration or
    /// release committed, so its next commit reads the set again before the
    /// notification arrives.
    pub fn changed(&self) {
        self.listened.mark_stale();
    }

    /// The registered ids held.
    fn held(&self) -> Arc<BTreeSet<String>> {
        Arc::clone(&self.ids.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Reads every registered id again and installs them.
    async fn read(&self) -> Result<(), StorageError> {
        let _installing = self.installing.lock().await;
        let ids = template_repo::registered_template_ids(&self.pool).await?;
        *self.ids.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(ids);
        Ok(())
    }
}

impl crate::service::FerroEhrService {
    /// The template addresses this replica holds as registered.
    #[must_use]
    pub fn registered_templates(&self) -> &Arc<RegisteredTemplates> {
        &self.registered_templates
    }
}
