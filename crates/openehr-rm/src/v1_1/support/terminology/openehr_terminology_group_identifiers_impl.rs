// @generated-from-template templates/openehr-rm/support/terminology/openehr_terminology_group_identifiers_impl.rs — DO NOT EDIT; edit the source and re-run `openehr-codegen -- emit`.
// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0
//! Hand-written RM spec functions for `OPENEHR_TERMINOLOGY_GROUP_IDENTIFIERS`.
//!
//! Spec:
//! `docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.support.openehr_terminology_group_identifiers.adoc`
//! §Constants + §Functions.
//!
//! NOTE: the released text and the vendored BMM both type
//! `valid_terminology_group_id`'s `an_id` parameter as `Boolean` while its own
//! documentation calls it an identifier and the set it tests against is
//! fourteen `String` constants, so the parameter is realized as the string the
//! function's meaning requires.

use crate::v1_1::support::terminology::openehr_terminology_group_identifiers::OpenehrTerminologyGroupIdentifiersData;

impl OpenehrTerminologyGroupIdentifiersData {
    /// Every openEHR terminology group identifier this class defines, in
    /// declaration order.
    ///
    /// Spec:
    /// `org.openehr.rm.support.openehr_terminology_group_identifiers.adoc`
    /// §Constants. `Terminology_id_openehr` is deliberately absent: it names
    /// openEHR's own TERMINOLOGY, not a group within it, which is why
    /// `TERMINOLOGY_SERVICE.terminology` takes it and this predicate does not.
    const ALL: [&str; 14] = [
        Self::GROUP_ID_AUDIT_CHANGE_TYPE,
        Self::GROUP_ID_ATTESTATION_REASON,
        Self::GROUP_ID_COMPOSITION_CATEGORY,
        Self::GROUP_ID_EVENT_MATH_FUNCTION,
        Self::GROUP_ID_INSTRUCTION_STATES,
        Self::GROUP_ID_INSTRUCTION_TRANSITIONS,
        Self::GROUP_ID_NULL_FLAVOURS,
        Self::GROUP_ID_PROPERTY,
        Self::GROUP_ID_PARTICIPATION_FUNCTION,
        Self::GROUP_ID_PARTICIPATION_MODE,
        Self::GROUP_ID_SETTING,
        Self::GROUP_ID_TERM_MAPPING_PURPOSE,
        Self::GROUP_ID_SUBJECT_RELATIONSHIP,
        Self::GROUP_ID_VERSION_LIFE_CYCLE_STATE,
    ];

    /// Returns `true` when `an_id` is one of the openEHR terminology group
    /// identifiers.
    ///
    /// Spec:
    /// `org.openehr.rm.support.openehr_terminology_group_identifiers.adoc`
    /// §Functions `valid_terminology_group_id` — "Validity function to test if
    /// an identifier is in the set defined by this class", where "the set
    /// defined by this class" is its own §Constants.
    #[must_use]
    pub fn valid_terminology_group_id(an_id: &str) -> bool {
        Self::ALL.contains(&an_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Each declared group identifier is in the set.
    #[test]
    fn every_declared_group_identifier_is_valid() {
        for id in OpenehrTerminologyGroupIdentifiersData::ALL {
            assert!(
                OpenehrTerminologyGroupIdentifiersData::valid_terminology_group_id(id),
                "{id:?} is declared by the class"
            );
        }
        assert!(
            OpenehrTerminologyGroupIdentifiersData::valid_terminology_group_id(
                "version lifecycle state"
            )
        );
    }

    /// The terminology name is not a group name, and neither is anything
    /// outside the §Constants.
    #[test]
    fn anything_outside_the_constant_set_is_invalid() {
        for id in [
            "",
            OpenehrTerminologyGroupIdentifiersData::TERMINOLOGY_ID_OPENEHR,
            "Setting",
            "null flavour",
        ] {
            assert!(
                !OpenehrTerminologyGroupIdentifiersData::valid_terminology_group_id(id),
                "{id:?} is not a declared group identifier"
            );
        }
    }
}
