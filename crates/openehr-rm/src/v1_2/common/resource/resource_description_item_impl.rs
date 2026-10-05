// @generated-from-template templates/openehr-rm/common/resource/resource_description_item_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `RESOURCE_DESCRIPTION_ITEM`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.resource_description_item.adoc`
//! §Invariants — `Purpose_valid` plus the present-implies-non-empty string
//! rules `Use_valid`, `misuse_valid`, `copyright_valid` (the generated core).
//! `Language_valid` is terminology-backed (the language code set) and stays
//! with the terminology binding table.

use crate::v1_2::common::resource::resource_description_item::ResourceDescriptionItem;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for ResourceDescriptionItem {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::resource_description_item_core(
            &self.purpose,
            self.use_.as_deref(),
            self.misuse.as_deref(),
            self.copyright.as_deref(),
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item() -> ResourceDescriptionItem {
        ResourceDescriptionItem {
            language: crate::v1_2::data_types::text::code_phrase::CodePhrase {
                terminology_id:
                    openehr_base::v1_3::base_types::identification::terminology_id::TerminologyId {
                        value: "ISO_639-1".to_owned(),
                    },
                code_string: "en".to_owned(),
                preferred_term: None,
            },
            purpose: "testing".to_owned(),
            keywords: None,
            use_: None,
            misuse: None,
            copyright: None,
            original_resource_uri: None,
            other_details: None,
        }
    }

    #[test]
    fn valid_item_passes_and_absent_optionals_are_legal() {
        assert!(item().invariants().is_empty());
    }

    #[test]
    fn empty_purpose_and_present_but_empty_optionals_are_violations() {
        let mut i = item();
        i.purpose.clear();
        i.use_ = Some(String::new());
        i.misuse = Some(String::new());
        i.copyright = Some(String::new());
        let v = i.invariants();
        for inv in [
            "Purpose_valid",
            "Use_valid",
            "misuse_valid",
            "copyright_valid",
        ] {
            assert!(
                v.iter().any(|m| m.message.contains(inv)),
                "{inv} must be reported, got {v:?}"
            );
        }
        // Populated optionals pass.
        let mut ok = item();
        ok.use_ = Some("clinical care".to_owned());
        assert!(ok.invariants().is_empty());
    }
}
