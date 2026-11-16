// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care step of the access evaluator.
//!
//! It holds the reads of [`RequestAccess`] (the subject references the
//! caller cares for, and whether it cares for the `PERSON` an EHR's subject
//! reference names, #5615, #5634) and the forms in which a query reads care
//! ([`Care`], #5699). A decision care admits is taken only on a copy of the
//! caller facts that holds the store's revisions (#5806). No openEHR spec
//! governs where the subject's party is found or how fresh the copy is: our
//! own design.

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use uuid::Uuid;

use super::{EhrEntry, RequestAccess};
use crate::access_eval::stale::{STALE_POLL, STALE_WAIT};
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::party_binding::grants::caller::CareTeam;
use crate::party_binding::grants::live::GrantsUnavailable;
use crate::storage::access_projection::EhrSubject;

/// Why a care decision could not be taken.
#[derive(Debug, thiserror::Error)]
pub enum CareUnsettled {
    /// The care copy missed and could not read.
    #[error(transparent)]
    Unavailable(#[from] GrantsUnavailable),
    /// The caller facts copy stayed older than the store for the whole wait.
    #[error("the caller facts copy stayed older than the store")]
    Refreshing,
}

impl From<CareUnsettled> for ServiceError {
    /// A copy that cannot be read answers `503`; one that stays older than
    /// the store answers `503` with the `access-refreshing` problem.
    fn from(error: CareUnsettled) -> Self {
        match error {
            CareUnsettled::Unavailable(error) => error.into(),
            CareUnsettled::Refreshing => Self::access_refreshing(),
        }
    }
}

/// Whether a request found the caller facts copy in force, and the care
/// teams the caller still holds once the copy was read again.
#[derive(Debug, Default)]
pub(super) struct CareInForce(Mutex<Checked>);

/// What a request's check of the caller facts copy found.
#[derive(Debug, Default, Clone)]
enum Checked {
    /// Not checked yet.
    #[default]
    Unchecked,
    /// The copy held the store's revisions.
    Held,
    /// The copy was older and was read again: the care teams the caller is
    /// a member of in the copy read, by container.
    ReadAgain(Vec<Uuid>),
}

impl CareInForce {
    fn get(&self) -> Checked {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn set(&self, checked: Checked) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = checked;
    }
}

impl RequestAccess {
    /// The subject references of the `PERSON`s the caller has a live care
    /// relationship to
    /// ([`crate::party_binding::grants::live::LiveGrants::cared_for_by`]); a
    /// caller of no care team reads nothing.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the care copy misses and cannot read.
    pub async fn cared_subjects(&self) -> Result<Vec<EhrSubject>, GrantsUnavailable> {
        let teams = self.care_team_ids();
        let Some(grants) = self.grants.as_ref() else {
            return Ok(Vec::new());
        };
        grants
            .cared_for_by(&teams, self.today(), self.policy().grace)
            .await
    }

    /// The care teams the caller is a member of, by container.
    #[must_use]
    pub fn care_team_ids(&self) -> Vec<Uuid> {
        self.care_teams_held().iter().map(CareTeam::team).collect()
    }

    /// The care teams of the caller's facts, less any the caller was found
    /// to have left when the copy was read again ([`Self::care_in_force`]).
    pub(super) fn care_teams_held(&self) -> Vec<CareTeam> {
        let teams = self.caller.facts.care_teams().iter().copied();
        match self.care.get() {
            Checked::ReadAgain(held) => teams.filter(|team| held.contains(&team.team())).collect(),
            Checked::Unchecked | Checked::Held => teams.collect(),
        }
    }

