// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The render policies a store writes its records under, one per category
//! map a record may name.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use tokio::sync::OnceCell;

use super::AuditRecord;
use crate::system_log::categories::CategoryMapConfig;
use crate::system_log::event::AuditEvent;
#[cfg(feature = "fhir")]
use crate::system_log::render::CompiledPolicy;
use crate::system_log::render::RenderPolicy;

/// The render policy a store writes its records under, registered in the
/// store the first time a record is written.
#[derive(Debug)]
pub(super) struct WritePolicy {
    pub(super) policy: RenderPolicy,
    /// The policy's digest, once the store holds the policy.
    pub(super) digest: OnceCell<Vec<u8>>,
    /// The policy compiled, which renders each record's document for its
    /// search tokens.
    #[cfg(feature = "fhir")]
    pub(super) compiled: OnceCell<Arc<CompiledPolicy>>,
}

impl WritePolicy {
    /// The policy compiled, the first time it is asked for.
    ///
    /// # Errors
    /// The [`RenderPolicy::compile`] errors.
    #[cfg(feature = "fhir")]
    pub(super) async fn compiled(
        &self,
    ) -> Result<Arc<CompiledPolicy>, crate::system_log::AuditError> {
        self.compiled
            .get_or_try_init(|| async { self.policy.compile().map(Arc::new) })
            .await
            .cloned()
    }

    fn new(policy: RenderPolicy) -> Self {
        Self {
            policy,
            digest: OnceCell::new(),
            #[cfg(feature = "fhir")]
            compiled: OnceCell::new(),
        }
    }
}

/// The policies a store writes under: the one of the category map in force,
/// and one per category map a record still in flight was classified through.
///
/// A record names the digest of the map the sender stamped it with, so it is
/// written under the policy that holds that map and renders with the
/// categories it was classified into, whatever map is in force when it is
/// written. No specification governs this: our own design.
#[derive(Debug)]
pub(super) struct Writers {
    /// The policy of the map in force.
    current: RwLock<Arc<WritePolicy>>,
    /// Every policy by the digest of its category map.
    by_map: Mutex<BTreeMap<String, Arc<WritePolicy>>>,
}

impl Writers {
    pub(super) fn new(policy: RenderPolicy) -> Self {
        let writers = Self {
            current: RwLock::new(Arc::new(WritePolicy::new(policy.clone()))),
            by_map: Mutex::new(BTreeMap::new()),
        };
        writers.install(policy);
        writers
    }

    /// Write every record from now on under `policy`, and every record whose
    /// map is the policy's under it too.
    pub(super) fn install(&self, policy: RenderPolicy) {
        let key = policy
            .categories
            .compile()
            .ok()
            .map(|map| map.digest().to_string());
        let mut by_map = self.by_map.lock().unwrap_or_else(PoisonError::into_inner);
        let writing = match key.as_ref().and_then(|key| by_map.get(key)) {
            Some(known) if known.policy == policy => Arc::clone(known),
            _ => Arc::new(WritePolicy::new(policy)),
        };
        if let Some(key) = key {
            by_map.insert(key, Arc::clone(&writing));
        }
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = writing;
    }

    /// Write every record from now on under the policy in force with
    /// `categories` as its category map.
    pub(super) fn set_categories(&self, categories: &CategoryMapConfig) {
        let policy = RenderPolicy {
            categories: categories.clone(),
            ..self.current().policy.clone()
        };
        self.install(policy);
    }

    /// The policy of the map in force.
    pub(super) fn current(&self) -> Arc<WritePolicy> {
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// The policy `event` is written under: the one of the map it was
    /// classified through, else the one in force.
    pub(super) fn of(&self, event: &AuditEvent) -> Arc<WritePolicy> {
        let known = event.category_map_digest.and_then(|digest| {
            self.by_map
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&digest.to_string())
                .cloned()
        });
        known.unwrap_or_else(|| self.current())
    }

    /// The batch as runs of consecutive records written under one policy, as
    /// `(policy, start, end)`, so the records keep their order.
    pub(super) fn runs(&self, records: &[AuditRecord]) -> Vec<(Arc<WritePolicy>, usize, usize)> {
        let mut runs: Vec<(Arc<WritePolicy>, usize, usize)> = Vec::new();
        for (index, record) in records.iter().enumerate() {
            let writing = self.of(&record.event);
            let end = index.saturating_add(1);
            match runs.last_mut() {
                Some((last, _, last_end)) if Arc::ptr_eq(last, &writing) => *last_end = end,
                _ => runs.push((writing, index, end)),
            }
        }
        runs
    }
}
