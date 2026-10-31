// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The terminology extract data model.
//!
//! The descriptors, terms, relationships and extracts the terminology calls
//! (`get_terminology_ids` … `get_value_set`) exchange. No openEHR spec governs
//! this wire: our own design.
//!
//! The `at_date` parameter (an ISO 8601 date) selects the terminology as it
//! stood on a date. Our default provider is the
//! compile-time, spec-pinned `openehr-term` bundle (a single version — TERM
//! 3.1.0), so `at_date` is accepted and validated in shape by the caller but
//! does not change the bundle's answer; it is modelled as `Option<String>`
//! (the ISO date text) rather than a strong date type because the native API
//! never date-resolves against multiple versions.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// The descriptor of a terminology as this terminology service knows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminologyDescription {
    /// `publisher` (1..1) — publisher organisation name.
    pub publisher: String,
    /// `available_versions` (0..1) — identifiers of available versions of
    /// this terminology in this service.
    pub available_versions: Option<Vec<String>>,
    /// `attributes` (0..1) — meta-model attributes that may be requested
    /// within extract requests.
    pub attributes: Option<Vec<String>>,
    /// `uri` (1..1) — published and/or standardised identifying URI for the
    /// terminology.
    pub uri: String,
}

/// A bare terminology concept within the terminology of the owning extract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermCode {
    /// `code` (1..1) — a terminology code or post-coordinated code
    /// expression.
    pub code: String,
}

/// A fully defined term within the terminology of the owning extract: a
/// [`TermCode`] (flattened in) with displayable text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DefinedTerm {
    /// `Term_code.code` (inherited, 1..1).
    pub code: String,
    /// `text` (1..1) — text of term.
    pub text: String,
    /// `language` (0..1) — code representing the language (ISO 639 / IETF
    /// RFC 5646), carried as the bare code string (the native API resolves
    /// rubrics per language directly against the `openehr-term` bundle).
    pub language: Option<String>,
    /// `is_preferred_term` (0..1) — true if this term is the preferred term
    /// among alternatives, if supported within the scoping terminology.
    pub is_preferred_term: Option<bool>,
}

/// A `Terminology_extract._terms_` value: either a bare `Term_code` or a
/// fully defined `Defined_term`.
///
/// Each term is a bare code or carries displayable text, and this closed enum
/// is that choice.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TermEntry {
    /// A fully defined term (`Defined_term`) — has displayable text.
    Defined(DefinedTerm),
    /// A bare code (`Term_code`).
    Bare(TermCode),
}

/// A term relationship, a 1:N code map within the terminology of the owning
/// extract.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TermRelationship {
    /// `origin_code` (1..1) — code of origin ('left-hand') concept.
    pub origin_code: String,
    /// `relation_name` (1..1) — name of the relation; must match a key in
    /// the owning `Terminology_extract._relations_`.
    pub relation_name: String,
    /// `target_codes` (0..1) — codes of target ('right-hand') concept(s).
    pub target_codes: Option<Vec<String>>,
}

/// The definition of a relationship within the terminology meta-model.
///
/// Invariant `Inv_valid_definition`: `local_code /= Void xor external_code /=
/// Void` — enforced by the [`TerminologyRelation::new`] constructor (exactly
/// one of `local_code`/`external_code` is `Some`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TerminologyRelation {
    /// `name` (1..1) — name of this relation from relevant meta-model.
    pub name: String,
    /// `local_code` (0..1) — local code defining this relation.
    pub local_code: Option<String>,
    /// `external_code` (0..1) — code from another terminology that defines a
    /// relation used by this terminology, carried as the bare code string.
    pub external_code: Option<String>,
}

/// The `Terminology_relation` invariant violation (`Inv_valid_definition`).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TerminologyRelationError {
    /// Neither `local_code` nor `external_code` was supplied, or both were —
    /// the `local_code /= Void xor external_code /= Void` invariant.
    #[error(
        "Terminology_relation.Inv_valid_definition: exactly one of local_code/external_code must be set"
    )]
    XorViolation,
}

