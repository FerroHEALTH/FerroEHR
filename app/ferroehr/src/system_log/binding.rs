// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outcome of the party binding an access record carries, and the fields
//! each outcome needs.
//!
//! EHDS (Regulation (EU) 2025/327) Annex II 3.2 asks the log to identify
//! "(a) ... the healthcare provider or other individuals having accessed" and
//! "(b) ... the specific natural person or persons having accessed"
//! (`docs/law/eu/ehds/text.html`). [`BindingCode`] names how the caller was
//! bound or why it was refused, and [`BindingRecord`] carries the party, the
//! groups, the provider and what a verified external accessor asserts beside
//! it. Which fields carry the binding is our own design, as no openEHR spec
//! governs access logging.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the binding columns are jsonb, whose \
              values are written as JSON (stored-content class)"
)]

use std::collections::BTreeMap;

use serde::Serialize;
use serde_json::Value;

/// How the caller of a record was bound, or the refusal it was answered with.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum BindingCode {
    /// A person bound to a `complete` `PERSON`.
    Bound,
    /// A person bound to an `incomplete` `PERSON`.
    Provisional,
    /// A verified external person of a `federation` issuer.
    External,
    /// An organisation under an agreement with `accessor = "organisation"`.
    Organisation,
    /// A client acting for nobody.
    System,
    /// A patient token of a `patients` issuer.
    SelfPatient,
    /// Refused: no party carries the login.
    RefusedUnbound,
    /// Refused: two or more parties carry it.
    RefusedAmbiguous,
    /// Refused: the `PERSON` is `800|inactive|`.
    RefusedInactive,
    /// Refused: the `PERSON` is `801|abandoned|`.
    RefusedAbandoned,
    /// Refused: the login's key belongs to a deleted `PERSON`.
    RefusedDeprovisioned,
    /// Refused: the party's binding marker does not fit the caller's kind.
    RefusedWrongMarker,
    /// Refused: no live representation relationship to the requested subject.
    RefusedRepresentationUnrecorded,
    /// Refused: the token lacks the key claim.
    RefusedKeyClaimMissing,
    /// Refused: the group claim is unreadable or overage.
    RefusedGroupsUnavailable,
    /// Refused: a guest of the issuer.
    RefusedGuest,
    /// Refused: a `federation` token lacks or garbles a required field.
    RefusedExternalIncomplete,
    /// Refused: the request's patient is not the TRC patient.
    RefusedTrcPatientMismatch,
    /// Refused: a contact-point patient-data request carries no TRC.
    RefusedTrcMissing,
    /// Refused: a system caller holds a `user/` or `patient/` scope.
    RefusedSystemPersonalScope,
    /// Refused: no federation agreement of the issuer admits the accessor.
    RefusedNoAgreement,
    /// Refused: an external caller holds a scope its agreement does not read.
    RefusedScopeBeyondAgreement,
}

impl BindingCode {
    /// Every code, in the order the `ck_audit_event_binding` constraint lists
    /// them.
    pub const ALL: [Self; 22] = [
        Self::Bound,
        Self::Provisional,
        Self::External,
        Self::Organisation,
        Self::System,
        Self::SelfPatient,
        Self::RefusedUnbound,
        Self::RefusedAmbiguous,
        Self::RefusedInactive,
        Self::RefusedAbandoned,
        Self::RefusedDeprovisioned,
        Self::RefusedWrongMarker,
        Self::RefusedRepresentationUnrecorded,
        Self::RefusedKeyClaimMissing,
        Self::RefusedGroupsUnavailable,
        Self::RefusedGuest,
        Self::RefusedExternalIncomplete,
        Self::RefusedTrcPatientMismatch,
        Self::RefusedTrcMissing,
        Self::RefusedSystemPersonalScope,
        Self::RefusedNoAgreement,
        Self::RefusedScopeBeyondAgreement,
    ];

    /// Whether a refusal under this code is a security event, recorded once
    /// per answered request and never summarised: XCA 8.0.1 §4.2.2 asks for
    /// an audit entry each time the TRC patient does not match.
    #[must_use]
    pub const fn is_security_event(self) -> bool {
        matches!(
            self,
            Self::RefusedTrcPatientMismatch | Self::RefusedTrcMissing
        )
    }

