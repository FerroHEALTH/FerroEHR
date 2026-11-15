// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care step's reads of [`RequestAccess`]: the subject references the
//! caller cares for, and whether it cares for the `PERSON` an EHR's subject
//! reference names (#5615, #5634). No openEHR spec governs where the
//! subject's party is found: our own design.

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
