// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The treatment context a caller's verified credential attests, under the
//! agreement it was admitted by: the `attested` care path.
//!
//! The draft access control decision procedure (Holding a Care Relationship)
//! lets a caller hold a care relationship that is never stored: its verified
//! credential, issued under an agreement `ACCESS_POLICY.attested_care` names,
//! attests a treatment context for this subject that is current at the
//! access. The credential's issuer and its claims are the deployment's data
//! (`[[auth.federation]]`); the CDR reads the attested patient, the
//! assertion's id and its window, and resolves the patient to the EHRs whose
//! subject the patient identifier names. An agreement is named by the
//! container of its `ROLE`, so a new version of the agreement keeps the
//! policy's reference, and by the partner's federation issuer when the
//! policy sets one (owner decision on #6078). Our own design.

use openehr_rm::draft::access_control::terminology::{
    ForeignCode, TERMINOLOGY_ID_PARTICIPATION_TYPE,
};
use openehr_rm::v1_2::data_types::basic::dv_identifier::DvIdentifier;
use uuid::Uuid;

use crate::ids::EhrId;

/// The agreement an attestation was admitted under.
#[derive(Clone, PartialEq, Eq)]
pub struct AttestingAgreement {
    /// The container of the agreement's `ROLE`.
    pub role: Uuid,
    /// The federation issuer the partner's client identifier is held under.
    pub issuer: String,
    /// Its name (`agreement_name`).
    pub name: String,
    /// Its version (`agreement_version`).
    pub version: String,
}

impl std::fmt::Debug for AttestingAgreement {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AttestingAgreement")
            .field("role", &self.role)
            .finish_non_exhaustive()
    }
}

/// A treatment context a verified credential attests.
#[derive(Clone, PartialEq, Eq)]
pub struct Attestation {
    /// The agreement the caller was admitted under.
    pub agreement: AttestingAgreement,
    /// The id of the assertion that attests the treatment context.
    pub assertion: String,
    /// The issuer of that assertion, when the credential names one.
    pub assertion_issuer: Option<String>,
    /// The start of the assertion's window, when it states one.
    pub not_before: Option<jiff::Timestamp>,
    /// The end of the assertion's window, excluded.
    pub not_on_or_after: jiff::Timestamp,
    /// The care role the credential attests, already held to the policy's
    /// `care_role` value set; `None` when it attests none of it.
    pub role: Option<ForeignCode>,
    /// The EHRs whose subject the attested patient identifier names.
    pub ehrs: Vec<EhrId>,
}

impl std::fmt::Debug for Attestation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Attestation")
            .field("agreement", &self.agreement)
            .field("ehrs", &self.ehrs.len())
            .finish_non_exhaustive()
    }
}

impl Attestation {
    /// Whether one of `agreements`, the policy's `attested_care`, names the
    /// agreement: its `id` is the container of the agreement's `ROLE`, and
    /// its `issuer`, when set, is the partner's federation issuer.
    #[must_use]
    pub fn named_by(&self, agreements: &[DvIdentifier]) -> bool {
        agreements.iter().any(|named| {
            // NOTE: owner decision on #6078, an id that is no UUID names no agreement ROLE, so
            // it admits no attested relationship.
            Uuid::parse_str(named.id.trim()).is_ok_and(|role| role == self.agreement.role)
                && named
                    .issuer
                    .as_deref()
                    .is_none_or(|issuer| issuer == self.agreement.issuer)
        })
    }

    /// Whether the assertion's window holds `now`.
    #[must_use]
    pub fn current_at(&self, now: jiff::Timestamp) -> bool {
        self.not_before.is_none_or(|start| start <= now) && now < self.not_on_or_after
    }

    /// Whether the caller holds an attested care relationship to the subject
    /// of `ehr` at `now` under a policy whose `attested_care` is
    /// `agreements`.
    #[must_use]
    pub fn holds(&self, agreements: &[DvIdentifier], ehr: EhrId, now: jiff::Timestamp) -> bool {
        self.ehrs.contains(&ehr) && self.current_at(now) && self.named_by(agreements)
    }

