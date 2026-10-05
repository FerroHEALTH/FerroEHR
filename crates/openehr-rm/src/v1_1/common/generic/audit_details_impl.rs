// @generated-from-template templates/openehr-rm/common/generic/audit_details_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM class invariant for `AUDIT_DETAILS`.
//!
//! Spec: RM `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.audit_details.adoc`
//! §Invariants declares two:
//!
//! - `System_id_valid`: `not system_id.is_empty` — realized here.
//! - `Change_type_valid`: `terminology (Terminology_id_openehr)
//!   .has_code_for_group_id (Group_id_audit_change_type,
//!   change_type.defining_code)` — terminology-bound, so it needs a bundle
//!   lookup rather than a typed-node property; realized by the binding table in
//!   [`crate::v1_1::validate::terminology`] against the `openehr-term` bundle.

use crate::v1_1::common::generic::audit_details::AuditDetailsData;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for AuditDetailsData {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.system_id.is_empty() {
            out.push(InvariantViolation::here(
                "Invariant System_id_valid failed on type AUDIT_DETAILS",
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
    use openehr_base::v1_2::prelude::TerminologyId;

    fn change_type() -> DvCodedText {
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

    fn audit(system_id: &str) -> AuditDetailsData {
        AuditDetailsData {
            system_id: system_id.to_owned(),
            time_committed: DvDateTime {
                normal_status: None,
                normal_range: None,
                other_reference_ranges: openehr_base::containers::present_nonempty(Vec::new()),
                magnitude_status: None,
                accuracy: None,
                value: "2021-01-01T00:00:00".to_owned(),
            },
            change_type: change_type(),
            description: None,
            committer: PartyProxy::PartySelf(PartySelf { external_ref: None }),
        }
    }

    #[test]
    fn valid_audit() {
        assert!(audit("system-1").invariants().is_empty());
    }

    #[test]
    fn empty_system_id_invalid() {
        assert_eq!(
            audit("").invariants()[0].message,
            "Invariant System_id_valid failed on type AUDIT_DETAILS"
        );
    }
}
