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
//! Two rules, both evaluated over a clinical commit body before it is stored:
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
//!    the refused proxy stays expressible.
//!
//! Both rules read the RM structure and never guess from a string's shape
//! whether it identifies a person: a clinical value such as a laboratory
//! order number is stored as written (#6081).
//!
//! A finding never carries the offending VALUE, only its RM path and the rule
//! that matched: a refusal travels into an error body, an access log and a
//! trace, and echoing an identifier into all three to complain about it would
//! be the leak the rule exists to prevent.

pub mod config;

use std::fmt::Write as _;

use openehr_base::validate::InvariantViolation;

use crate::error::ServiceError;
use crate::privacy::config::PrivacyConfig;
use crate::versioning::tree::{Shape, Tree, Typed};
use openehr_rm::v1_2::validate::tree::Node;

/// What a clinical commit body carried that the policy refuses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// The RM instance path of the offending node.
    pub path: String,
    /// What is wrong, naming the shape and never the value.
    pub message: String,
}

/// The runtime form of [`PrivacyConfig`], built once at boot and held by the
/// service.
///
/// The [`Default`] is the refusing posture [`PrivacyConfig::default`] builds:
/// identified `self` parties refused and no namespace declared (#3243). A
/// `FerroEhrService` built without
/// [`crate::service::FerroEhrService::with_privacy`] therefore runs the
/// posture the shipped binary runs, never a more permissive one.
#[derive(Debug, Clone, Default)]
pub struct PrivacyPolicy {
    subject_namespaces: Vec<String>,
    allow_identified_parties: bool,
}

impl From<&PrivacyConfig> for PrivacyPolicy {
    fn from(config: &PrivacyConfig) -> Self {
        Self {
            subject_namespaces: config.subject_namespaces.clone(),
            allow_identified_parties: config.allow_identified_parties_in_ehr,
        }
    }
}

impl PrivacyPolicy {
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

    /// Whether `id` under `namespace` is a subject reference the subject rule
    /// admits on `EHR_STATUS`: a declared pseudonym namespace and a UUID.
    ///
    /// Strict where [`Self::findings`] is lenient: with no namespace declared,
    /// nothing is admitted, because a caller asking holds an identifier whose
    /// pseudonymity it has to establish.
    #[must_use]
    pub fn admits_subject(&self, namespace: &str, id: &str) -> bool {
        self.subject_namespaces.iter().any(|n| n == namespace) && is_uuid(id)
    }

    /// Whether the pass has a rule in force for a body of RM type `rm_type`:
    /// the identified-party rule for every clinical kind, or the subject rule
    /// for an `EHR_STATUS`. A caller skips the pass, and the reading of the
    /// body it needs, when neither is.
    #[must_use]
    pub fn applies_to(&self, rm_type: &str) -> bool {
        self.party_rule_in_force() || (rm_type == "EHR_STATUS" && self.subject_rule_in_force())
    }

    /// Whether the identified-party rule is in force (the deployment did not
    /// opt in to identified parties).
    fn party_rule_in_force(&self) -> bool {
        !self.allow_identified_parties
    }

    /// Every finding in a clinical commit body, in document order.
    ///
    /// `root` names the RM type the body is (`COMPOSITION`, `EHR_STATUS`, …)
    /// and heads every reported path. The subject rule applies to an
    /// `EHR_STATUS` body only; the identified-party rule to every clinical
    /// kind.
    /// The body is read through either shape ([`crate::versioning::tree`]).
    #[must_use]
    pub fn findings<'a, T: Tree<'a>>(&self, root: &str, body: T) -> Vec<Finding> {
        let mut findings = Vec::new();
        if root == "EHR_STATUS" && self.subject_rule_in_force() {
            self.check_subject(root, body, &mut findings);
        }
        if self.party_rule_in_force() {
            walk(&mut root.to_owned(), body, &mut findings);
        }
        findings
    }

    /// The clinical-side data-minimisation pass over one decoded body of RM
    /// type `rm_type`, read through either shape ([`crate::versioning::tree`]).
    ///
    /// Its refusals hold whenever their rule is in force, on the commit path and
    /// on the verbatim-replay paths (EHR-Extract import, admin load), which can
    /// only refuse (#3237). A finding never carries the offending value. The
    /// status is `422` (ITS-REST `overview/Requests_and_responses.md` §HTTP status
    /// codes). No openEHR spec governs this: our own design.
    ///
    /// # Errors
    /// [`ServiceError::ValidationFailed`] carrying one entry per refused finding,
    /// each keyed by its own RM path (→ 422).
    pub(crate) fn enforce<'a, T: Tree<'a>>(
        &self,
        rm_type: &str,
        data: T,
    ) -> Result<(), ServiceError> {
        Self::refuse(self.findings(rm_type, data))
    }

    /// The identified-party rule on one node of a typed body's walk, at RM
    /// instance path `path` (empty for the root) below a body of RM type
    /// `root`: what [`Self::findings`] finds at that node while the rule is in
    /// force.
    ///
    /// The commit validation calls it from the RM pass's walk
    /// ([`openehr_sdt::rm_instance::validate_rm_and_terminology_typed_visiting`]),
    /// so the pass needs no walk of its own.
    pub(crate) fn check_typed_node(
        root: &str,
        path: &str,
        node: &Node<'_>,
        findings: &mut Vec<Finding>,
    ) {
        use openehr_rm::v1_2::access::AsRm;
        use openehr_rm::v1_2::common::generic::party_identified::PartyIdentified;
        use openehr_rm::v1_2::common::generic::party_related::PartyRelated;
        let object = match node.rm_type {
            "PARTY_RELATED" => node.value.downcast_ref::<PartyRelated>().map(AsRm::as_rm),
            "PARTY_IDENTIFIED" => node
                .value
                .downcast_ref::<PartyIdentified>()
                .map(AsRm::as_rm),
            _ => None,
        };
        if let Some(object) = object {
            check_party(&format!("{root}{path}"), Typed::Object(object), findings);
        }
    }

