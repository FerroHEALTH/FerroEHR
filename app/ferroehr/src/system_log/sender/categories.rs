// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit category register in force on the sender, which classifies every
//! record and is replaced whole when the register gains a version.

use std::sync::{Arc, PoisonError};

use super::AuditSender;
use crate::system_log::categories::{CategoryMapConfig, Classifier};

impl AuditSender {
    /// The category map every access record is classified through.
    #[must_use]
    pub fn categories(&self) -> Arc<Classifier> {
        Arc::clone(
            &self
                .inner
                .categories
                .read()
                .unwrap_or_else(PoisonError::into_inner),
        )
    }

    /// Classify every record from now on through `classifier`, the compiled
    /// `map`, and have the local store render each one under the map it was
    /// classified through.
    pub fn set_categories(&self, map: &CategoryMapConfig, classifier: Classifier) {
        if let Some(store) = &self.inner.store {
            store.set_categories(map);
        }
        *self
            .inner
            .categories
            .write()
            .unwrap_or_else(PoisonError::into_inner) = Arc::new(classifier);
    }
}
