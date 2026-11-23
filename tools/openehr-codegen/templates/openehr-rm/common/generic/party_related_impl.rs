// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! `PARTY_RELATED` invariants (`org.openehr.rm.common.party_related.adoc`):
//! the inherited ones as `party_identified_impl`, and `Relationship_valid` in
//! [`crate::v1_2::validate::terminology`].

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
        assert_eq!(
            party(Some("Jane")).invariants(),
            Vec::<InvariantViolation>::new()
        );
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
