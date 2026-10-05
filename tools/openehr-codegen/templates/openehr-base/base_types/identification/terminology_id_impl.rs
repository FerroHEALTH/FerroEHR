// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written accessor functions for `TERMINOLOGY_ID`.
//!
//! Spec: BASE 1.3.0
//! `docs/specs/openehr/BASE/docs/UML/classes/org.openehr.base.base_types.terminology_id.adoc`.
//! Lexical form: `name [ '(' version ')' ]`, e.g. `SNOMED-CT`, `ICD10AM(3rd_ed)`.
//!
//! That lexical form is DESCRIPTIVE. The class table carries no `Invariants`
//! row and the BMM no `invariants` key — unlike its sibling `VERSION_TREE_ID`,
//! which declares seven — so this type constrains no value and there is no
//! `Validate` impl below. Released QUERY 1.1.0
//! (`docs/specs/openehr/QUERY/docs/AQL/master03-syntax.adoc` §Node predicate)
//! publishes `terminology_id/value='snomed_ct(3.1)'`, which the master05
//! §Syntaxes production forbids: enforcing it refused openEHR's own example.

use super::terminology_id::TerminologyId;

impl TerminologyId {
    /// The terminology name, i.e. the part before any `(version)` suffix (BASE
    /// `TERMINOLOGY_ID.name`). Distinct names correspond to distinct
    /// terminologies (`ICD10AM` vs `ICD10`).
    #[must_use]
    pub fn name(&self) -> &str {
        match self.value.split_once('(') {
            Some((n, _)) => n,
            None => &self.value,
        }
    }

    /// The terminology version, i.e. the part inside a trailing `(...)`, or the
    /// empty string when versioning is not used (BASE
    /// `TERMINOLOGY_ID.version_id`).
    #[must_use]
    pub fn version_id(&self) -> &str {
        self.value
            .split_once('(')
            .and_then(|(_, rest)| rest.strip_suffix(')'))
            .unwrap_or("")
    }
}

impl crate::validate::Validate for TerminologyId {
    /// Declares nothing, because the class declares nothing.
    ///
    /// The impl exists because the RM validation walk requires every visited
    /// type to implement the trait, not because there is a constraint to
    /// express: see the module documentation for why the lexical form is not
    /// one.
    fn validate_invariants(&self, _out: &mut Vec<crate::validate::InvariantViolation>) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tid(v: &str) -> TerminologyId {
        TerminologyId {
            value: v.to_owned(),
        }
    }

    #[test]
    fn unversioned() {
        let t = tid("SNOMED-CT");
        assert_eq!(t.name(), "SNOMED-CT");
        assert_eq!(t.version_id(), "");
    }

    #[test]
    fn versioned() {
        let t = tid("ICD10AM(3rd_ed)");
        assert_eq!(t.name(), "ICD10AM");
        assert_eq!(t.version_id(), "3rd_ed");
    }

    #[test]
    fn unclosed_parenthesis_yields_empty_version() {
        let t = tid("ICD10AM(3rd_ed");
        assert_eq!(t.name(), "ICD10AM");
        assert_eq!(t.version_id(), "");
    }

    /// The accessors read every shape a released component publishes as a
    /// `TERMINOLOGY_ID.value`, including the two the withdrawn production
    /// refused.
    #[test]
    fn released_shapes_decompose() {
        // QUERY 1.1.0 `master03-syntax.adoc:239` spells this as a
        // `terminology_id/value` in the canonical node-predicate expansion.
        let dotted = tid("snomed_ct(3.1)");
        assert_eq!(dotted.name(), "snomed_ct");
        assert_eq!(dotted.version_id(), "3.1");

        // An interior space is a name like any other now that nothing
        // constrains the value.
        let spaced = tid("SNOMED CT");
        assert_eq!(spaced.name(), "SNOMED CT");
        assert_eq!(spaced.version_id(), "");

        // A URI has no version group, so it decomposes to itself.
        let uri = tid("http://snomed.info/sct");
        assert_eq!(uri.name(), "http://snomed.info/sct");
        assert_eq!(uri.version_id(), "");
    }
}
