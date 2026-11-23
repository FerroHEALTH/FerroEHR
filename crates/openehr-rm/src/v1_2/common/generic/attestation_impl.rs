// @generated-from-template templates/openehr-rm/common/generic/attestation_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `ATTESTATION` invariants (`org.openehr.rm.common.attestation.adoc`,
//! `audit_details.adoc` §Invariants): `System_id_valid` here; `Change_type_valid`
//! and `Reason_valid` in [`crate::v1_2::validate::terminology`]; `Items_valid`
//! by the `NonEmptyVec` type.

use crate::v1_2::common::generic::attestation::Attestation;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Attestation {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.system_id.is_empty() {
            out.push(InvariantViolation::here(
                "Invariant System_id_valid failed on type ATTESTATION",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::v1_2::common::generic::party_proxy::PartyProxy;
    use crate::v1_2::common::generic::party_self::PartySelf;
    use crate::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_2::data_types::text::code_phrase::CodePhrase;
    use crate::v1_2::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_2::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_3::prelude::TerminologyId;
    use openehr_term::codes::AuditChangeType;

    fn coded() -> DvCodedText {
        DvCodedText {
            value: AuditChangeType::Creation.rubric().to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
            defining_code: CodePhrase {
                terminology_id: TerminologyId {
                    value: "openehr".to_owned(),
                },
                code_string: AuditChangeType::Creation.code().to_owned(),
                preferred_term: None,
            },
        }
    }

    fn attestation(system_id: &str) -> Attestation {
        Attestation {
            system_id: system_id.to_owned(),
            time_committed: DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                magnitude_status: None,
                accuracy: None,
                value: "2021-01-01T00:00:00".to_owned(),
            },
            change_type: coded(),
            description: None,
            committer: Box::new(PartyProxy::PartySelf(PartySelf { external_ref: None })),
            attested_view: None,
            proof: None,
            items: openehr_base::containers::present_nonempty(Vec::new()),
            reason: DvText::DvText(DvTextData {
                value: "witness".to_owned(),
                hyperlink: None,
                formatting: None,
                mappings: openehr_base::containers::present_nonempty(Vec::new()),
                language: None,
                encoding: None,
            }),
            is_pending: false,
        }
    }

    #[test]
    fn valid_attestation() {
        assert_eq!(
            attestation("system-1").invariants(),
            Vec::<InvariantViolation>::new()
        );
    }

    #[test]
    fn empty_system_id_invalid() {
        assert_eq!(
            attestation("").invariants()[0].message,
            "Invariant System_id_valid failed on type ATTESTATION"
        );
    }
}
