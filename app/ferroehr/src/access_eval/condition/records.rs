// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The caller's own records: the EHRs whose `EHR_STATUS.subject` names the
//! `PERSON` the caller's authenticated identity is bound to, of which the
//! caller is the subject (EHDS Art. 3). The party binding is the one source
//! of who that is. No openEHR spec governs this: our own design (#6106).

use std::sync::PoisonError;

use uuid::Uuid;

use super::{EhrEntry, RequestAccess};
use crate::access_eval::procedure::Verdict;
use crate::ids::EhrId;
use crate::party_binding::grants::live::GrantsUnavailable;
use crate::service::ehr::access_settings::EhrAccessSettings;

/// How the caller holds one EHR.
#[derive(Debug, Clone, Copy, Default)]
pub(super) struct Held {
    /// The EHR is one a confined caller reaches.
    pub(super) own: bool,
    /// The EHR's subject is the `PERSON` the caller is bound to.
    pub(super) subject: bool,
}

impl Held {
    /// How a caller, confined or not, holds every EHR of a settings class,
    /// whose subject a query leaves unread.
    pub(super) fn of_class(confined: bool) -> Self {
        Self {
            own: confined,
            subject: false,
        }
    }
}

impl RequestAccess {
    /// The verdict for the EHR `ehr_id` with `settings`, the caller `cared`
    /// for its subject or not; `None` stands for an EHR that is no subject
    /// caller's own.
    #[must_use]
    pub fn judge(
        &self,
        ehr_id: Option<EhrId>,
        settings: Option<&EhrAccessSettings>,
        cared: bool,
    ) -> Verdict {
        let subject = self.is_record(ehr_id);
        // NOTE: our own design (#6106), a confined caller with no launch context reaches the
        // records the party binding names as its own, and one with a context only that EHR.
        let own = self.owns(ehr_id) || (subject && self.caller.own_ehrs.is_empty());
        self.judge_as(Held { own, subject }, settings, cared)
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
        let named = self.names_the_caller(entry).await?;
        let mut records = self.records.lock().unwrap_or_else(PoisonError::into_inner);
        if named {
            records.insert(ehr_id);
        } else {
            records.remove(&ehr_id);
        }
        Ok(named)
    }

    /// Whether the subject reference `entry` names is the `PERSON` the
    /// caller is bound to ([`Caller::party`](crate::access_eval::procedure::Caller::party)).
    async fn names_the_caller(&self, entry: &EhrEntry) -> Result<bool, GrantsUnavailable> {
        let (Some(grants), Some(subject)) = (&self.grants, &entry.subject) else {
            tracing::debug!(
                grants = self.grants.is_some(),
                subject = entry.subject.is_some(),
                "the EHR names no subject this request can bind"
            );
            return Ok(false);
        };
        // NOTE: the local source names each party by its container's UUID, so a bound
        // party id that is no UUID names no PERSON of this store and no record.
        let Some(party) = self
            .caller()
            .party
            .as_deref()
            .and_then(|p| p.parse::<Uuid>().ok())
        else {
            tracing::debug!(
                party = self.caller().party.is_some(),
                "the caller is bound to no PERSON of this store"
            );
            return Ok(false);
        };
        let person = grants.person_of(subject).await?;
        tracing::debug!(
            resolved = person.is_some(),
            named = person == Some(party),
            "the subject's PERSON against the caller's"
        );
        Ok(person == Some(party))
    }
}
