// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The clinical-side data-minimisation policy: what content the `ehr` domain
//! refuses to hold.
//!
//! **No openEHR spec governs this — our own design/extension.** The RM makes
//! the room for it and points the same way: an EHR carries its subject as a
//! `PARTY_SELF` whose optional `external_ref` is a `PARTY_REF` into a
//! demographic service and nothing else identifying (RM ehr
//! `UML/classes/org.openehr.rm.ehr.ehr_status.adoc` §Attributes; RM common
//! `UML/classes/org.openehr.rm.common.party_self.adoc`), and
//! `PARTY_IDENTIFIED` says of itself "Should not be used to include patient
//! identifying information", a sentence the same paragraph scopes: the class
//! covers a party "other than the subject of the record", with health care
//! providers as its typical case (RM common
//! `UML/classes/org.openehr.rm.common.party_identified.adoc` §Description).
//! Neither is a machine-checkable rule, so this module is where FerroEHR makes
//! them ones. The legal ground is GDPR Art. 4(5) and Art. 25(2)
//! (<https://eur-lex.europa.eu/eli/reg/2016/679/oj>) with the Dutch UAVG
//! Art. 46 and the Wabvpz over the BSN specifically
//! (<https://wetten.overheid.nl/BWBR0040940>,
//! <https://wetten.overheid.nl/BWBR0023864>).
//!
//! Three rules, all evaluated over the canonical JSON of a clinical commit
//! body before it is stored:
//!
//! 1. **The subject reference.** `EHR_STATUS.subject.external_ref` must name a
//!    configured pseudonym namespace and carry a UUID. In force only once the
//!    deployment declares its namespaces
//!    ([`config::PrivacyConfig::subject_namespaces`]).
//! 2. **Identified parties.** In clinical content a `PARTY_IDENTIFIED` may
//!    carry `name` and `identifiers` — the class is explicitly the provider
//!    proxy, "other than the subject of the record", whose paradigm case is
//!    "name and provider number of an institution" — and so may a
//!    `PARTY_RELATED`, unless its relationship codes `self`, where the party
//!    IS the subject and both are refused. The deployment can opt in to the
//!    `self` case. `Basic_validity` is satisfied by `external_ref` alone, so
//!    the refused proxy stays expressible; a subject's identifier VALUE is
//!    the scanner's job (rule 3) wherever it sits.
//! 3. **The identifier scanner.** No string leaf may carry a value one of the
//!    active identifier rules claims. The rules are keyed by jurisdiction and
//!    each transcribes the checksum its own issuing register publishes; a
//!    deployment selects which are active and adds patterns for the local
//!    kinds no build can ship a rule for ([`detect`]).
//!
//! A finding never carries the offending VALUE, only its RM path and the rule
//! that matched: a refusal travels into an error body, an access log and a
//! trace, and echoing a national identifier into all three to complain about it
//! would be the leak the rule exists to prevent.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior — this pass reads the \
              canonical JSON body the write path already holds, the same seam \
              service/ehr/validation.rs carries"
)]

pub mod config;
pub mod detect;

use serde_json::Value;

use crate::privacy::config::{PrivacyConfig, ScanMode};
use crate::privacy::detect::{CustomPattern, IdentifierRule};

/// The JSON key whose value is a terminology code by construction, and which
/// the identifier scanner therefore skips.
///
/// `code_string` is the only string attribute of `CODE_PHRASE` (BASE `base_types`
/// `master06-terminology_package.adoc` §Class Definitions; `terminology_id` is
/// an object), and no other RM class declares an attribute of that name, so
/// the key alone identifies the slot. Every checksum rule accepts some fraction
/// of random digit runs, and a terminology code system's identifiers are digit
/// runs by construction, so this one slot is where the collision is systematic
/// rather than incidental. The carve-out is structural and jurisdiction-neutral:
/// it names an RM slot, not a country.
pub(crate) const TERMINOLOGY_CODE_KEY: &str = "code_string";

/// What a clinical commit body carried that the policy refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The RM instance path of the offending node.
    pub path: String,
    /// What is wrong, naming the shape and never the value.
    pub message: String,
    /// Whether this finding is a refusal or, under
    /// [`ScanMode::Warn`], a recorded warning.
    pub class: FindingClass,
}

/// Which rule produced a [`Finding`], and therefore whether the scan mode
/// applies to it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FindingClass {
    /// The subject-reference rule or the identified-party rule: an
    /// unconditional refusal whenever the rule is in force.
    Refusal,
    /// The identifier scanner: refused under [`ScanMode::Strict`], recorded as
    /// an audit warning under [`ScanMode::Warn`].
    IdentifierShape,
}

