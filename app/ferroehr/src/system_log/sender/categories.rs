// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit category register in force on the sender, which classifies every
//! record and is replaced whole when the register gains a version, and every
//! earlier map the drain still projects a record through.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError, RwLock};

use super::AuditSender;
use crate::system_log::categories::{CategoryMapConfig, Classifier, MapDigest};

/// The category maps of the sender: the one in force, and each one a record
/// may have been classified through, by its digest.
#[derive(Debug)]
pub(super) struct Classifiers {
    current: RwLock<Arc<Classifier>>,
    by_digest: Mutex<BTreeMap<String, Arc<Classifier>>>,
}

impl Classifiers {
    /// The maps of a sender that classifies through `classifier`.
    pub(super) fn new(classifier: Classifier) -> Self {
        let classifiers = Self {
            current: RwLock::new(Arc::new(classifier.clone())),
            by_digest: Mutex::new(BTreeMap::new()),
        };
        classifiers.replace(classifier);
        classifiers
    }

    /// The map in force.
    pub(super) fn current(&self) -> Arc<Classifier> {
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Put `classifier` in force, keeping the maps before it.
    pub(super) fn replace(&self, classifier: Classifier) {
        let classifier = Arc::new(classifier);
        self.by_digest
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(classifier.digest().to_string(), Arc::clone(&classifier));
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = classifier;
    }

    /// The map whose digest is `digest`, else the one in force.
    #[cfg_attr(
        not(feature = "fhir"),
        expect(dead_code, reason = "the drain's, for the projections of entries")
    )]
    pub(super) fn of(&self, digest: Option<MapDigest>) -> Arc<Classifier> {
        digest
            .and_then(|digest| {
                self.by_digest
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&digest.to_string())
                    .cloned()
            })
            .unwrap_or_else(|| self.current())
    }
}

impl AuditSender {
    /// The category map every access record is classified through.
    #[must_use]
    pub fn categories(&self) -> Arc<Classifier> {
        self.inner.categories.current()
    }

    /// Classify every record from now on through `classifier`, the compiled
    /// `map`, and have the local store render each one under the map it was
    /// classified through.
    pub fn set_categories(&self, map: &CategoryMapConfig, classifier: Classifier) {
        if let Some(store) = &self.inner.store {
            store.set_categories(map);
        }
        self.inner.categories.replace(classifier);
    }
}
