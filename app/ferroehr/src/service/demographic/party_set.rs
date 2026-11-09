// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A demographic CONTRIBUTION the server builds: several new parties whose
//! bodies reference each other, committed together.
//!
//! A provisioned professional is a `PERSON` that lists its `ROLE` in `roles`
//! and a `ROLE` that names the `PERSON` as `performer` (RM demographic
//! `org.openehr.rm.demographic.actor.adoc`, `…role.adoc`). Each body needs the
//! other's container id, so both ids are allocated before the commit, and one
//! CONTRIBUTION creates both (RM common `master06-change_control_package.adoc`
//! §Contributions: the change-set is committed atomically). No openEHR spec
//! governs when a server allocates an id: our own design.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694): the commit interior carries the canonical \
              fragment the seam produced once; stored-content serving"
)]

use openehr_rm::prelude::PartyProxy;
use serde_json::Value;
use sqlx::Connection as _;

use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::ids::AllocatedVoId;
use crate::service::FerroEhrService;
use crate::service::demographic::support;
use crate::service::demographic::types::PartyKind;
use crate::versioning::audit::change_type;
use crate::versioning::change::{self, Change, CommittedContribution, WriteEnvelope};
use crate::versioning::contribution::typed_decode_gate;
use crate::versioning::lifecycle;

/// One party a [`FerroEhrService::create_parties`] commits.
#[derive(Debug)]
pub struct NewParty {
    /// The container id, from [`FerroEhrService::allocate_party_id`].
    pub id: AllocatedVoId,
    /// The party's class.
    pub kind: PartyKind,
    /// The canonical body of its first version.
    pub body: Value,
    /// Its `version_lifecycle_state` code; `None` is `532|complete|`.
    pub lifecycle_state: Option<String>,
}

impl FerroEhrService {
    /// A fresh container id for a party a later [`Self::create_parties`]
    /// creates, so the bodies of that set can reference it.
    #[must_use]
    pub fn allocate_party_id(&self) -> AllocatedVoId {
        AllocatedVoId::minted(&self.stamp)
    }

    /// Commits the first version of each of `parties` in one demographic
    /// CONTRIBUTION, under the audit `description` and the committer
    /// `audited_by`, the caller's when `None`.
    ///
    /// Each body passes the gates of a member of a demographic CONTRIBUTION
    /// over ITS-REST: the typed decode, the RM invariants and the mapped
    /// template.
    ///
    /// # Errors
    /// The `400` and `422` of a body those gates refuse, naming its index; the
    /// commit refusals (an illegal first lifecycle state, a binding-key
    /// clash); a storage failure.
    pub async fn create_parties(
        &self,
        parties: Vec<NewParty>,
        description: &str,
        audited_by: Option<PartyProxy>,
    ) -> Result<CommittedContribution, ServiceError> {
        let mut audit = self.audit(change_type::CREATION, description);
        if let Some(party) = audited_by {
            audit.committer = party;
        }
        let mut changes = Vec::with_capacity(parties.len());
        for (index, party) in parties.into_iter().enumerate() {
            let kind = support::kind_of(party.kind);
            let incomplete = party.lifecycle_state.as_deref() == Some(lifecycle::state::INCOMPLETE);
            typed_decode_gate(kind, &party.body, incomplete)
                .map_err(|e| e.for_version_member(index))?;
            self.validate_for_commit(kind, &party.body, incomplete)
                .await
                .map_err(|e| e.for_version_member(index))?;
            let envelope = WriteEnvelope {
                lifecycle_state: party.lifecycle_state,
                ..WriteEnvelope::default()
            };
            changes.push((
                audit.clone(),
                Change::create_allocated(party.id, kind, party.body, envelope),
            ));
        }
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await?;
        let committed = change::commit_contribution(
            &mut tx,
            None,
            None,
            &audit,
            changes,
            Vec::new(),
            &self.signing_ctx(),
        )
        .await?;
        tx.commit().await?;
        Ok(committed)
    }
}