/// The compiled runtime form of [`PrivacyConfig`].
///
/// Built once at boot (the MRN patterns are compiled there, so an uncompilable
/// one is a configuration error rather than a per-write surprise) and held by
/// the service.
#[derive(Debug, Clone)]
pub struct PrivacyPolicy {
    subject_namespaces: Vec<String>,
    allow_identified_parties: bool,
    scan_mode: Option<ScanMode>,
    rules: Vec<&'static IdentifierRule>,
    patterns: Vec<CustomPattern>,
}

impl Default for PrivacyPolicy {
    /// The refusing posture, the same one [`PrivacyConfig::default`] compiles
    /// to: identified `self` parties refused, every shipped identifier rule in
    /// `strict` mode, no namespaces declared (#3243).
    ///
    /// A `FerroEhrService` built without [`crate::service::FerroEhrService::with_privacy`]
    /// therefore runs the posture the shipped binary runs, never a more
    /// permissive one: a test or an embedding host that means to accept more
    /// installs the policy that says so.
    fn default() -> Self {
        Self {
            subject_namespaces: Vec::new(),
            allow_identified_parties: false,
            scan_mode: Some(ScanMode::Strict),
            rules: detect::built_in_rules().iter().collect(),
            patterns: Vec::new(),
        }
    }
}

/// A `[privacy.identifier_scan]` entry the policy cannot be built from.
#[derive(Debug, thiserror::Error)]
pub enum PolicyError {
    /// A configured pattern that is not a valid regular expression.
    #[error("privacy.identifier_scan.patterns: `{pattern}` is not a valid regular expression")]
    Pattern {
        /// The pattern as the operator wrote it.
        pattern: String,
        /// The compile failure.
        #[source]
        source: regex::Error,
    },
    /// A rule key this build ships no rule for.
    #[error(
        "privacy.identifier_scan.rules: `{key}` is not an identifier rule this build ships; \
         the rules are {available}"
    )]
    UnknownRule {
        /// The key as the operator wrote it.
        key: String,
        /// Every key this build does ship, comma-separated.
        available: String,
    },
}

impl PrivacyPolicy {
    /// Compile the configured policy.
    ///
    /// # Errors
    /// [`PolicyError`] naming the first unknown rule key or uncompilable
    /// pattern.
    pub fn compile(config: &PrivacyConfig) -> Result<Self, PolicyError> {
        let mut rules = Vec::with_capacity(config.identifier_scan.rules.len());
        for key in &config.identifier_scan.rules {
            let resolved = detect::rule(key).ok_or_else(|| PolicyError::UnknownRule {
                key: key.clone(),
                available: detect::built_in_rules()
                    .iter()
                    .map(|rule| rule.key)
                    .collect::<Vec<_>>()
                    .join(", "),
            })?;
            rules.push(resolved);
        }
        let mut patterns = Vec::with_capacity(config.identifier_scan.patterns.len());
        for pattern in &config.identifier_scan.patterns {
            let compiled = CustomPattern::new(pattern).map_err(|source| PolicyError::Pattern {
                pattern: pattern.clone(),
                source,
            })?;
            patterns.push(compiled);
        }
        Ok(Self {
            subject_namespaces: config.subject_namespaces.clone(),
            allow_identified_parties: config.allow_identified_parties_in_ehr,
            scan_mode: Some(config.identifier_scan.mode),
            rules,
            patterns,
        })
    }

    /// The active identifier rules, in registry order — what the boot log
    /// states so an operator can see which jurisdictions are covered.
    #[must_use]
    pub fn active_rules(&self) -> &[&'static IdentifierRule] {
        &self.rules
    }

    /// The pseudonym namespaces this deployment declared, in configuration
    /// order; the first is the one the server mints into (#3232).
    #[must_use]
    pub fn subject_namespaces(&self) -> &[String] {
        &self.subject_namespaces
    }

    /// Whether the subject-reference rule is in force (the deployment declared
    /// at least one pseudonym namespace).
    #[must_use]
    pub fn subject_rule_in_force(&self) -> bool {
        !self.subject_namespaces.is_empty()
    }

    /// Whether clinical content may carry a named or identified party proxy.
    #[must_use]
    pub fn identified_parties_allowed(&self) -> bool {
        self.allow_identified_parties
    }

    /// The scanner's mode, or `None` when no scanner is installed (the
    /// unenforced [`Default`]).
    #[must_use]
    pub fn scan_mode(&self) -> Option<ScanMode> {
        self.scan_mode
    }

