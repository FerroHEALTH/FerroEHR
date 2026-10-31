// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The platform services whose activity the ADMIN report counts.

/// A versioned-content service of the platform, by name.
///
/// The activity-report extension (`list_contributions`, `contribution_count`,
/// `versioned_composition_count`, `composition_version_count`) takes one to
/// name the service whose contributions or versions it counts. No openEHR spec
/// governs the report: our own design/extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlatformService {
    /// `Admin` — the administrative service.
    Admin,
    /// `Definitions` — the DEFINITION (templates/archetypes/queries) service.
    Definitions,
    /// `Ehr` — the EHR (clinical, EHR-scoped) service.
    Ehr,
    /// `Ehr_index` — the EHR Index (subject↔EHR) service.
    EhrIndex,
    /// `Demographic` — the demographic (ehr-less party) service.
    Demographic,
    /// `Message` — the messaging service.
    Message,
    /// `Query` — the querying (AQL) service.
    Query,
    /// `System_log` — the system-log service.
    SystemLog,
}

impl PlatformService {
    /// The name of this service as the activity-report extension spells it.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "Admin",
            Self::Definitions => "Definitions",
            Self::Ehr => "Ehr",
            Self::EhrIndex => "Ehr_index",
            Self::Demographic => "Demographic",
            Self::Message => "Message",
            Self::Query => "Query",
            Self::SystemLog => "System_log",
        }
    }

    /// All eight members, in declaration order.
    pub const ALL: &[PlatformService] = &[
        Self::Admin,
        Self::Definitions,
        Self::Ehr,
        Self::EhrIndex,
        Self::Demographic,
        Self::Message,
        Self::Query,
        Self::SystemLog,
    ];
}

impl std::str::FromStr for PlatformService {
    type Err = ();

    /// Parse a service name ASCII-case-insensitively.
    ///
    /// NOTE: no openEHR spec governs the activity-report extension, so the
    /// spelling is our own; BASE `master05` §"Composite Identifiers and Case"
    /// is why case alone never makes two spellings name different things.
    ///
    /// # Errors
    /// `Err(())` when the text is not one of the eight members; the
    /// caller decides the wire status.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|member| member.as_str().eq_ignore_ascii_case(raw))
            .ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use super::PlatformService;
    use std::str::FromStr;

    #[test]
    fn names_keep_their_wire_spelling() {
        assert_eq!(PlatformService::EhrIndex.as_str(), "Ehr_index");
        assert_eq!(PlatformService::SystemLog.as_str(), "System_log");
        assert_eq!(PlatformService::ALL.len(), 8);
    }

    #[test]
    fn parses_every_member_case_insensitively() {
        for member in PlatformService::ALL {
            assert_eq!(PlatformService::from_str(member.as_str()), Ok(*member));
            assert_eq!(
                PlatformService::from_str(&member.as_str().to_lowercase()),
                Ok(*member)
            );
        }
        assert_eq!(PlatformService::from_str("terminology"), Err(()));
    }
}
