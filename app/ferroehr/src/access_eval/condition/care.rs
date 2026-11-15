// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care step of the access evaluator.
//!
//! It holds the reads of [`RequestAccess`] (the subject references the
//! caller cares for, and whether it cares for the `PERSON` an EHR's subject
//! reference names, #5615, #5634) and the forms in which a query reads care
//! ([`Care`], #5699). No openEHR spec governs where the subject's party is
//! found: our own design.

use std::sync::Arc;

use uuid::Uuid;

use super::{EhrEntry, RequestAccess};
use crate::party_binding::grants::caller::CareTeam;
use crate::party_binding::grants::live::GrantsUnavailable;
use crate::storage::access_projection::EhrSubject;

impl RequestAccess {
    /// The subject references of the `PERSON`s the caller has a live care
    /// relationship to
    /// ([`crate::party_binding::grants::live::LiveGrants::cared_for_by`]); a
    /// caller of no care team reads nothing.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the care copy misses and cannot read.
    pub async fn cared_subjects(&self) -> Result<Vec<EhrSubject>, GrantsUnavailable> {
        let teams: Vec<Uuid> = self
            .caller
            .facts
            .care_teams()
            .iter()
            .map(CareTeam::team)
            .collect();
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
        self.caller
            .facts
            .care_teams()
            .iter()
            .map(CareTeam::team)
            .collect()
    }

    /// Of the EHRs `entries` name, those whose subject the caller has a live
    /// care relationship to, as [`Self::cares_for`] judges each.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the care copy misses and cannot read.
    pub async fn cared_ehrs(
        &self,
        entries: &[(crate::ids::EhrId, EhrEntry)],
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
            (Some(grants), Some(subject)) if !self.caller.facts.care_teams().is_empty() => {
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