    /// Every finding in a clinical commit body, in document order.
    ///
    /// `root` names the RM type the body is (`COMPOSITION`, `EHR_STATUS`, …)
    /// and heads every reported path. The subject rule applies to an
    /// `EHR_STATUS` body only; the other two apply to every clinical kind.
    #[must_use]
    pub fn findings(&self, root: &str, body: &Value) -> Vec<Finding> {
        let mut findings = Vec::new();
        if root == "EHR_STATUS" && self.subject_rule_in_force() {
            self.check_subject(root, body, &mut findings);
        }
        self.walk(root, body, &mut findings);
        findings
    }

    /// The subject-reference rule over an `EHR_STATUS` body.
    ///
    /// A `PARTY_SELF` with no `external_ref` is untouched: the RM types the
    /// attribute `0..1` and the ITS-REST default `EHR_STATUS` carries a bare
    /// `PARTY_SELF` (`specifications/operations/ehr_create.yaml`), so an EHR
    /// with no subject reference is the spec's own baseline and carries nothing
    /// to minimise.
    fn check_subject(&self, root: &str, body: &Value, findings: &mut Vec<Finding>) {
        let Some(external_ref) = body.pointer("/subject/external_ref") else {
            return;
        };
        let at = format!("{root}/subject/external_ref");
        match external_ref.pointer("/namespace").and_then(Value::as_str) {
            Some(namespace) if self.subject_namespaces.iter().any(|n| n == namespace) => {}
            Some(_) => findings.push(Finding {
                path: format!("{at}/namespace"),
                message: format!(
                    "is not one of the configured pseudonym namespaces ({}); the clinical \
                     side identifies its subject only through a pseudonymisation domain \
                     this deployment declared (privacy.subject_namespaces)",
                    self.subject_namespaces.join(", ")
                ),
                class: FindingClass::Refusal,
            }),
            None => findings.push(Finding {
                path: format!("{at}/namespace"),
                message: "is mandatory once privacy.subject_namespaces is configured".to_owned(),
                class: FindingClass::Refusal,
            }),
        }
        let id_value = external_ref.pointer("/id/value").and_then(Value::as_str);
        if !id_value.is_some_and(is_uuid) {
            findings.push(Finding {
                path: format!("{at}/id/value"),
                message: "must be a UUID: the clinical side holds an opaque subject \
                          pseudonym, never a national identifier, a medical-record number \
                          or a name (privacy.subject_namespaces is configured)"
                    .to_owned(),
                class: FindingClass::Refusal,
            });
        }
    }

