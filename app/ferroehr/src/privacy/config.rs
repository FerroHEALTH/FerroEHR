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
//! deployment states its side of it — including which jurisdictions' personal
//! identifiers its data could carry, because openEHR is deployed across many
//! and no build can know which.
//!
//! A field of the one config tree ([`crate::config::FerroEhrConfig`]); no
//! loader of its own.

use serde::{Deserialize, Serialize};

use crate::privacy::detect::built_in_rules;

/// What the identifier scanner does with a clinical write that carries a value
/// one of its rules claims.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ScanMode {
    /// Refuse the write, naming the RM path and the rule that matched. The
    /// default: a scanner that only observes is a scanner nobody acts on.
    #[default]
    Strict,
    /// Accept the write and record a warning naming the RM path and the rule.
    /// For a deployment measuring its own content before turning refusal on.
    Warn,
}

impl ScanMode {
    /// The mode's configuration spelling, for boot logs and diagnostics.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Strict => "strict",
            Self::Warn => "warn",
        }
    }
}

/// The identifier scanner (`[privacy.identifier_scan]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct IdentifierScanConfig {
    /// `strict` (refuse) or `warn` (accept and record)
    /// (`FERROEHR__PRIVACY__IDENTIFIER_SCAN__MODE`).
    pub mode: ScanMode,
    /// The national personal-identifier rules this deployment scans for, by
    /// rule key (`FERROEHR__PRIVACY__IDENTIFIER_SCAN__RULES`, comma-separated
    /// in the environment). An unknown key is a boot error naming it.
    ///
    /// Defaults to EVERY rule the build ships, not to none and not to one
    /// jurisdiction. An operator who has not configured the scanner is exactly
    /// the operator who has not yet worked out which identifiers their content
    /// carries, and clinical data crosses borders: a Dutch hospital receives
    /// referrals carrying a Norwegian fødselsnummer. The two costs are not
    /// symmetric — a false positive is a loud `422` naming the RM path and the
    /// rule, which an operator answers by narrowing this list or moving to
    /// [`ScanMode::Warn`], while a false negative is a national identifier
    /// stored on the clinical side indefinitely. A deployment that knows its
    /// jurisdictions narrows the list; nothing is silently assumed for it.
    pub rules: Vec<String>,
    /// Extra identifier patterns this deployment refuses, as Rust regular
    /// expressions (`FERROEHR__PRIVACY__IDENTIFIER_SCAN__PATTERNS`,
    /// comma-separated in the environment).
    ///
    /// For the kinds no build can ship a rule for: local medical-record
    /// numbers, payer references, national postcode-plus-house-number forms.
    /// Empty by default — none of these has an issuing register whose
    /// definition could be transcribed. Each is compiled at boot; an
    /// uncompilable one is a boot error.
    pub patterns: Vec<String>,
}

impl Default for IdentifierScanConfig {
    fn default() -> Self {
        Self {
            mode: ScanMode::Strict,
            rules: built_in_rules()
                .iter()
                .map(|rule| rule.key.to_owned())
                .collect(),
            patterns: Vec::new(),
        }
    }
}

/// The clinical-side data-minimisation policy (`[privacy]`).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
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
    /// `[privacy.identifier_scan]` — the identifier scanner.
    pub identifier_scan: IdentifierScanConfig,
}
