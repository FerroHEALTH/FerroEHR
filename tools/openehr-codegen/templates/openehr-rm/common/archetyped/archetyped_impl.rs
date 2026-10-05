// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Hand-written RM class invariant for `ARCHETYPED`.
//!
//! `Rm_version_valid` (`not rm_version.is_empty`) —
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.archetyped.adoc`
//! §Invariants.

use crate::v1_2::common::archetyped::archetyped::Archetyped;
use openehr_base::validate::{InvariantViolation, Validate};

impl Validate for Archetyped {
    fn validate_invariants(&self, out: &mut Vec<InvariantViolation>) {
        crate::v1_2::validate::generated::archetyped_core(&self.rm_version, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_base::v1_3::prelude::ArchetypeId;

    fn archetyped(rm_version: &str) -> Archetyped {
        Archetyped {
            archetype_id: ArchetypeId {
                value: "openEHR-EHR-COMPOSITION.example.v1".to_owned(),
            },
            template_id: None,
            rm_version: rm_version.to_owned(),
        }
    }

    #[test]
    fn valid_rm_version() {
        assert!(archetyped("1.1.0").invariants().is_empty());
    }

    #[test]
    fn empty_rm_version_invalid() {
        assert_eq!(
            archetyped("").invariants()[0].message,
            "Invariant Rm_version_valid failed on type ARCHETYPED"
        );
    }
}
