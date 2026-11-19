// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller's facts about care relationships, read from the policy in
//! force with each void attribute at its strictest default.
//!
//! The draft package gives each attribute its reading when void (draft
//! access control package, The Controller's Facts about Care
//! Relationships): no self-declared and no attested relationship, no
//! conflict allowed, no level concealed, a query narrowable to the caseload,
//! and each coded attribute on the package's default code system. With no
//! policy committed every attribute is void. Our own design over our own
//! model.

use openehr_rm::draft::access_control::access_control::access_conflict_rule::AccessConflictRule;
use openehr_rm::draft::access_control::access_control::access_policy::AccessPolicy;
use openehr_rm::draft::access_control::terminology::{Confidentiality, Term, ValueSetBinding};
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_duration::DvDuration;
use openehr_rm::v1_2::data_types::uri::dv_uri::DvUri;

/// The care facts of one policy, or of none.
#[derive(Debug, Clone, Copy)]
pub struct CareFacts<'a> {
    policy: Option<&'a AccessPolicy>,
}

impl<'a> CareFacts<'a> {
    /// The facts of `policy`; `None` reads every attribute as void.
    #[must_use]
    pub const fn of(policy: Option<&'a AccessPolicy>) -> Self {
        Self { policy }
    }

    /// Whether a query may be narrowed to the caller's caseload; void means
    /// true.
    #[must_use]
    pub fn caseload(self) -> bool {
        self.policy.and_then(|p| p.caseload).unwrap_or(true)
    }

    /// The longest period of a self-declared care relationship; `None`
    /// refuses every self-declared relationship.
    #[must_use]
    pub fn self_declared_max(self) -> Option<&'a DvDuration> {
        self.policy.and_then(|p| p.self_declared_care.as_ref())
    }

    /// The agreements under which a verified credential may attest a care
    /// relationship; empty admits no attested relationship.
    #[must_use]
    pub fn attested_agreements(self) -> &'a [DvIdentifier] {
        self.policy
            .and_then(|p| p.attested_care.as_deref())
            .unwrap_or_default()
    }

    /// The conflicts the controller allows through the professional path;
    /// empty allows none.
    #[must_use]
    pub fn conflict_allowances(self) -> &'a [AccessConflictRule] {
        self.policy
            .and_then(|p| p.conflict_rules.as_deref())
            .unwrap_or_default()
    }

    /// The confidentiality levels whose refused records answer as absent;
    /// empty answers every refused named record as forbidden. A level that
    /// does not read never reaches a stored policy, which the write check
    /// refuses.
    #[must_use]
    pub fn concealed(self) -> Vec<Confidentiality> {
        let mut levels: Vec<Confidentiality> = self
            .policy
            .and_then(|p| p.concealed_confidentialities.as_deref())
            .unwrap_or_default()
            .iter()
            // NOTE: our own design (#6072), the write check refuses a level that does not read.
            .filter_map(|level| Confidentiality::read(level).ok())
            .collect();
        levels.sort_unstable();
        levels.dedup();
        levels
    }

    /// The canonical URL of the value set the policy binds `binding` to;
    /// `None` leaves the attribute on the package's default code system.
    #[must_use]
    pub fn value_set(self, binding: ValueSetBinding) -> Option<&'a str> {
        self.policy
            .and_then(|p| p.value_sets.as_deref())
            .unwrap_or_default()
            .iter()
            .find(|bound| binding.is(&bound.binding))
            .map(|bound| match &bound.value_set {
                DvUri::DvUri(uri) => uri.value.as_str(),
                DvUri::DvEhrUri(uri) => uri.value.as_str(),
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A policy over the default archetype with `extra` members.
    fn policy(extra: &str) -> AccessPolicy {
        let body = format!(
            r#"{{ "_type": "ACCESS_POLICY",
                "name": {{ "_type": "DV_TEXT", "value": "policy" }},
                "archetype_node_id": "openEHR-EHR-ACCESS_POLICY.controller.v1",
                "controller": {{ "_type": "PARTY_REF", "namespace": "demographic",
                    "type": "ORGANISATION",
                    "id": {{ "_type": "HIER_OBJECT_ID", "value": "2c3d4e5f-6a7b-4c8d-9e0f-a1b2c3d4e5f7" }} }},
                "care_relationship_required": true,
                "purpose_required": false{extra} }}"#
        );
        openehr_its::json::from_canonical_json(&body).expect("the policy should decode")
    }

    /// A coded term as JSON text.
    fn term(terminology: &str, code: &str) -> String {
        format!(
            r#"{{ "_type": "DV_CODED_TEXT", "value": "{code}",
                "defining_code": {{ "terminology_id": {{ "value": "{terminology}" }},
                    "code_string": "{code}" }} }}"#
        )
    }

    #[test]
    fn every_void_attribute_reads_as_its_strictest_default() {
        for facts in [CareFacts::of(None), CareFacts::of(Some(&policy("")))] {
            assert!(facts.caseload());
            assert!(facts.self_declared_max().is_none());
            assert_eq!(
                facts.attested_agreements().to_vec(),
                Vec::<DvIdentifier>::new()
            );
            assert_eq!(
                facts.conflict_allowances().to_vec(),
                Vec::<AccessConflictRule>::new()
            );
            assert_eq!(facts.concealed(), Vec::<Confidentiality>::new());
            for binding in ValueSetBinding::ALL {
                assert_eq!(facts.value_set(*binding), None, "{binding}");
            }
        }
    }

    #[test]
    fn a_set_attribute_reads_as_written() {
        let set = policy(&format!(
            r#", "self_declared_care": {{ "_type": "DV_DURATION", "value": "P1D" }},
                "attested_care": [{{ "_type": "DV_IDENTIFIER", "id": "urn:example:agreement" }}],
                "caseload": false,
                "conflict_rules": [{{ "_type": "ACCESS_CONFLICT_RULE", "conflict": {} }}],
                "concealed_confidentialities": [{}, {}, {}],
                "value_sets": [{{ "_type": "ACCESS_VALUE_SET", "binding": {},
                    "value_set": {{ "_type": "DV_URI", "value": "http://vs.example/kinship" }} }}]"#,
            term("openehr_access", "self"),
            term("HL7_Confidentiality", "V"),
            term("HL7_Confidentiality", "R"),
            term("HL7_Confidentiality", "V"),
            term("openehr_access", "kinship"),
        ));
        let facts = CareFacts::of(Some(&set));
        assert!(!facts.caseload());
        assert_eq!(
            facts.self_declared_max().map(|d| d.value.as_str()),
            Some("P1D")
        );
        assert_eq!(facts.attested_agreements().len(), 1);
        assert_eq!(facts.conflict_allowances().len(), 1);
        assert_eq!(
            facts.concealed(),
            vec![Confidentiality::Restricted, Confidentiality::VeryRestricted]
        );
        assert_eq!(
            facts.value_set(ValueSetBinding::Kinship),
            Some("http://vs.example/kinship")
        );
        assert_eq!(facts.value_set(ValueSetBinding::CareRole), None);
    }
}