    /// `Ok` for no finding, else the `422` refusal carrying one entry per
    /// finding, each keyed by its own RM path.
    ///
    /// # Errors
    /// [`ServiceError::ValidationFailed`] when `refused` holds a finding.
    pub(crate) fn refuse(refused: Vec<Finding>) -> Result<(), ServiceError> {
        if refused.is_empty() {
            return Ok(());
        }
        crate::telemetry::metrics::metrics()
            .validation_failures
            .add(
                u64::try_from(refused.len()).unwrap_or(u64::MAX),
                &[opentelemetry::KeyValue::new("pass", "privacy")],
            );
        Err(ServiceError::ValidationFailed(
            refused
                .into_iter()
                .map(|finding| InvariantViolation::at(finding.path, finding.message))
                .collect(),
        ))
    }

    /// The subject-reference rule over an `EHR_STATUS` body.
    ///
    /// A `PARTY_SELF` with no `external_ref` is untouched: the RM types the
    /// attribute `0..1` and the ITS-REST default `EHR_STATUS` carries a bare
    /// `PARTY_SELF` (`specifications/operations/ehr_create.yaml`), so an EHR
    /// with no subject reference is the spec's own baseline and carries nothing
    /// to minimise.
    fn check_subject<'a, T: Tree<'a>>(&self, root: &str, body: T, findings: &mut Vec<Finding>) {
        let Some(external_ref) = body.get("subject").and_then(|s| s.get("external_ref")) else {
            return;
        };
        let at = format!("{root}/subject/external_ref");
        match external_ref.get_text("namespace").as_deref() {
            Some(namespace) if self.subject_namespaces.iter().any(|n| n == namespace) => {}
            Some(_) => findings.push(Finding {
                path: format!("{at}/namespace"),
                message: format!(
                    "is not one of the configured pseudonym namespaces ({}); the clinical \
                     side identifies its subject only through a pseudonymisation domain \
                     this deployment declared (privacy.subject_namespaces)",
                    self.subject_namespaces.join(", ")
                ),
            }),
            None => findings.push(Finding {
                path: format!("{at}/namespace"),
                message: "is mandatory once privacy.subject_namespaces is configured".to_owned(),
            }),
        }
        let id_value = external_ref.get("id").and_then(|id| id.get_text("value"));
        if !id_value.as_deref().is_some_and(is_uuid) {
            findings.push(Finding {
                path: format!("{at}/id/value"),
                message: "must be a UUID: the clinical side holds an opaque subject \
                          pseudonym, never a national identifier, a medical-record number \
                          or a name (privacy.subject_namespaces is configured)"
                    .to_owned(),
            });
        }
    }
}

/// The identified-party rule over every object of the body. `at` is the path
/// of `node`, one buffer the walk extends and restores around each child, so a
/// path is copied only into a finding. String leaves are never read.
fn walk<'a, T: Tree<'a>>(at: &mut String, node: T, findings: &mut Vec<Finding>) {
    match node.shape() {
        Shape::Object => {
            check_party(at, node, findings);
            node.each_member(&mut |key, child| {
                if !matches!(child.shape(), Shape::Object | Shape::Array) {
                    return;
                }
                let len = at.len();
                at.push('/');
                at.push_str(key);
                walk(at, child, findings);
                at.truncate(len);
            });
        }
        Shape::Array => {
            let mut index = 0_usize;
            node.each_item(&mut |child| {
                if matches!(child.shape(), Shape::Object | Shape::Array) {
                    let len = at.len();
                    let _ = write!(at, "[{index}]");
                    walk(at, child, findings);
                    at.truncate(len);
                }
                index = index.saturating_add(1);
            });
        }
        Shape::String | Shape::Other => {}
    }
}

/// Refuses the party-proxy attributes that identify the record's subject.
///
/// `PARTY_IDENTIFIED.name` and `identifiers` are permitted: the class is
/// "Proxy data for an identified party other than the subject of the record",
/// "Typically for health care providers" (RM common
/// `UML/classes/org.openehr.rm.common.party_identified.adoc` §Description).
/// On `PARTY_RELATED` both are refused only where its `relationship` is
/// `self`, code `0` of the openEHR `subject relationship` group (RM common
/// `UML/classes/org.openehr.rm.common.party_related.adoc` §Attributes); a
/// relationship that cannot be read counts as `self`. `Basic_validity` is
/// satisfied by `external_ref` alone, so every refused proxy stays
/// expressible (#3252, #3254).
fn check_party<'a, T: Tree<'a>>(at: &str, map: T, findings: &mut Vec<Finding>) {
    let related = match map.get_text("_type").as_deref() {
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
fn relationship_is_self<'a, T: Tree<'a>>(relationship: Option<T>) -> bool {
    let Some(relationship) = relationship else {
        return true;
    };
    let defining_code = relationship.get("defining_code");
    let code = defining_code.and_then(|c| c.get_text("code_string"));
    let terminology = defining_code
        .and_then(|c| c.get("terminology_id"))
        .and_then(|t| t.get_text("value"));
    match (code.as_deref(), terminology.as_deref()) {
        (Some(code), Some(terminology)) => terminology == "openehr" && code == "0",
        (Some(code), None) => code == "0",
        (None, _) => relationship
            .get_text("value")
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
