// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The category and origin rules the ABAC content attributes are read under
//! (EHDS Annex II 3.4). No openEHR spec governs authorization: our own design.

use std::sync::Arc;

use ferroehr::policy_register::LiveRegisters;
use ferroehr::system_log::categories::{AccessedContent, CategoryRecord, Classifier};

/// The deployment's rules for the category and origin attributes (EHDS
/// Annex II 3.4).
///
/// The policy registers whose category map every access is classified
/// through, and the `system_id` a direct commit by a caller holding no
/// `data_origin` grant is stamped with ([`ferroehr::origins`]).
#[derive(Debug, Clone)]
pub struct ContentRules {
    /// The policy registers in force; `None` classifies through an empty map.
    pub registers: Option<Arc<LiveRegisters>>,
    /// The effective openEHR `system_id` (`[api] system_id`).
    pub system_id: String,
}

impl Default for ContentRules {
    fn default() -> Self {
        Self {
            registers: None,
            system_id: ferroehr::service::DEFAULT_SYSTEM_ID.to_owned(),
        }
    }
}

impl ContentRules {
    /// The categories of `content` under the category map in force.
    #[must_use]
    pub fn classify(&self, content: &AccessedContent, written: bool) -> CategoryRecord {
        match &self.registers {
            Some(registers) => registers.categories().classifier.classify(content, written),
            None => Classifier::default().classify(content, written),
        }
    }
}
