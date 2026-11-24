// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller's own records: the EHRs whose `EHR_STATUS.subject` names the
//! `PERSON` the caller's authenticated identity is bound to, of which the
//! caller is the subject (EHDS Art. 3), and the records of the persons the
//! caller is kin to, with which a professional is in conflict (#6077). The
//! party binding is the one source of who the caller is. No openEHR spec
//! governs this: our own design (#6106).

use std::sync::PoisonError;

use openehr_rm::draft::access_control::terminology::ForeignCode;
use uuid::Uuid;

use super::{EhrEntry, RequestAccess};
use crate::access_eval::care_tie::Cared;
use crate::access_eval::procedure::Verdict;
use crate::ids::EhrId;
use crate::party_binding::grants::live::GrantsUnavailable;
use crate::service::ehr::access_settings::EhrAccessSettings;

/// How the caller holds one EHR.
#[derive(Debug, Clone, Default)]
pub(super) struct Held {
    /// The EHR is one a confined caller reaches.
    pub(super) own: bool,
    /// The EHR's subject is the `PERSON` the caller is bound to.
    pub(super) subject: bool,
    /// The kinships that join the caller to the EHR's subject.
    pub(super) kinships: Vec<ForeignCode>,
}

impl Held {
    /// How a caller, confined or not, holds every EHR of a settings class,
    /// whose subject a query leaves unread.
    pub(super) fn of_class(confined: bool) -> Self {
        Self {
            own: confined,
            subject: false,
            kinships: Vec::new(),
        }
    }
}

impl RequestAccess {
    /// The verdict for the EHR `ehr_id` with `settings`, the caller's care
    /// for its subject read as `care`; `None` stands for an EHR that is no
    /// subject caller's own.
    #[must_use]
    pub fn judge(
        &self,
        ehr_id: Option<EhrId>,
        settings: Option<&EhrAccessSettings>,
        care: &Cared,
    ) -> Verdict {
        let subject = self.is_record(ehr_id);
        // NOTE: SMART on openEHR master08 §Resource Scopes, a `patient/` grant reaches only the
        // launch context's EHR, so only `user/` grants reach the person's records by identity.
        let own = self.owns(ehr_id)
            || (subject && self.caller.own_ehrs.is_empty() && !self.caller.patient_grants_only);
        let kinships = ehr_id
            .and_then(|ehr_id| {
                self.kinships
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .get(&ehr_id)
                    .cloned()
            })
            .unwrap_or_default();
        self.judge_as(
            &Held {
                own,
                subject,
                kinships,
            },
            settings,
            care,
        )
    }

    /// Whether a read of this request found `ehr_id`'s subject to be the
    /// `PERSON` the caller is bound to.
    #[must_use]
    pub fn is_record(&self, ehr_id: Option<EhrId>) -> bool {
        ehr_id.is_some_and(|ehr_id| {
            self.records
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .contains(&ehr_id)
        })
    }

    /// Finds whether the subject `entry`, the copy of `ehr_id`, names is the
    /// `PERSON` the caller's authenticated identity is bound to, and keeps
    /// the answer for every later judgement of `ehr_id` in this request.
    ///
    /// The subject reference is the one the copy holds, whose revision every
    /// read of the EHR checks against the store, and the `PERSON` the binding
    /// of this request named. No openEHR spec governs this: our own design
    /// under EHDS Art. 3 (#6106).
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the subject's `PERSON` cannot be read.
    pub async fn bind_subject(
        &self,
        ehr_id: EhrId,
        entry: &EhrEntry,
    ) -> Result<bool, GrantsUnavailable> {
        let person = self.person_of(entry).await?;
        let named = person.is_some() && person == self.bound_person();
        let kinships =
            person.map_or_else(Vec::new, |person| self.caller().facts.kinships_with(person));
        {
            let mut held = self.kinships.lock().unwrap_or_else(PoisonError::into_inner);
            if kinships.is_empty() {
                held.remove(&ehr_id);
            } else {
                held.insert(ehr_id, kinships);
            }
        }
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        if named {
            records.insert(ehr_id);
        } else {
            records.remove(&ehr_id);
        }
        Ok(named)
    }

    /// The persons a professional is in conflict with and no conflict rule
    /// lets it read on an EHR whose settings admit it, each `PERSON` once:
    /// the one it is bound to on the professional path, and the persons it is
    /// kin to. A rule that allows a conflict only to a caller in care is read
    /// as not allowing it, since a query binds no subject's care for this
    /// (draft decision procedure step 5; our own design, #6077).
    #[must_use]
    pub fn conflicted_persons(&self) -> Vec<Uuid> {
        let caller = self.caller();
        if caller.kind != openehr_rm::draft::access_control::terminology::CallerKind::Professional {
            return Vec::new();
        }
        let mut persons: Vec<Uuid> = caller.facts.kin().iter().map(|kin| kin.person).collect();
        persons.extend(self.bound_person());
        persons.sort_unstable();
        persons.dedup();
        persons.retain(|person| {
            let held = Held {
                own: false,
                subject: Some(*person) == self.bound_person(),
                kinships: caller.facts.kinships_with(*person),
            };
            self.judge_as(&held, None, &Cared::No)
                .rules
                .iter()
                .any(|rule| {
                    rule.then
                        == crate::access_eval::procedure::Outcome::Refused(
                            openehr_rm::draft::access_control::terminology::Refusal::Conflict,
                        )
                })
        });
        persons
    }

    /// The `PERSON` the caller is bound to
    /// ([`Caller::party`](crate::access_eval::procedure::Caller::party)).
    fn bound_person(&self) -> Option<Uuid> {
        // NOTE: the local source names each party by its container's UUID, so a bound
        // party id that is no UUID names no PERSON of this store and no record.
        self.caller()
            .party
            .as_deref()
            .and_then(|p| p.parse::<Uuid>().ok())
    }

    /// The `PERSON` the subject reference `entry` names; `None` when it names
    /// none, or when the caller is bound to no `PERSON` and is kin to none,
    /// so nothing of the subject's `PERSON` is read.
    async fn person_of(&self, entry: &EhrEntry) -> Result<Option<Uuid>, GrantsUnavailable> {
        let (Some(grants), Some(subject)) = (&self.grants, &entry.subject) else {
            return Ok(None);
        };
        if self.bound_person().is_none() && self.caller().facts.kin().is_empty() {
            return Ok(None);
        }
        grants.person_of(subject).await
    }
}
