// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! What [`LiveGrants`] answers about the subject of an EHR.
//!
//! It answers from the care copy, or with one indexed read on a miss: the
//! `PERSON` an EHR subject reference names, the references that name a
//! `PERSON`, and the care relationships that hang off it. No openEHR spec
//! governs where the subject's party is found: our own design (#5634).

use uuid::Uuid;

use super::{GrantsUnavailable, LiveGrants};
use crate::party_binding::grants::caller::CareTeam;
use crate::storage::access_projection::EhrSubject;
use crate::storage::error::StorageError;

impl LiveGrants {
    /// The `PERSON` the EHR subject reference `subject` names.
    ///
    /// A reference in this CDR's own namespace whose id is a UUID names the
    /// `PERSON` of that container, with no read; any other names the
    /// `complete` or `incomplete` `PERSON` that declares it by a record
    /// subject element, from the care copy or read on a miss; otherwise
    /// there is none, and a step reading the subject's `PERSON` fails
    /// closed. Both are legal subject references (BASE `OBJECT_REF`, RM
    /// Common IM §`PARTY_SELF`), so the namespace is compared with the id.
    /// The order is our own design (#5634).
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when a miss cannot be read.
    pub async fn person_of(&self, subject: &EhrSubject) -> Result<Option<Uuid>, GrantsUnavailable> {
        if *subject.namespace == *self.namespace() {
            // NOTE: the local source names each party by its container's UUID, so an id
            // in the own namespace that is no UUID names no PERSON of this store.
            return Ok(subject.id.parse::<Uuid>().ok());
        }
        Ok(self.care.person_of(&self.pool, subject).await?)
    }

    /// The subject references that name the `PERSON` `person`: its own
    /// reference in this CDR's namespace, then each record subject it
    /// declares while it is `complete` or `incomplete`, read on the party
    /// pool.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when the read fails.
    pub async fn subjects_of(&self, person: Uuid) -> Result<Vec<EhrSubject>, GrantsUnavailable> {
        let mut conn = self.pool.acquire().await.map_err(StorageError::from)?;
        let declared =
            crate::party_binding::grants::caller::subject::declared_by(&mut conn, &[person])
                .await?;
        let mut out = vec![EhrSubject::new(&*self.namespace(), person.to_string())];
        out.extend(declared.into_iter().map(|(_, subject)| subject));
        Ok(out)
    }

    /// The care teams and units the subject `subject` is under the care of:
    /// each `under_care_of` relationship of its live `PERSON` head with the
    /// dates it holds, whether or not it holds today, so the evaluator judges
    /// a discharge and the grace after it; from the care copy, or read on a
    /// miss.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when a miss cannot be read.
    pub async fn under_care_of(&self, subject: Uuid) -> Result<Vec<CareTeam>, GrantsUnavailable> {
        Ok(self.care.of_subject(&self.pool, subject).await?.to_vec())
    }

    /// The subject references of the `PERSON`s under the care of any of
    /// `teams` on `day`, with `grace` after each relationship ends: the
    /// patients a member of those teams has a live care relationship to
    /// ([`CareTeam::live_on`]), each by its own reference in this CDR's
    /// namespace and by every record subject it declares, sorted and
    /// distinct; from the care copy, or read on a miss.
    ///
    /// # Errors
    /// [`GrantsUnavailable`] when a miss cannot be read.
    pub async fn cared_for_by(
        &self,
        teams: &[Uuid],
        day: jiff::civil::Date,
        grace: jiff::Span,
    ) -> Result<Vec<EhrSubject>, GrantsUnavailable> {
        let namespace = self.namespace();
        let mut subjects = Vec::new();
        for team in teams {
            let census = self.care.of_team(&self.pool, *team).await?;
            for (person, _) in census
                .cared
                .iter()
                .filter(|(_, care)| care.live_on(day, grace))
            {
                subjects.push(EhrSubject::new(&*namespace, person.to_string()));
                subjects.extend(census.declared.get(person).into_iter().flatten().cloned());
            }
        }
        subjects.sort_unstable();
        subjects.dedup();
        Ok(subjects)
    }
}
