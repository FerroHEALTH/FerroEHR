// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `PLATFORM_SERVICE` (`master03-common_package.adoc` §Overview: "an
//! enumeration of the available services, used in various interfaces";
//! `platform_service.adoc`).

/// `PLATFORM_SERVICE` — "Enumeration of platform service names"
/// (`platform_service.adoc`).
///
/// The ADMIN statistics calls (`i_admin_service.adoc` `list_contributions` /
/// `contribution_count` / `versioned_composition_count` /
/// `composition_version_count`) each take a `PLATFORM_SERVICE` naming the
/// versioned-content service whose contributions/versions to count.
///
/// NOTE (spec defect): `platform_service.adoc` is named by the master03
/// §Overview but **not** `include::`d in its §Class Definitions, and its
/// enumeration lists exactly these eight members, omitting `Terminology` and
/// `Subject_proxy` (the SM defines both interfaces but the enum forgot their
/// members). This type carries the eight vendored members verbatim; the two
/// missing services are not versioned-content services and would count zero
/// regardless.
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
    /// The SM enumeration literal, exactly as `platform_service.adoc` spells
    /// it.
    #[must_use]
    pub fn sm_name(self) -> &'static str {
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

    /// All eight vendored members, in `platform_service.adoc` order.
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

    /// Parse an SM enumeration literal ASCII-case-insensitively.
    ///
    /// NOTE (`platform_service.adoc` fixes the literals but no openEHR spec
    /// governs how a member is spelled on a REST wire — the statistics calls
    /// have no released endpoint at all, so the transport is our own
    /// design/extension): the accepted spelling is the vendored literal, and
    /// the comparison is ASCII-case-insensitive for the same reason BASE
    /// `master05` §"Composite Identifiers and Case" gives for identifiers —
    /// case alone must not make two spellings name different things.
    ///
    /// # Errors
    /// `Err(())` when the text is not one of the eight vendored members; the
    /// caller decides the wire status.
    fn from_str(raw: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .iter()
            .copied()
            .find(|member| member.sm_name().eq_ignore_ascii_case(raw))
            .ok_or(())
    }
}

#[cfg(test)]
mod tests {
    use super::PlatformService;
    use std::str::FromStr;

    #[test]
    fn sm_names_match_the_vendored_literals() {
        assert_eq!(PlatformService::EhrIndex.sm_name(), "Ehr_index");
        assert_eq!(PlatformService::SystemLog.sm_name(), "System_log");
        assert_eq!(PlatformService::ALL.len(), 8);
    }

    #[test]
    fn parses_every_member_case_insensitively() {
        for member in PlatformService::ALL {
            assert_eq!(PlatformService::from_str(member.sm_name()), Ok(*member));
            assert_eq!(
                PlatformService::from_str(&member.sm_name().to_lowercase()),
                Ok(*member)
            );
        }
        assert_eq!(PlatformService::from_str("terminology"), Err(()));
    }
}
