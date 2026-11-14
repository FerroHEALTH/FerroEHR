// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The hooks that run in a commit's transaction once its version is written:
//! the access-delay hold of a new COMPOSITION, the party binding side tables
//! of a demographic version, the projection of an `EHR_ACCESS` version and
//! the restriction and opt-out marks its directives in force set.
//!
//! Each writes rows derived from the version in the same transaction, so a
//! rolled-back commit leaves none. No openEHR spec governs these derived
//! rows: our own design.

use sqlx::PgConnection;

use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::party_binding::derive::Facts;
use crate::service::ehr::access_projection::AccessCommit;
use crate::versioning::change::Committed;
use crate::versioning::{Kind, SigningCtx};

/// What the hooks of one written version read beside the version itself,
/// captured before the change was resolved.
#[derive(Debug)]
pub(crate) struct Hooks<'a> {
    /// The owning EHR, or `None` for an ehr-less object.
    pub(crate) ehr_id: Option<EhrId>,
    /// What a party body holds that the binding hooks read.
    pub(crate) party_facts: Option<&'a Facts>,
    /// What an `EHR_ACCESS` version projects.
    pub(crate) access: Option<AccessCommit>,
}

impl Hooks<'_> {
    /// Runs every hook for `committed`, on its transaction.
    ///
    /// # Errors
    /// The first hook's error: the hold, the binding or the projection
    /// write.
    pub(crate) async fn run(
        self,
        tx: &mut PgConnection,
        ctx: &SigningCtx<'_>,
        committed: &Committed,
    ) -> Result<(), ServiceError> {
        crate::versioning::access_delay::hold_on_creation(tx, ctx, self.ehr_id, committed).await?;
        if committed.kind.is_demographic() {
            Box::pin(crate::party_binding::store::reindex_committed(
                tx,
                ctx,
                committed,
                self.party_facts,
            ))
            .await?;
        }
        if let (Some(ehr_id), Some(access)) = (self.ehr_id, self.access) {
            access.project(tx, ehr_id, committed).await?;
        }
        if let Some(ehr_id) = self.ehr_id
            && committed.kind == Kind::EhrAccess
        {
            crate::service::ehr::access_marks::project(tx, ehr_id).await?;
        }
        Ok(())
    }
}
