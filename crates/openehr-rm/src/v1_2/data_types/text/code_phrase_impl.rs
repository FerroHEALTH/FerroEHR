// @generated-from-template templates/openehr-rm/data_types/text/code_phrase_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariant for `CODE_PHRASE`.
//!
//! `Code_string_valid` (`not code_string.is_empty`) —
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.data_types.code_phrase.adoc`
//! §Invariants.

use crate::v1_2::data_types::text::code_phrase::CodePhrase;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for CodePhrase {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::code_phrase_core(&self.code_string, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::v1_3::prelude::TerminologyId;

    fn code_phrase(code: &str) -> CodePhrase {
        CodePhrase {
            terminology_id: TerminologyId {
                value: "local".to_owned(),
            },
            code_string: code.to_owned(),
            preferred_term: None,
        }
    }

    #[test]
    fn valid_code_string() {
        assert!(code_phrase("at0001").invariants().is_empty());
    }

    #[test]
    fn empty_code_string_invalid() {
        let v = code_phrase("").invariants();
        assert_eq!(v.len(), 1);
        assert_eq!(
            v[0].message,
            "Invariant Code_string_valid failed on type CODE_PHRASE"
        );
    }
}
