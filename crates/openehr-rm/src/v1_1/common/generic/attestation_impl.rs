// @generated-from-template templates/openehr-rm/common/generic/attestation_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariant for `ATTESTATION`.
//!
//! Spec: RM `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.attestation.adoc`
//! §Invariants declares two, and `ATTESTATION` inherits the two of
//! `…org.openehr.rm.common.audit_details.adoc` §Invariants. Each is realized,
//! but only one of the four is realizable on a typed node:
//!
//! - inherited `System_id_valid` (`not system_id.is_empty`) — realized here.
//! - inherited `Change_type_valid` (`change_type.defining_code` in the openEHR
//!   `audit change type` group) — terminology-bound, so it needs a bundle
//!   lookup rather than a typed-node property; realized by the binding table in
//!   [`crate::v1_1::validate::terminology`].
//! - `Reason_valid` (a `DV_CODED_TEXT` `reason` must code to the openEHR
//!   `attestation reason` group) — terminology-bound likewise, and realized in
//!   the same binding table.
//! - `Items_valid` (`items /= Void implies not items.is_empty`) — structural:
//!   `items` is `Option<NonEmptyVec<DvEhrUri>>`, whose `Deserialize` refuses an
//!   empty list at the door, so a violating value cannot be built or read.

use crate::v1_1::common::generic::attestation::Attestation;
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
    use crate::v1_1::common::generic::party_proxy::PartyProxy;
    use crate::v1_1::common::generic::party_self::PartySelf;
    use crate::v1_1::data_types::quantity::date_time::dv_date_time::DvDateTime;
    use crate::v1_1::data_types::text::code_phrase::CodePhrase;
    use crate::v1_1::data_types::text::dv_coded_text::DvCodedText;
    use crate::v1_1::data_types::text::dv_text::{DvText, DvTextData};
    use openehr_base::v1_2::prelude::TerminologyId;

    fn coded() -> DvCodedText {
        DvCodedText {
            value: "creation".to_owned(),
            hyperlink: None,
            formatting: None,
            mappings: openehr_base::containers::present_nonempty(Vec::new()),
            language: None,
            encoding: None,
            defining_code: CodePhrase {
                terminology_id: TerminologyId {
                    value: "openehr".to_owned(),
                },
                code_string: "249".to_owned(),
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
            committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
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
        assert!(attestation("system-1").invariants().is_empty());
    }

    #[test]
    fn empty_system_id_invalid() {
        assert_eq!(
            attestation("").invariants()[0].message,
            "Invariant System_id_valid failed on type ATTESTATION"
        );
    }
}
