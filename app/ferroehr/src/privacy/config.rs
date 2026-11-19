// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[privacy]` section — the clinical-side data-minimisation policy.
//!
//! **No openEHR spec governs this — our own design/extension.** The openEHR RM
//! leaves `EHR_STATUS.subject.external_ref` open and only advises against
//! identifying content on a party proxy (RM common
//! `UML/classes/org.openehr.rm.common.party_identified.adoc` §Description:
//! "Should not be used to include patient identifying information"); GDPR
//! Art. 4(5) and Art. 25(2) make that a duty
//! (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>). This section is where a
//! deployment states its side of it.
//!
//! A field of the one config tree ([`crate::config::FerroEhrConfig`]); no
//! loader of its own.

#![allow(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the `schema` feature's schemars derive expands to \
              serde_json maps; without the feature nothing here names one"
)]

use serde::{Deserialize, Serialize};

/// The clinical-side data-minimisation policy (`[privacy]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct PrivacyConfig {
    /// The namespaces an `EHR_STATUS.subject.external_ref` may name
    /// (`FERROEHR__PRIVACY__SUBJECT_NAMESPACES`, comma-separated in the
    /// environment) — the pseudonymisation domains this deployment issues
    /// subject pseudonyms in.
    ///
    /// Empty (the default) leaves the subject-reference rule out of force:
    /// a pseudonym namespace is a deployment fact — which service mints the
    /// tokens — and there is no name a server could invent for an operator.
    /// Declaring one is the same act as turning the rule on: from then on a
    /// subject reference must name a listed namespace and carry a UUID as its
    /// `id.value`.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-must-set" = true)))]
    pub subject_namespaces: Vec<String>,
    /// Accept `PARTY_IDENTIFIED` / `PARTY_RELATED` carrying `name` or
    /// `identifiers` in clinical content
    /// (`FERROEHR__PRIVACY__ALLOW_IDENTIFIED_PARTIES_IN_EHR`).
    ///
    /// Off by default, and announced at boot when on. Off refuses two of the
    /// four combinations: `identifiers` on either class — "One or more formal
    /// identifiers (possibly computable)" (RM common
    /// `UML/classes/org.openehr.rm.common.party_identified.adoc` §Attributes),
    /// the national-identifier slot — and `name` on `PARTY_RELATED`, which is
    /// defined by its relationship to the subject and may be the subject
    /// itself (RM common `UML/classes/org.openehr.rm.common.party_related.adoc`
    /// §Attributes). A `PARTY_IDENTIFIED.name` is accepted either way: that
    /// class is the proxy for a party "other than the subject of the record",
    /// "Typically for health care providers". `Basic_validity` is satisfied by
    /// `external_ref` alone, so the refused shapes stay expressible.
    pub allow_identified_parties_in_ehr: bool,
    /// `[privacy.identifier_protection]` — the national-identifier
    /// protection of the demographic domain; absent, off.
    pub identifier_protection:
        Option<crate::identifier_protection::config::IdentifierProtectionConfig>,
    /// `[privacy.cohort]` — the cross-domain cohort-query allow-list and
    /// limits.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub cohort: crate::service::linkage::cohort::config::CohortConfig,
}