    /// Whether the caller facts copy holds the store's revisions, so a
    /// decision care admits may be taken on it: `true` when it did, `false`
    /// when it was older and was read again, after which the caller's care
    /// is read again too. Checked once per request, with one indexed read.
    ///
    /// The care teams the caller is a member of in the copy read are those
    /// it keeps; one it joined since the request began waits for the next
    /// request, which fails closed. A copy that stays older for
    /// [`STALE_WAIT`] refuses the request. No openEHR spec governs the
    /// copy: our own design (#5806).
    ///
    /// # Errors
    /// [`CareUnsettled::Unavailable`] when the store does not answer, and
    /// [`CareUnsettled::Refreshing`] once the wait has passed.
    pub async fn care_in_force(&self) -> Result<bool, CareUnsettled> {
        let Some(grants) = self.grants.as_ref() else {
            return Ok(true);
        };
        if !matches!(self.care.get(), Checked::Unchecked) {
            return Ok(true);
        }
        let Some(store) = grants.behind().await? else {
            self.care.set(Checked::Held);
            return Ok(true);
        };
        let since = Instant::now();
        loop {
            grants.refresh().await?;
            if grants.holds(store) {
                break;
            }
            if since.elapsed() >= STALE_WAIT {
                count("refused");
                return Err(CareUnsettled::Refreshing);
            }
            tokio::time::sleep(STALE_POLL).await;
        }
        let held = self.caller.facts.party().map_or_else(Vec::new, |party| {
            grants
                .facts_of(party, self.now)
                .care_teams()
                .iter()
                .map(CareTeam::team)
                .collect()
        });
        self.care.set(Checked::ReadAgain(held));
        count("judged_again");
        Ok(false)
    }

    /// Whether the verdict of `ehr_id` as `entry` reads it depends on the
    /// caller's care for its subject.
    pub(super) fn care_decides(&self, ehr_id: EhrId, entry: &EhrEntry) -> bool {
        let settings = (*entry.settings).as_ref();
        let (with, without) = (
            self.judge(Some(ehr_id), settings, true),
            self.judge(Some(ehr_id), settings, false),
        );
        (with.rules, with.otherwise, with.lifted)
            != (without.rules, without.otherwise, without.lifted)
    }

    /// Of the EHRs `entries` name, those whose subject the caller has a live
    /// care relationship to, judged as the care step judges one EHR.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the care copy misses and cannot read.
    pub async fn cared_ehrs(
        &self,
        entries: &[(EhrId, EhrEntry)],
    ) -> Result<Vec<Uuid>, GrantsUnavailable> {
        let mut cared = Vec::new();
        for (ehr_id, entry) in entries {
            if self.cares_for(entry).await? {
                cared.push(ehr_id.0);
            }
        }
        Ok(cared)
    }

    /// Whether the caller has a live care relationship to the `PERSON` the
    /// subject reference of `entry` names; a caller of no care team has none,
    /// and reads nothing for it.
    pub(super) async fn cares_for(&self, entry: &EhrEntry) -> Result<bool, GrantsUnavailable> {
        let under_care_of = match (&self.grants, &entry.subject) {
            (Some(grants), Some(subject)) if !self.care_teams_held().is_empty() => {
                match grants.person_of(subject).await? {
                    Some(person) => grants.under_care_of(person).await?,
                    None => Vec::new(),
                }
            }
            _ => Vec::new(),
        };
        Ok(self.cared(&under_care_of))
    }
}

/// How a condition reads the caller's care for the subject of an EHR.
///
/// A query pinned to EHRs it names reads their care from the evaluator's
/// copy and binds only the EHRs it found cared for; any other query binds
/// the subjects of every patient under the caller's care teams. The service
/// resolves [`Care::Unresolved`] before the statement is built, once it knows
/// which EHRs the query reads (#5699). No openEHR spec governs this: our own
/// design.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Care {
    /// Not read yet: the service reads it once it knows the query's EHRs.
    Unresolved,
    /// The subject references of every `PERSON` the caller has a live care
    /// relationship to.
    Census(Arc<[EhrSubject]>),
    /// Of the EHRs the query is pinned to, those whose subject the caller has
    /// a live care relationship to; every other EHR is uncared.
    Ehrs(Arc<[Uuid]>),
    /// The clinical care projection, read by the EHR row's subject digest:
    /// the caller's care teams and the day and grace a relationship is live
    /// on, as [`CareTeam::live_on`] judges it.
    Projected(ProjectedCare),
}

/// What a probe of the clinical care projection binds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedCare {
    /// The care teams the caller is a member of.
    pub teams: Arc<[Uuid]>,
    /// The day of the request.
    pub day: jiff::civil::Date,
    /// The grace after a relationship ends, as a PostgreSQL interval of
    /// calendar units only (`P14D`).
    pub grace: String,
}

/// Increment `access_care_stale_total{outcome}`.
fn count(outcome: &'static str) {
    crate::telemetry::access_path::metrics()
        .care_stale
        .add(1, &[opentelemetry::KeyValue::new("outcome", outcome)]);
}

#[cfg(test)]
#[path = "care_tests.rs"]
mod tests;