    /// The stored spelling, matching the `ck_audit_event_binding` constraint.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bound => "bound",
            Self::Provisional => "provisional",
            Self::External => "external",
            Self::Organisation => "organisation",
            Self::System => "system",
            Self::SelfPatient => "self",
            Self::RefusedUnbound => "refused-unbound",
            Self::RefusedAmbiguous => "refused-ambiguous",
            Self::RefusedInactive => "refused-inactive",
            Self::RefusedAbandoned => "refused-abandoned",
            Self::RefusedDeprovisioned => "refused-deprovisioned",
            Self::RefusedWrongMarker => "refused-wrong-marker",
            Self::RefusedRepresentationUnrecorded => "refused-representation-unrecorded",
            Self::RefusedKeyClaimMissing => "refused-key-claim-missing",
            Self::RefusedGroupsUnavailable => "refused-groups-unavailable",
            Self::RefusedGuest => "refused-guest",
            Self::RefusedExternalIncomplete => "refused-external-incomplete",
            Self::RefusedTrcPatientMismatch => "refused-trc-patient-mismatch",
            Self::RefusedTrcMissing => "refused-trc-missing",
            Self::RefusedSystemPersonalScope => "refused-system-personal-scope",
            Self::RefusedNoAgreement => "refused-no-agreement",
            Self::RefusedScopeBeyondAgreement => "refused-scope-beyond-agreement",
        }
    }

    /// The code spelled `text`, or `None` for a spelling no code has.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|code| code.as_str() == text)
    }
}

/// A demographic party as a `PARTY_REF` triple.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PartyTriple {
    /// The `PARTY_REF.namespace`.
    pub namespace: String,
    /// The `PARTY_REF.type`.
    #[serde(rename = "type")]
    pub class: String,
    /// The id of the party's versioned object.
    pub id: String,
}

/// An identifier under its issuer: a healthcare provider or a facility.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Identified {
    /// The identifier's issuer.
    pub issuer: String,
    /// The identifier.
    pub id: String,
}

/// Whom a representative acts for, and on which ground.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Representation {
    /// The represented subject's identifier, as asserted.
    pub represented: String,
    /// The ground of the representation, as asserted.
    pub ground: Option<String>,
    /// The `subject_relationship` code the representative's committer carries.
    pub relationship: Option<String>,
}

/// The federation agreement an external accessor was admitted under.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AgreementRecord {
    /// The agreement's name.
    pub name: String,
    /// The agreement's version.
    pub version: String,
    /// The coded legal ground, as the agreement states it.
    pub legal_ground: Option<String>,
    /// `person` or `organisation`.
    pub accessor: String,
    /// `automated` or `chained`, for an organisation.
    pub access_class: Option<String>,
}

/// A summary record's count, and the EHR ids the refusals named.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Summary {
    /// How many refusals the record stands for.
    pub count: u64,
    /// The EHR ids the refused requests named.
    pub ehr_ids: Vec<String>,
}

/// The binding outcome of one access record, with the fields it needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindingRecord {
    /// How the caller was bound, or the refusal.
    pub code: BindingCode,
    /// The bound party.
    pub party: Option<PartyTriple>,
    /// The ids of the groups the bound party belongs to.
    pub groups: Vec<String>,
    /// The healthcare provider the access is attributed to.
    pub provider: Option<Identified>,
    /// Whether the caller is a guest its issuer admits.
    pub guest: bool,
    /// For a representative, whom they act for.
    pub representation: Option<Representation>,
    /// The fields a federation token asserts, by field name.
    pub external: BTreeMap<String, Vec<String>>,
    /// The agreement an external accessor was admitted under.
    pub agreement: Option<AgreementRecord>,
    /// The RFC 8693 actor chain, outermost first, as `iss sub` pairs.
    pub actor_chain: Vec<String>,
    /// The facilities the compositions a system caller committed name.
    pub facilities: Vec<Identified>,
    /// What a refusal names beside its code, by name.
    pub detail: BTreeMap<String, Vec<String>>,
}

