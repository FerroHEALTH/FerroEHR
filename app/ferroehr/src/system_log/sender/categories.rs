// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit category register in force on the sender, which classifies every
//! record and is replaced whole when the register gains a version; a stored
//! record names the map that classified it, and the store renders it under
//! that map.

use std::sync::{Arc, PoisonError, RwLock};

use super::AuditSender;
use crate::system_log::categories::{CategoryMapConfig, Classifier};

/// The category map in force on the sender.
#[derive(Debug)]
pub(super) struct Classifiers {
    current: RwLock<Arc<Classifier>>,
}

impl Classifiers {
    /// The map of a sender that classifies through `classifier`.
    pub(super) fn new(classifier: Classifier) -> Self {
        Self {
            current: RwLock::new(Arc::new(classifier)),
        }
    }

    /// The map in force.
    pub(super) fn current(&self) -> Arc<Classifier> {
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Put `classifier` in force.
    pub(super) fn replace(&self, classifier: Classifier) {
        *self.current.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(classifier);
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
        self.inner
            .store
            .change_render_policy(|policy| policy.categories = map.clone());
        self.set_context_categories(map);
        self.inner.categories.replace(classifier);
    }
}
