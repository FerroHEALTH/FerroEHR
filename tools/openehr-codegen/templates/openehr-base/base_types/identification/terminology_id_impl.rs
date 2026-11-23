// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Accessors for `TERMINOLOGY_ID` (BASE 1.3.0
//! `org.openehr.base.base_types.terminology_id.adoc`):
//! `name [ '(' version ')' ]`.
//!
//! The form is descriptive: the class declares no invariant, and QUERY 1.1.0
//! `master03-syntax.adoc` §Node predicate publishes
//! `terminology_id/value='snomed_ct(3.1)'`.

use super::terminology_id::TerminologyId;

impl TerminologyId {
    /// A terminology id from `name` and `version_id`; an empty `version_id` adds no
    /// suffix.
    #[must_use]
    pub fn new(name: &str, version_id: &str) -> Self {
        let value = if version_id.is_empty() {
            name.to_owned()
        } else {
            format!("{name}({version_id})")
        };
        Self { value }
    }

    /// The terminology name, before any `(version)` suffix (BASE
    /// `TERMINOLOGY_ID.name`).
    #[must_use]
    pub fn name(&self) -> &str {
        match self.value.split_once('(') {
            Some((n, _)) => n,
            None => &self.value,
        }
    }

    /// The version inside a trailing `(...)`, or the empty string (BASE
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
    /// Declares nothing: the class declares no invariant.
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
    fn a_name_and_a_version_build_the_lexical_form() {
        let versioned = TerminologyId::new("ICD10AM", "3rd_ed");
        assert_eq!(versioned.value, "ICD10AM(3rd_ed)");
        assert_eq!(versioned.name(), "ICD10AM");
        assert_eq!(versioned.version_id(), "3rd_ed");
        let unversioned = TerminologyId::new("SNOMED-CT", "");
        assert_eq!(unversioned.value, "SNOMED-CT");
        assert_eq!(unversioned.version_id(), "");
    }

    #[test]
    fn unclosed_parenthesis_yields_empty_version() {
        let t = tid("ICD10AM(3rd_ed");
        assert_eq!(t.name(), "ICD10AM");
        assert_eq!(t.version_id(), "");
    }

    /// The accessors read every shape a released component publishes.
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
