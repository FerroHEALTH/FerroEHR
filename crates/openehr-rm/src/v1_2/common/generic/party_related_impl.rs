// @generated-from-template templates/openehr-rm/common/generic/party_related_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariants for `PARTY_RELATED`.
//!
//! Spec: RM `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.party_related.adoc`
//! §Invariants declares one, and `PARTY_RELATED` inherits those of
//! `…org.openehr.rm.common.party_identified.adoc` §Invariants:
//!
//! - inherited `Basic_validity` and `Name_valid` — realized here, through the
//!   generated `party_identified_core`; the inherited `Identifiers_valid` is
//!   structural, for the reason given in `party_identified_impl`.
//! - `Relationship_valid`: `terminology (Terminology_id_openehr)
//!   .has_code_for_group_id (Group_id_subject_relationship,
//!   relationship.defining_code)` — terminology-bound, so it needs a bundle
//!   lookup rather than a typed-node property; realized by the binding table in
//!   [`crate::v1_2::validate::terminology`] against the `openehr-term` bundle.

use crate::v1_2::common::generic::party_related::PartyRelated;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for PartyRelated {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::party_identified_core(
            "PARTY_RELATED",
            self.name.as_deref(),
            self.identifiers.is_some(),
            self.external_ref.is_some(),
            out,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use openehr_base::v1_3::prelude::TerminologyId;

    fn relationship() -> DvCodedText {
        DvCodedText {
            value: "mother".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
            defining_code: CodePhrase {
                terminology_id: TerminologyId {
                    value: "openehr".to_owned(),
                },
                code_string: "10".to_owned(),
                preferred_term: None,
            },
        }
    }

    fn party(name: Option<&str>) -> PartyRelated {
        PartyRelated {
            external_ref: None,
            name: name.map(str::to_owned),
            identifiers: openehr_base::containers::present_nonempty(Vec::new()),
            relationship: relationship(),
        }
    }

    #[test]
    fn named_party_valid() {
        assert!(party(Some("Jane")).invariants().is_empty());
    }

    #[test]
    fn no_identity_invalid() {
        let v = party(None).invariants();
        assert!(
            v.iter()
                .any(|m| m.message == "Invariant Basic_validity failed on type PARTY_RELATED")
        );
    }
}
