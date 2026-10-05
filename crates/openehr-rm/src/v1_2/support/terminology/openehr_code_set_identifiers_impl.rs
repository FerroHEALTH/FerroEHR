// @generated-from-template templates/openehr-rm/support/terminology/openehr_code_set_identifiers_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `OPENEHR_CODE_SET_IDENTIFIERS`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.support.openehr_code_set_identifiers.adoc`
//! §Constants + §Functions.

use crate::v1_2::support::terminology::openehr_code_set_identifiers::OpenehrCodeSetIdentifiersData;

impl OpenehrCodeSetIdentifiersData {
    /// Every openEHR code set identifier this class defines, in declaration
    /// order.
    ///
    /// Spec: `org.openehr.rm.support.openehr_code_set_identifiers.adoc`
    /// §Constants — the seven identifiers, read from the emitted constants so
    /// there is no second copy of the list to drift.
    const ALL: [&str; 7] = [
        Self::CODE_SET_ID_CHARACTER_SETS,
        Self::CODE_SET_ID_COMPRESSION_ALGORITHMS,
        Self::CODE_SET_ID_COUNTRIES,
        Self::CODE_SET_INTEGRITY_CHECK_ALGORITHMS,
        Self::CODE_SET_ID_LANGUAGES,
        Self::CODE_SET_ID_MEDIA_TYPES,
        Self::CODE_SET_ID_NORMAL_STATUSES,
    ];

    /// Returns `true` when `an_id` is one of the openEHR code set identifiers.
    ///
    /// Spec: `org.openehr.rm.support.openehr_code_set_identifiers.adoc`
    /// §Functions `valid_code_set_id` — "Validity function to test if an
    /// identifier is in the set defined by this class", where "the set defined
    /// by this class" is its own §Constants. The comparison is exact: these
    /// identifiers are spec literals, and `TERMINOLOGY_SERVICE.code_set_for_id`
    /// takes this function as its precondition
    /// (`org.openehr.rm.support.terminology_service.adoc` §Functions), so a
    /// looser match would admit an id no code set answers to.
    #[must_use]
    pub fn valid_code_set_id(an_id: &str) -> bool {
        Self::ALL.contains(&an_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each of the seven §Constants is in the set.
    #[test]
    fn every_declared_identifier_is_valid() {
        for id in OpenehrCodeSetIdentifiersData::ALL {
            assert!(
                OpenehrCodeSetIdentifiersData::valid_code_set_id(id),
                "{id:?} is declared by the class"
            );
        }
        assert!(OpenehrCodeSetIdentifiersData::valid_code_set_id(
            "languages"
        ));
    }

    /// Nothing else is — including case variants and the near-misses an
    /// identifier is easily confused with.
    #[test]
    fn anything_outside_the_constant_set_is_invalid() {
        for id in ["", "Languages", "language", "media type", "openehr"] {
            assert!(
                !OpenehrCodeSetIdentifiersData::valid_code_set_id(id),
                "{id:?} is not declared by the class"
            );
        }
    }
}
