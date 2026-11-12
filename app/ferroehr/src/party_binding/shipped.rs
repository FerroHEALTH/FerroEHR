// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The default demographic definitions FerroEHR ships, and the ids the
//! default party mapping names.
//!
//! The binary carries the files of `deploy/definitions/` ([`DEFINITIONS`]),
//! and every boot stores each one whose id no stored definition holds, as it
//! stores the policy definitions ([`crate::party_binding::policy`]). With no
//! mapping key set, `[authz.party_binding]` maps them
//! ([`crate::config::binding::PartyBindingConfig`]'s `Default`), so a zero
//! configuration boots on them. A deployment with templates of its own sets
//! the mapping keys, and nothing is stored under its ids. No openEHR spec
//! governs seeding definitions or a default mapping: our own design.

/// The professional's `PERSON` template.
pub const PROFESSIONAL_PERSON: &str = "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0";

/// The professional `ROLE` template.
pub const PROFESSIONAL_ROLE: &str = "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0";

/// The care-team `GROUP` template.
pub const CARE_TEAM: &str = "openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0";

/// The membership `PARTY_RELATIONSHIP` archetype, in its interface form.
pub const MEMBERSHIP: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1";

/// The representation `PARTY_RELATIONSHIP` archetype, in its interface form.
pub const REPRESENTATION: &str = "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1";

/// The patient's `PERSON` template.
pub const PATIENT: &str = "openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0";

/// The default demographic definitions, each its file name and its ADL 2
/// text, in the order the book's "Default demographic definitions" page
/// lists them.
pub const DEFINITIONS: [(&str, &str); 7] = [
    (
        "openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls",
        include_str!(
            "../../../../deploy/definitions/professional/openEHR-DEMOGRAPHIC-PERSON.healthcare_professional.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls",
        include_str!(
            "../../../../deploy/definitions/professional/openEHR-DEMOGRAPHIC-ROLE.healthcare_professional.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls",
        include_str!(
            "../../../../deploy/definitions/care-team/openEHR-DEMOGRAPHIC-GROUP.care_team.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls",
        include_str!(
            "../../../../deploy/definitions/care-team/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.membership.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls",
        include_str!(
            "../../../../deploy/definitions/representation/openEHR-DEMOGRAPHIC-PARTY_RELATIONSHIP.representation.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls",
        include_str!(
            "../../../../deploy/definitions/patient/openEHR-DEMOGRAPHIC-PERSON.patient.v1.0.0.adls"
        ),
    ),
    (
        "openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0.adls",
        include_str!(
            "../../../../deploy/definitions/provider/openEHR-DEMOGRAPHIC-ORGANISATION.healthcare_provider.v1.0.0.adls"
        ),
    ),
];