    /// The role of the attested relationship: the one the credential attests
    /// of the `care_role` value set, else `PART` (draft access control
    /// decision procedure, Holding a Care Relationship, `attested`).
    #[must_use]
    pub fn care_role(&self) -> ForeignCode {
        self.role.clone().unwrap_or_else(|| ForeignCode {
            terminology: TERMINOLOGY_ID_PARTICIPATION_TYPE.to_owned(),
            code: "PART".to_owned(),
        })
    }

    /// The assertion as the access log names it.
    #[must_use]
    pub fn assertion_ref(&self) -> DvIdentifier {
        DvIdentifier {
            issuer: self.assertion_issuer.clone(),
            assigner: None,
            id: self.assertion.clone(),
            r#type: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ROLE: Uuid = Uuid::from_u128(0x0190_b3a2_5a3e_7c1d_9e2f_3b4c_5d6e_7f99);
    const ISSUER: &str = "https://federation.example";

    fn attestation() -> Attestation {
        Attestation {
            agreement: AttestingAgreement {
                role: ROLE,
                issuer: ISSUER.to_owned(),
                name: "Regional exchange".to_owned(),
                version: "1".to_owned(),
            },
            assertion: "trc-1".to_owned(),
            assertion_issuer: None,
            not_before: Some(at(1_000)),
            not_on_or_after: at(2_000),
            role: None,
            ehrs: vec![EhrId(Uuid::from_u128(1))],
        }
    }

    fn at(second: i64) -> jiff::Timestamp {
        jiff::Timestamp::from_second(second).expect("an instant")
    }

    fn named(issuer: Option<&str>, id: &str) -> DvIdentifier {
        DvIdentifier {
            issuer: issuer.map(str::to_owned),
            assigner: None,
            id: id.to_owned(),
            r#type: None,
        }
    }

    #[test]
    fn an_agreement_is_named_by_its_role_container_and_partner_issuer() {
        let held = attestation();
        let role = ROLE.to_string();
        assert!(held.named_by(&[named(None, &role)]));
        assert!(held.named_by(&[named(Some(ISSUER), &role.to_uppercase())]));
        assert!(!held.named_by(&[named(Some("https://other.example"), &role)]));
        assert!(!held.named_by(&[named(None, &Uuid::from_u128(7).to_string())]));
        assert!(!held.named_by(&[named(None, "Regional exchange")]));
        assert!(!held.named_by(&[]));
    }

    #[test]
    fn it_holds_only_within_its_window_for_its_subject() {
        let held = attestation();
        let agreements = [named(None, &ROLE.to_string())];
        let ehr = EhrId(Uuid::from_u128(1));
        assert!(held.holds(&agreements, ehr, at(1_000)));
        assert!(held.holds(&agreements, ehr, at(1_999)));
        assert!(!held.holds(&agreements, ehr, at(999)), "not yet current");
        assert!(!held.holds(&agreements, ehr, at(2_000)), "expired");
        assert!(
            !held.holds(&agreements, EhrId(Uuid::from_u128(2)), at(1_500)),
            "another subject"
        );
        assert!(!held.holds(&[], ehr, at(1_500)), "no agreement named");
    }

    #[test]
    fn its_role_is_the_attested_one_else_part() {
        let mut held = attestation();
        assert_eq!(
            held.care_role(),
            ForeignCode {
                terminology: TERMINOLOGY_ID_PARTICIPATION_TYPE.to_owned(),
                code: "PART".to_owned(),
            }
        );
        let attending = ForeignCode {
            terminology: TERMINOLOGY_ID_PARTICIPATION_TYPE.to_owned(),
            code: "ATND".to_owned(),
        };
        held.role = Some(attending.clone());
        assert_eq!(held.care_role(), attending);
    }

    #[test]
    fn its_debug_names_no_assertion() {
        assert!(!format!("{:?}", attestation()).contains("trc-1"));
    }
}
