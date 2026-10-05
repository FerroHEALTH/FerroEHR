// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reproducibility conversion log.
//!
//! NOTE: no openEHR spec governs 1.4→2 conversion — our own design (see the
//! [`crate::adl14`] module flag). The log records every code the converter
//! *synthesises* (external-code at-codes, value-set ac-codes) keyed by a stable
//! signature of what they were minted for, so a re-conversion that consults the
//! same log reuses the same codes — the conversion is idempotent.

use std::collections::BTreeMap;

/// A record of the codes a conversion synthesised, so re-running is idempotent.
///
/// Codes are allocated *outside* the existing (shifted) code-number range and
/// recorded here on first mint; a subsequent conversion given this log looks up
/// the signature first and reuses the stored code instead of allocating a fresh
/// one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConversionLog {
    /// External terminology code (`terminology::code`, e.g. `openehr::127`) →
    /// the synthesised at-code minted for it (e.g. `at1`).
    pub external_at_codes: BTreeMap<String, String>,
    /// Value-set signature (the joined, converted member code list) → the
    /// synthesised ac-code (e.g. `ac1`).
    pub value_sets: BTreeMap<String, String>,
    /// Human-readable provenance notes for the non-mechanical decisions a
    /// conversion took (specialised-code collapse, VCOSU re-mints) — surfaced
    /// by callers into `RESOURCE_DESCRIPTION.conversion_details`.
    pub notes: Vec<String>,
}

impl ConversionLog {
    /// A fresh, empty log.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// The synthesised at-code previously minted for `external_code`, if any.
    #[must_use]
    pub fn external_at_code(&self, external_code: &str) -> Option<&str> {
        self.external_at_codes
            .get(external_code)
            .map(String::as_str)
    }

    /// Record the at-code minted for an external code.
    pub fn record_external_at_code(&mut self, external_code: &str, at_code: &str) {
        self.external_at_codes
            .insert(external_code.to_owned(), at_code.to_owned());
    }

    /// The ac-code previously minted for a value-set `signature`, if any.
    #[must_use]
    pub fn value_set(&self, signature: &str) -> Option<&str> {
        self.value_sets.get(signature).map(String::as_str)
    }

    /// Record a provenance note.
    pub fn note(&mut self, message: String) {
        self.notes.push(message);
    }

    /// Record the ac-code minted for a value-set signature.
    pub fn record_value_set(&mut self, signature: &str, ac_code: &str) {
        self.value_sets
            .insert(signature.to_owned(), ac_code.to_owned());
    }
}
