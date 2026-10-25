// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-FileCopyrightText: openEHR Foundation
// SPDX-License-Identifier: Apache-2.0

//! Named concepts of the openEHR terminology groups a version commit codes.
//!
//! Spec: TERM 3.1.0, the openEHR terminology groups `audit_change_type`
//! (`AUDIT_DETAILS.change_type`) and `version_lifecycle_state`
//! (`VERSION.lifecycle_state`), RM common `master06-change_control_package.adoc`.
//! Each code and English rubric is held equal to the embedded bundle
//! ([`crate::bundle::openehr`]) by the tests below, both ways, so a group
//! that gains or changes a concept fails the build here.

/// A concept of the `audit_change_type` group, the kind of change an
/// `AUDIT_DETAILS` records.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum AuditChangeType {
    /// `249` creation.
    Creation,
    /// `250` amendment.
    Amendment,
    /// `251` modification.
    Modification,
    /// `252` synthesis.
    Synthesis,
    /// `523` deleted.
    Deleted,
    /// `666` attestation.
    Attestation,
    /// `816` restoration.
    Restoration,
    /// `817` format conversion.
    FormatConversion,
    /// `253` unknown.
    Unknown,
}

impl AuditChangeType {
    /// The group id in the openEHR terminology.
    pub const GROUP_ID: &'static str = "audit_change_type";

    /// Every concept of the group, in the bundle's order.
    pub const ALL: [Self; 9] = [
        Self::Creation,
        Self::Amendment,
        Self::Modification,
        Self::Synthesis,
        Self::Deleted,
        Self::Attestation,
        Self::Restoration,
        Self::FormatConversion,
        Self::Unknown,
    ];

    /// The concept's code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Creation => "249",
            Self::Amendment => "250",
            Self::Modification => "251",
            Self::Synthesis => "252",
            Self::Deleted => "523",
            Self::Attestation => "666",
            Self::Restoration => "816",
            Self::FormatConversion => "817",
            Self::Unknown => "253",
        }
    }

    /// The concept's English rubric.
    #[must_use]
    pub const fn rubric(self) -> &'static str {
        match self {
            Self::Creation => "creation",
            Self::Amendment => "amendment",
            Self::Modification => "modification",
            Self::Synthesis => "synthesis",
            Self::Deleted => "deleted",
            Self::Attestation => "attestation",
            Self::Restoration => "restoration",
            Self::FormatConversion => "format conversion",
            Self::Unknown => "unknown",
        }
    }

    /// The concept a code names in this group, `None` for a code of no
    /// concept here.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.code() == code)
    }
}

/// A concept of the `version_lifecycle_state` group, the state of a `VERSION`.
///
/// NOTE: SPECPR-51, the rubric of `532` is `complete` here and `completed` in `instruction_states`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum VersionLifecycleState {
    /// `532` complete.
    Complete,
    /// `553` incomplete.
    Incomplete,
    /// `523` deleted.
    Deleted,
    /// `800` inactive.
    Inactive,
    /// `801` abandoned.
    Abandoned,
}

impl VersionLifecycleState {
    /// The group id in the openEHR terminology.
    pub const GROUP_ID: &'static str = "version_lifecycle_state";

    /// Every concept of the group, in the bundle's order.
    pub const ALL: [Self; 5] = [
        Self::Complete,
        Self::Incomplete,
        Self::Deleted,
        Self::Inactive,
        Self::Abandoned,
    ];

    /// The concept's code.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Complete => "532",
            Self::Incomplete => "553",
            Self::Deleted => "523",
            Self::Inactive => "800",
            Self::Abandoned => "801",
        }
    }

    /// The concept's English rubric.
    #[must_use]
    pub const fn rubric(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
            Self::Deleted => "deleted",
            Self::Inactive => "inactive",
            Self::Abandoned => "abandoned",
        }
    }

    /// The concept a code names in this group, `None` for a code of no
    /// concept here.
    #[must_use]
    pub fn from_code(code: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.code() == code)
    }
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

    #[test]
    fn the_change_types_are_the_bundles() {
        let named: Vec<(String, String)> = AuditChangeType::ALL
            .iter()
            .map(|c| (c.code().to_owned(), c.rubric().to_owned()))
            .collect();
        assert_eq!(named, bundle_group(AuditChangeType::GROUP_ID));
    }

    #[test]
    fn the_lifecycle_states_are_the_bundles() {
        let named: Vec<(String, String)> = VersionLifecycleState::ALL
            .iter()
            .map(|c| (c.code().to_owned(), c.rubric().to_owned()))
            .collect();
        assert_eq!(named, bundle_group(VersionLifecycleState::GROUP_ID));
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
    }
}