    /// The identified-party rule and the identifier scanner, over every node of
    /// the body.
    fn walk(&self, at: &str, node: &Value, findings: &mut Vec<Finding>) {
        match node {
            Value::Object(map) => {
                if !self.allow_identified_parties {
                    check_party(at, map, findings);
                }
                for (key, child) in map {
                    if key == TERMINOLOGY_CODE_KEY {
                        continue;
                    }
                    self.walk(&format!("{at}/{key}"), child, findings);
                }
            }
            Value::Array(items) => {
                for (index, child) in items.iter().enumerate() {
                    self.walk(&format!("{at}[{index}]"), child, findings);
                }
            }
            Value::String(text) => self.scan_text(at, text, findings),
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }

    /// Run the active identifier rules over one string leaf.
    fn scan_text(&self, at: &str, text: &str, findings: &mut Vec<Finding>) {
        if self.scan_mode.is_none() {
            return;
        }
        let mut report = |claim: String| {
            findings.push(Finding {
                path: at.to_owned(),
                message: format!(
                    "carries a value {claim}; the clinical side holds no identifying data \
                     (privacy.identifier_scan)"
                ),
                class: FindingClass::IdentifierShape,
            });
        };
        for rule in &self.rules {
            if rule.matches(text) {
                report(format!(
                    "the `{}` rule claims — a {} {} ({})",
                    rule.key, rule.jurisdiction, rule.label, rule.source
                ));
            }
        }
        for pattern in &self.patterns {
            if pattern.is_match(text) {
                report(format!(
                    "matching the configured pattern `{}` \
                     (privacy.identifier_scan.patterns)",
                    pattern.source()
                ));
            }
        }
    }
}

/// Refuse the party-proxy attributes that identify the record's subject.
///
/// `PARTY_IDENTIFIED.name` is permitted: the class is "Proxy data for an
/// identified party **other than the subject of the record**", "Typically for
/// health care providers, e.g. name and provider number of an institution"
/// (RM common `UML/classes/org.openehr.rm.common.party_identified.adoc`
/// §Description), so a name there is clinician or institution identity — the
/// class's own paradigm case, not patient identity.
///
/// `PARTY_IDENTIFIED.identifiers` is permitted for the same reason: the class
/// is "Used to describe parties where only identifiers may be known … e.g.
/// name and provider number of an institution" (same file, §Description), so
/// a clinician's registration number is its paradigm case, and the Simplified
/// Formats build exactly that from `ctx/participation_identifiers`
/// (ITS-REST `simplified_formats` master06 §Participation). The SUBJECT's
/// identifier is what the boundary keeps off the clinical side, and a national
/// identifier VALUE is the scanner's job (rule 3) wherever it sits (#3254).
///
/// Both `name` and `identifiers` are refused only where the party IS the subject. The
/// class is "Proxy type for identifying a party **and its relationship to the
/// subject** of the record", and its `relationship` is "coded as self" when
/// "it is the patient" (RM common
/// `UML/classes/org.openehr.rm.common.party_related.adoc` §Attributes; `self`
/// is code `0` of the openEHR `subject relationship` group, TERM
/// `SupportTerminology/codesets/openehr_terminology-vocabularies.adoc`). A
/// named mother, guardian or donor is a third party the RM models on purpose,
/// not the subject's identity, and the released spec obliges a server to
/// accept it (#3252). A relationship that cannot be read is treated as `self`:
/// the rule refuses what it cannot prove harmless, and the RM validator names
/// the missing 1..1 attribute on its own. `Basic_validity` is satisfied by
/// `external_ref` alone, so every refused proxy stays expressible.
///
/// A free function rather than a method: the caller has already decided the
/// rule is in force, so this reads no policy state.
fn check_party(at: &str, map: &serde_json::Map<String, Value>, findings: &mut Vec<Finding>) {
    let related = match map.get("_type").and_then(Value::as_str) {
        Some("PARTY_RELATED") => true,
        Some("PARTY_IDENTIFIED") => false,
        _ => return,
    };
    let present = |attribute: &str| map.get(attribute).is_some_and(|v| !v.is_null());
    if related && present("name") && relationship_is_self(map.get("relationship")) {
        findings.push(Finding {
            path: format!("{at}/name"),
            message: "names the record's subject: a PARTY_RELATED whose relationship is \
                      `self` IS the patient (PARTY_RELATED §Attributes: \"If it is the \
                      patient, coded as self\"), so a name here identifies or re-identifies \
                      the subject. external_ref alone satisfies Basic_validity. Set \
                      privacy.allow_identified_parties_in_ehr to accept it."
                .to_owned(),
            class: FindingClass::Refusal,
        });
    }
    if related && present("identifiers") && relationship_is_self(map.get("relationship")) {
        findings.push(Finding {
            path: format!("{at}/identifiers"),
            message: "carries the record's subject's formal identifiers: a PARTY_RELATED \
                      whose relationship is `self` IS the patient (PARTY_RELATED \
                      §Attributes), and identifiers is \"One or more formal identifiers \
                      (possibly computable)\" (PARTY_IDENTIFIED §Attributes) — the \
                      national-identifier slot the clinical side does not hold for its \
                      subject. external_ref alone satisfies Basic_validity. Set \
                      privacy.allow_identified_parties_in_ehr to accept it."
                .to_owned(),
            class: FindingClass::Refusal,
        });
    }
}

/// Whether a `PARTY_RELATED.relationship` says the party IS the subject.
///
/// `self` is code `0` of the openEHR `subject relationship` group, so a
/// `DV_CODED_TEXT` coded `openehr::0` is `self`, and so is one whose text
/// reads `self` when it carries no `defining_code` a terminology could
/// settle. An absent or unreadable relationship counts as `self`: the rule
/// refuses what it cannot prove harmless, and the RM validator reports the
/// missing 1..1 attribute on its own.
fn relationship_is_self(relationship: Option<&Value>) -> bool {
    let Some(relationship) = relationship else {
        return true;
    };
    let code = relationship
        .pointer("/defining_code/code_string")
        .and_then(Value::as_str);
    let terminology = relationship
        .pointer("/defining_code/terminology_id/value")
        .and_then(Value::as_str);
    match (code, terminology) {
        (Some(code), Some(terminology)) => terminology == "openehr" && code == "0",
        (Some(code), None) => code == "0",
        (None, _) => relationship
            .get("value")
            .and_then(Value::as_str)
            .is_none_or(|text| text.trim().eq_ignore_ascii_case("self")),
    }
}

/// Whether `value` is a UUID in the hyphenated form, in either case.
///
/// Parsed by [`uuid::Uuid`] and then required to render back to the same
/// characters, so the braced, URN and simple forms the parser also accepts are
/// refused. The promoted `ehr.subject_id` column is compared as text by the
/// one-EHR-per-subject index, and four spellings of one pseudonym would be four
/// subjects there.
fn is_uuid(value: &str) -> bool {
    uuid::Uuid::try_parse(value)
        .is_ok_and(|parsed| parsed.hyphenated().to_string().eq_ignore_ascii_case(value))
}
