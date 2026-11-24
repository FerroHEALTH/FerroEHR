// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The declared-episode rule of a COMPOSITION write (draft access control
//! package, Episodes of Care): every episode a composition names is the
//! `episode` of a care relationship of its EHR's subject, live or ended.
//!
//! A write judges it before its clinical transaction, and only when the
//! composition is placed, from the care copy. Our own design (#6412).

use std::collections::BTreeSet;

use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::FerroEhrService;
use crate::versioning::body::Body;
use crate::versioning::episode::{self, EpisodeKey};

impl FerroEhrService {
    /// Refuses the COMPOSITION `body` written into `ehr_id` when it names an
    /// episode no care relationship of the EHR's subject names; reads nothing
    /// when it names none.
    ///
    /// # Errors
    /// [`ServiceError::Unprocessable`] (`422`) naming the element for a
    /// malformed placement or an undeclared episode, and the read's error
    /// when the subject or its relationships cannot be read.
    pub(crate) async fn placement_declared(
        &self,
        ehr_id: EhrId,
        body: &Body,
    ) -> Result<(), ServiceError> {
        let placed = body.episodes()?;
        if placed.is_empty() {
            return Ok(());
        }
        self.episodes_declared(ehr_id, &placed).await
    }

    /// Refuses the placement `placed` of a composition in `ehr_id` unless
    /// every key is the episode of a care relationship of the EHR's subject.
    ///
    /// # Errors
    /// As [`Self::placement_declared`].
    pub(crate) async fn episodes_declared(
        &self,
        ehr_id: EhrId,
        placed: &[EpisodeKey],
    ) -> Result<(), ServiceError> {
        let declared: BTreeSet<EpisodeKey> = match self.ehr_subject_person(ehr_id).await? {
            Some(person) => self
                .grants
                .under_care_of(person)
                .await?
                .iter()
                .filter_map(|care| care.episode().map(|episode| episode.key))
                .collect(),
            None => BTreeSet::new(),
        };
        if placed.iter().all(|key| declared.contains(key)) {
            Ok(())
        } else {
            Err(episode::undeclared())
        }
    }
}
