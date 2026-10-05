// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariant for `FEEDER_AUDIT_DETAILS`.
//!
//! `System_id_valid` (`not system_id.is_empty`) —
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.feeder_audit_details.adoc`
//! §Invariants.

use crate::v1_2::common::archetyped::feeder_audit_details::FeederAuditDetails;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for FeederAuditDetails {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        if self.system_id.is_empty() {
            out.push(InvariantViolation::here(
                "Invariant System_id_valid failed on type FEEDER_AUDIT_DETAILS",
            ));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn details(system_id: &str) -> FeederAuditDetails {
        FeederAuditDetails {
            system_id: system_id.to_owned(),
            location: None,
            subject: None,
            provider: None,
            time: None,
            version_id: None,
            other_details: None,
        }
    }

    #[test]
    fn valid_details() {
        assert!(details("legacy-system").invariants().is_empty());
    }

    #[test]
    fn empty_system_id_invalid() {
        assert_eq!(
            details("").invariants()[0].message,
            "Invariant System_id_valid failed on type FEEDER_AUDIT_DETAILS"
        );
    }
}
