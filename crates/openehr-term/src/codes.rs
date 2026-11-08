// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Named concepts of openEHR terminology groups.
//!
//! Spec: TERM 3.1.0, the openEHR terminology groups `audit_change_type`
//! (`AUDIT_DETAILS.change_type`) and `version_lifecycle_state`
//! (`VERSION.lifecycle_state`), RM common `master06-change_control_package.adoc`;
//! `null_flavours` (`ELEMENT.null_flavour`, RM data_structures
//! `org.openehr.rm.data_structures.representation.element.adoc`); and `setting`
//! (`EVENT_CONTEXT.setting`, RM composition
//! `org.openehr.rm.composition.event_context.adoc`). Each code and English rubric
//! is held equal to the embedded bundle ([`crate::bundle::openehr`]) by the tests
//! below, both ways, so a group that gains or changes a concept fails the build
//! here.

/// Declares one terminology group as an enum with its group id, its concepts
/// in the bundle's order, and each concept's code and English rubric.
macro_rules! group {
    (
        $(#[$meta:meta])*
        $name:ident, $group:literal, [$($variant:ident = $code:literal, $rubric:literal;)+]
    ) => {
        $(#[$meta])*
        #[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
        pub enum $name {
            $(
                #[doc = concat!("`", $code, "` ", $rubric, ".")]
                $variant,
            )+
        }

        impl $name {
            /// The group id in the openEHR terminology.
            pub const GROUP_ID: &str = $group;

            /// Every concept of the group, in the bundle's order.
            pub const ALL: &[Self] = &[$(Self::$variant),+];

            /// The concept's code.
            #[must_use]
            pub const fn code(self) -> &'static str {
                match self {
                    $(Self::$variant => $code,)+
                }
            }

            /// The concept's English rubric.
            #[must_use]
            pub const fn rubric(self) -> &'static str {
                match self {
                    $(Self::$variant => $rubric,)+
                }
            }

            /// The concept a code names in this group, `None` for a code of no
            /// concept here.
            #[must_use]
            pub fn from_code(code: &str) -> Option<Self> {
                Self::ALL.iter().copied().find(|c| c.code() == code)
            }
        }
    };
}

group! {
    /// A concept of the `audit_change_type` group, the kind of change an
    /// `AUDIT_DETAILS` records.
    AuditChangeType, "audit_change_type", [
        Creation = "249", "creation";
        Amendment = "250", "amendment";
        Modification = "251", "modification";
        Synthesis = "252", "synthesis";
        Deleted = "523", "deleted";
        Attestation = "666", "attestation";
        Restoration = "816", "restoration";
        FormatConversion = "817", "format conversion";
        Unknown = "253", "unknown";
    ]
}

group! {
    /// A concept of the `version_lifecycle_state` group, the state of a `VERSION`.
    ///
    /// NOTE: SPECPR-51, the rubric of `532` is `complete` here and `completed` in `instruction_states`.
    VersionLifecycleState, "version_lifecycle_state", [
        Complete = "532", "complete";
        Incomplete = "553", "incomplete";
        Deleted = "523", "deleted";
        Inactive = "800", "inactive";
        Abandoned = "801", "abandoned";
    ]
}

group! {
    /// A concept of the `null_flavours` group, why an `ELEMENT` carries no
    /// value (`ELEMENT.null_flavour`).
    NullFlavour, "null_flavours", [
        NoInformation = "271", "no information";
        Unknown = "253", "unknown";
        Masked = "272", "masked";
        NotApplicable = "273", "not applicable";
    ]
}

group! {
    /// A concept of the `setting` group, the care setting of an
    /// `EVENT_CONTEXT` (`EVENT_CONTEXT.setting`).
    Setting, "setting", [
        Home = "225", "home";
        EmergencyCare = "227", "emergency care";
        PrimaryMedicalCare = "228", "primary medical care";
        PrimaryNursingCare = "229", "primary nursing care";
        PrimaryAlliedHealthCare = "230", "primary allied health care";
        MidwiferyCare = "231", "midwifery care";
        SecondaryMedicalCare = "232", "secondary medical care";
        SecondaryNursingCare = "233", "secondary nursing care";
        SecondaryAlliedHealthCare = "234", "secondary allied health care";
        ComplementaryHealthCare = "235", "complementary health care";
        DentalCare = "236", "dental care";
        NursingHomeCare = "237", "nursing home care";
        MentalHealthcare = "802", "mental healthcare";
        OtherCare = "238", "other care";
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bundle::openehr;

    /// The `(code, rubric)` pairs of `group` in the bundle, in its order.
    fn bundle_group(group: &str) -> Vec<(String, String)> {
        openehr()
            .concepts_in_group(group)
            .iter()
            .map(|c| (c.id.clone(), c.rubric.clone()))
            .collect()
    }

    /// The `(code, rubric)` pairs a typed group names, in its order.
    fn named<T: Copy>(
        all: &[T],
        code: fn(T) -> &'static str,
        rubric: fn(T) -> &'static str,
    ) -> Vec<(String, String)> {
        all.iter()
            .map(|&c| (code(c).to_owned(), rubric(c).to_owned()))
            .collect()
    }

    #[test]
    fn the_change_types_are_the_bundles() {
        let typed = named(
            AuditChangeType::ALL,
            AuditChangeType::code,
            AuditChangeType::rubric,
        );
        assert_eq!(typed, bundle_group(AuditChangeType::GROUP_ID));
    }

    #[test]
    fn the_lifecycle_states_are_the_bundles() {
        let typed = named(
            VersionLifecycleState::ALL,
            VersionLifecycleState::code,
            VersionLifecycleState::rubric,
        );
        assert_eq!(typed, bundle_group(VersionLifecycleState::GROUP_ID));
    }

    #[test]
    fn the_null_flavours_are_the_bundles() {
        let typed = named(NullFlavour::ALL, NullFlavour::code, NullFlavour::rubric);
        assert_eq!(typed, bundle_group(NullFlavour::GROUP_ID));
    }

    #[test]
    fn the_settings_are_the_bundles() {
        let typed = named(Setting::ALL, Setting::code, Setting::rubric);
        assert_eq!(typed, bundle_group(Setting::GROUP_ID));
    }

    #[test]
    fn a_code_reads_back_to_its_concept() {
        assert_eq!(
            AuditChangeType::from_code("251"),
            Some(AuditChangeType::Modification)
        );
        assert_eq!(
            VersionLifecycleState::from_code("523"),
            Some(VersionLifecycleState::Deleted)
        );
        assert_eq!(AuditChangeType::from_code("532"), None);
        assert_eq!(NullFlavour::from_code("272"), Some(NullFlavour::Masked));
        assert_eq!(
            Setting::from_code("238").map(Setting::rubric),
            Some("other care")
        );
        assert_eq!(Setting::from_code("253"), None);
    }
}
