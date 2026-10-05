// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariants for `RESOURCE_DESCRIPTION`.
//!
//! Spec: RM 1.2.0
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.resource_description.adoc`
//! §Invariants — `Original_author_valid`, `Lifecycle_state_valid`,
//! `Details_valid` (the own non-empty rules, evaluated by the generated
//! core). `Language_valid` and `Parent_resource_valid` read the OWNING
//! `parent_resource` back-reference, which the generated data model
//! deliberately does not carry (back-references are broken at emission):
//! `Language_valid` is realized at the whole-resource ingest seams, where
//! the owner is in hand (`app/ferroehr/src/validation/opt/resource.rs`;
//! `openehr-adl` `validate/resource_meta.rs`), and `Parent_resource_valid`
//! is excluded — both rows live in the generated complex-invariant
//! register.

use crate::v1_2::common::resource::resource_description::ResourceDescription;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for ResourceDescription {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::resource_description_core(
            self.original_author.is_empty(),
            &self.lifecycle_state,
            self.details.is_empty(),
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn description() -> ResourceDescription {
        ResourceDescription {
            original_author: std::iter::once(("name".to_owned(), "author".to_owned())).collect(),
            other_contributors: None,
            lifecycle_state: "unmanaged".to_owned(),
            resource_package_uri: None,
            other_details: None,
            details: std::iter::once((
                "en".to_owned(),
                crate::v1_2::common::resource::resource_description_item::ResourceDescriptionItem {
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
                },
            ))
            .collect(),
        }
    }

    #[test]
    fn valid_description_passes() {
        assert!(description().invariants().is_empty());
    }

    #[test]
    fn each_empty_member_is_its_own_violation() {
        let mut d = description();
        d.original_author.clear();
        d.lifecycle_state.clear();
        d.details.clear();
        let v = d.invariants();
        for inv in [
            "Original_author_valid",
            "Lifecycle_state_valid",
            "Details_valid",
        ] {
            assert!(
                v.iter().any(|m| m.message.contains(inv)),
                "{inv} must be reported, got {v:?}"
            );
        }
    }
}