impl TerminologyRelation {
    /// Construct a `Terminology_relation`, enforcing `Inv_valid_definition`
    /// (`local_code /= Void xor external_code /= Void`).
    ///
    /// # Errors
    ///
    /// [`TerminologyRelationError::XorViolation`] if neither or both of
    /// `local_code`/`external_code` are supplied.
    pub fn new(
        name: impl Into<String>,
        local_code: Option<String>,
        external_code: Option<String>,
    ) -> Result<Self, TerminologyRelationError> {
        if local_code.is_some() == external_code.is_some() {
            return Err(TerminologyRelationError::XorViolation);
        }
        Ok(Self {
            name: name.into(),
            local_code,
            external_code,
        })
    }

    /// Construct a relation defined by a `local_code` (from this
    /// terminology) — upholds `Inv_valid_definition` structurally.
    #[must_use]
    pub fn local(name: impl Into<String>, local_code: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            local_code: Some(local_code.into()),
            external_code: None,
        }
    }

    /// Construct a relation defined by an `external_code` (from another
    /// terminology) — upholds `Inv_valid_definition` structurally.
    #[must_use]
    pub fn external(name: impl Into<String>, external_code: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            local_code: None,
            external_code: Some(external_code.into()),
        }
    }
}

/// The root of a collection of items extracted from one version or release
/// of one terminology.
///
/// May represent a flat value-set, a structured value-set, or a subsumption
/// hierarchy.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TerminologyExtract {
    /// `terminology_id` (1..1) — the namespace identifier of the terminology.
    pub terminology_id: String,
    /// `terminology_version` (0..1) — terminology version (date or dotted
    /// numeric).
    pub terminology_version: Option<String>,
    /// `terms` (0..1) — the terms in the extract, keyed by code; each is a
    /// bare code or a fully defined term ([`TermEntry`]).
    pub terms: Option<BTreeMap<String, TermEntry>>,
    /// `relationships` (0..1) — relationships according to the specification
    /// generating the extract.
    pub relationships: Option<Vec<TermRelationship>>,
    /// `relations` (0..1) — definitions of relations used in this extract,
    /// keyed by `_name_`.
    pub relations: Option<BTreeMap<String, TerminologyRelation>>,
}

impl TerminologyExtract {
    /// The standalone form of a terminology code within this extract's
    /// terminology: a
    /// [`TerminologyCode`](openehr_base::prelude::TerminologyCode) whose
    /// `terminology_id` is this extract's.
    #[must_use]
    pub fn create_terminology_code(
        &self,
        code: impl Into<String>,
    ) -> openehr_base::prelude::TerminologyCode {
        openehr_base::prelude::TerminologyCode {
            terminology_id: self.terminology_id.clone(),
            terminology_version: self.terminology_version.clone(),
            code_string: code.into(),
            uri: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminology_relation_xor_invariant() {
        // local only → ok.
        assert!(TerminologyRelation::new("is_a", Some("123".into()), None).is_ok());
        // external only → ok.
        assert!(TerminologyRelation::new("is_a", None, Some("SCT:116680003".into())).is_ok());
        // neither → Inv_valid_definition violation.
        assert_eq!(
            TerminologyRelation::new("is_a", None, None),
            Err(TerminologyRelationError::XorViolation)
        );
        // both → Inv_valid_definition violation.
        assert_eq!(
            TerminologyRelation::new("is_a", Some("123".into()), Some("SCT:1".into())),
            Err(TerminologyRelationError::XorViolation)
        );
        // the infallible smart constructors uphold the invariant structurally.
        assert_eq!(
            TerminologyRelation::local("is_a", "123")
                .local_code
                .as_deref(),
            Some("123")
        );
        assert_eq!(
            TerminologyRelation::external("is_a", "SCT:1")
                .external_code
                .as_deref(),
            Some("SCT:1")
        );
    }

    #[test]
    fn create_terminology_code_uses_extract_terminology_id() {
        let extract = TerminologyExtract {
            terminology_id: "openehr".to_owned(),
            ..Default::default()
        };
        let tc = extract.create_terminology_code("249");
        assert_eq!(tc.terminology_id, "openehr");
        assert_eq!(tc.code_string, "249");
    }
}