impl BindingRecord {
    /// The record of `code` with no field beside it.
    #[must_use]
    pub const fn of(code: BindingCode) -> Self {
        Self {
            code,
            party: None,
            groups: Vec::new(),
            provider: None,
            guest: false,
            representation: None,
            external: BTreeMap::new(),
            agreement: None,
            actor_chain: Vec::new(),
            facilities: Vec::new(),
            detail: BTreeMap::new(),
        }
    }

    /// A person bound to `party`, provisional when the `PERSON` is
    /// `incomplete`, with its groups, its provider and its guest mark.
    #[must_use]
    pub fn bound(
        party: PartyTriple,
        provisional: bool,
        groups: Vec<String>,
        provider: Option<Identified>,
        guest: bool,
    ) -> Self {
        Self {
            party: Some(party),
            groups,
            provider,
            guest,
            ..Self::of(if provisional {
                BindingCode::Provisional
            } else {
                BindingCode::Bound
            })
        }
    }

    /// A system caller acting for `provider`.
    #[must_use]
    pub fn system(provider: Option<Identified>) -> Self {
        Self {
            provider,
            ..Self::of(BindingCode::System)
        }
    }

    /// A verified external person or organisation admitted under `agreement`,
    /// asserting `external`, attributed to `provider`.
    #[must_use]
    pub fn external(
        organisation: bool,
        external: BTreeMap<String, Vec<String>>,
        agreement: AgreementRecord,
        provider: Option<Identified>,
    ) -> Self {
        Self {
            external,
            agreement: Some(agreement),
            provider,
            ..Self::of(if organisation {
                BindingCode::Organisation
            } else {
                BindingCode::External
            })
        }
    }

    /// The refusal `code`, naming `detail`.
    #[must_use]
    pub fn refused(code: BindingCode, detail: BTreeMap<String, Vec<String>>) -> Self {
        Self {
            detail,
            ..Self::of(code)
        }
    }

