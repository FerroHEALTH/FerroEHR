// @generated-from-template templates/openehr-rm/common/generic/audit_details_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! `AUDIT_DETAILS` invariants (`org.openehr.rm.common.audit_details.adoc`):
//! `System_id_valid` here, `Change_type_valid` in
//! [`crate::v1_1::validate::terminology`].

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
    use openehr_term::codes::AuditChangeType;

    fn change_type() -> DvCodedText {
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
            committer: Box::new(PartyProxy::PartySelf(PartySelf { external_ref: None })),
        }
    }

    #[test]
    fn valid_audit() {
        assert_eq!(
            audit("system-1").invariants(),
            Vec::<InvariantViolation>::new()
        );
    }

    #[test]
    fn empty_system_id_invalid() {
        assert_eq!(
            audit("").invariants()[0].message,
            "Invariant System_id_valid failed on type AUDIT_DETAILS"
        );
    }
}
