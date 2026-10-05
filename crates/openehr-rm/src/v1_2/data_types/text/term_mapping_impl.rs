// @generated-from-template templates/openehr-rm/data_types/text/term_mapping_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariant for `TERM_MAPPING`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.term_mapping.adoc`
//! §Invariants + §Functions.
//!
//! `Match_valid` (`is_valid_match_code (match)`) is enforced here. The sibling
//! `Purpose_valid` — `purpose /= Void implies terminology
//! (Terminology_id_openehr).has_code_for_group_id
//! (Group_id_term_mapping_purpose, purpose.defining_code)` — is terminology-bound
//! and cannot be decided in this crate, which has no terminology dependency; it
//! is enforced in the terminology-aware path (`validate::terminology`, the
//! `TERM_MAPPING` slot) against the `openehr-term` bundle. The clause's typing
//! half ("purpose is a `DV_CODED_TEXT`") is structural here
//! (`purpose: Option<DvCodedText>`).

use crate::v1_2::data_types::text::term_mapping::TermMapping;
use openehr_base::validate::{InvariantViolation, Validate};

impl TermMapping {
    /// RM `TERM_MAPPING.is_valid_match_code(c)`: `c` is one of `> = < ?`.
    #[must_use]
    pub fn is_valid_match_code(c: char) -> bool {
        matches!(c, '<'..='?')
    }

    /// RM `TERM_MAPPING.narrower()`: the mapping is to a narrower term
    /// (`match = '<'`).
    #[must_use]
    pub fn narrower(&self) -> bool {
        self.r#match == '<'
    }

    /// RM `TERM_MAPPING.broader()`: the mapping is to a broader term
    /// (`match = '>'`).
    #[must_use]
    pub fn broader(&self) -> bool {
        self.r#match == '>'
    }

    /// RM `TERM_MAPPING.equivalent()`: the mapping is to an equivalent term
    /// (`match = '='`).
    #[must_use]
    pub fn equivalent(&self) -> bool {
        self.r#match == '='
    }

    /// RM `TERM_MAPPING.unknown()`: the kind of mapping is unknown
    /// (`match = '?'`).
    #[must_use]
    pub fn unknown(&self) -> bool {
        self.r#match == '?'
    }
}

impl Validate for TermMapping {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::term_mapping_core(self.r#match, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use openehr_base::v1_3::prelude::TerminologyId;

    fn term_mapping(m: char) -> TermMapping {
        TermMapping {
            r#match: m,
            purpose: None,
            target: CodePhrase {
                terminology_id: TerminologyId {
                    value: "SNOMED-CT".to_owned(),
                },
                code_string: "123".to_owned(),
                preferred_term: None,
            },
        }
    }

    #[test]
    fn valid_match_codes() {
        for m in ['<', '=', '>', '?'] {
            assert!(
                term_mapping(m).invariants().is_empty(),
                "{m} should be valid"
            );
        }
    }

    #[test]
    fn invalid_match_code() {
        let v = term_mapping('Q').invariants();
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].message,
            "Invariant Match_valid failed on type TERM_MAPPING"
        );
    }
}