    /// The JSON value of each jsonb binding column, in the order the inserts
    /// list them after `binding` and `guest`: party, groups, provider,
    /// representation, external fields, agreement, actor chain, facilities
    /// and detail; `NULL` for an empty field.
    #[must_use]
    pub fn json_columns(&self) -> [Option<Value>; 9] {
        fn json<T: Serialize>(value: &T) -> Option<Value> {
            serde_json::to_value(value).ok()
        }
        fn listed<T: Serialize>(values: &[T]) -> Option<Value> {
            (!values.is_empty()).then(|| json(&values)).flatten()
        }
        fn mapped(map: &BTreeMap<String, Vec<String>>) -> Option<Value> {
            (!map.is_empty()).then(|| json(map)).flatten()
        }
        [
            self.party.as_ref().and_then(json),
            listed(&self.groups),
            self.provider.as_ref().and_then(json),
            self.representation.as_ref().and_then(json),
            mapped(&self.external),
            self.agreement.as_ref().and_then(json),
            listed(&self.actor_chain),
            listed(&self.facilities),
            mapped(&self.detail),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn party() -> PartyTriple {
        PartyTriple {
            namespace: "demographic".to_owned(),
            class: "PERSON".to_owned(),
            id: "0b9e6c2a-5d4f-4e8a-9b1c-2d3e4f5a6b7c".to_owned(),
        }
    }

    fn provider() -> Identified {
        Identified {
            issuer: "urn:oid:2.999.7.9".to_owned(),
            id: "dev".to_owned(),
        }
    }

    #[test]
    fn every_code_has_one_spelling_that_parses_back() {
        let mut spellings: Vec<&str> = BindingCode::ALL.iter().map(|c| c.as_str()).collect();
        spellings.sort_unstable();
        spellings.dedup();
        assert_eq!(spellings.len(), BindingCode::ALL.len());
        for code in BindingCode::ALL {
            assert_eq!(BindingCode::parse(code.as_str()), Some(code));
        }
        assert_eq!(BindingCode::parse("bound-ish"), None);
    }

    #[test]
    fn a_bound_and_a_provisional_person_carry_party_groups_provider_and_guest() {
        let bound = BindingRecord::bound(
            party(),
            false,
            vec!["g1".to_owned()],
            Some(provider()),
            true,
        );
        assert_eq!(bound.code, BindingCode::Bound);
        let [party_json, groups, provider_json, ..] = bound.json_columns();
        assert_eq!(
            party_json,
            Some(
                serde_json::json!({ "namespace": "demographic", "type": "PERSON",
                                     "id": "0b9e6c2a-5d4f-4e8a-9b1c-2d3e4f5a6b7c" })
            )
        );
        assert_eq!(groups, Some(serde_json::json!(["g1"])));
        assert_eq!(
            provider_json,
            Some(serde_json::json!({ "issuer": "urn:oid:2.999.7.9", "id": "dev" }))
        );
        assert!(bound.guest);
        let provisional = BindingRecord::bound(party(), true, Vec::new(), None, false);
        assert_eq!(provisional.code, BindingCode::Provisional);
        assert_eq!(provisional.json_columns()[1], None, "no group is NULL");
    }

    #[test]
    fn a_system_caller_carries_its_provider_and_its_facilities() {
        let mut system = BindingRecord::system(Some(provider()));
        system.facilities.push(Identified {
            issuer: "urn:oid:2.999.1".to_owned(),
            id: "ward-7".to_owned(),
        });
        assert_eq!(system.code, BindingCode::System);
        assert_eq!(
            system.json_columns()[7],
            Some(serde_json::json!([{ "issuer": "urn:oid:2.999.1", "id": "ward-7" }]))
        );
    }

    #[test]
    fn an_external_accessor_carries_its_fields_and_agreement() {
        let agreement = AgreementRecord {
            name: "Regional exchange".to_owned(),
            version: "1".to_owned(),
            legal_ground: Some("GDPR Art. 6(1)(e), Art. 9(2)(h)".to_owned()),
            accessor: "person".to_owned(),
            access_class: None,
        };
        let fields = BTreeMap::from([(
            "professional.hp_identifier".to_owned(),
            vec!["hp-1".to_owned()],
        )]);
        let person = BindingRecord::external(false, fields.clone(), agreement.clone(), None);
        assert_eq!(person.code, BindingCode::External);
        assert_eq!(
            person.json_columns()[4],
            Some(serde_json::json!({ "professional.hp_identifier": ["hp-1"] }))
        );
        let organisation = BindingRecord::external(true, fields, agreement, Some(provider()));
        assert_eq!(organisation.code, BindingCode::Organisation);
        assert!(organisation.json_columns()[5].is_some());
    }

    #[test]
    fn self_and_every_refusal_carry_their_code() {
        assert_eq!(
            BindingRecord::of(BindingCode::SelfPatient).code.as_str(),
            "self"
        );
        for code in BindingCode::ALL
            .into_iter()
            .filter(|code| code.as_str().starts_with("refused-"))
        {
            let detail = BTreeMap::from([("claim".to_owned(), vec!["oid".to_owned()])]);
            let refused = BindingRecord::refused(code, detail);
            assert_eq!(refused.code, code);
            assert_eq!(
                refused.json_columns()[8],
                Some(serde_json::json!({ "claim": ["oid"] }))
            );
        }
    }

    #[test]
    fn a_summary_record_carries_its_count_and_ehr_ids() {
        let summary = Summary {
            count: 12,
            ehr_ids: vec!["e1".to_owned()],
        };
        assert_eq!(
            serde_json::to_value(&summary).ok(),
            Some(serde_json::json!({ "count": 12, "ehr_ids": ["e1"] }))
        );
    }

    #[test]
    fn only_the_trc_refusals_are_security_events() {
        let security: Vec<BindingCode> = BindingCode::ALL
            .into_iter()
            .filter(|code| code.is_security_event())
            .collect();
        assert_eq!(
            security,
            [
                BindingCode::RefusedTrcPatientMismatch,
                BindingCode::RefusedTrcMissing
            ]
        );
    }
}
