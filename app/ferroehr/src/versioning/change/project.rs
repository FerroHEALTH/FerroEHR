// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The projections a committed version updates in the commit's own
//! transaction. No openEHR spec governs a projection: our own design.

use sqlx::PgConnection;

use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::service::ehr::access_projection::AccessCommit;
use crate::versioning::change::Committed;
use crate::versioning::{Kind, SigningCtx};

/// The projections a committed version updates in its own transaction: the
/// EHDS Art. 3(3) hold of a new object, the marks of the `restriction` and
/// `opt_out` directives of an `EHR_ACCESS`, the projection of its settings
/// (`access`, [`AccessCommit`]), and the binding side table of a demographic
/// party.
///
/// # Errors
/// The errors of each projection.
pub(super) async fn committed(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    ehr_id: Option<EhrId>,
    committed: &Committed,
    party_facts: Option<&crate::party_binding::derive::Facts>,
    access: Option<AccessCommit>,
) -> Result<(), ServiceError> {
    crate::versioning::access_delay::hold_on_creation(tx, ctx, ehr_id, committed).await?;
    if let Some(ehr_id) = ehr_id
        && committed.kind == Kind::EhrAccess
    {
        crate::service::ehr::access_marks::project(tx, ehr_id).await?;
    }
    if let (Some(ehr_id), Some(access)) = (ehr_id, access) {
        access.project(tx, ehr_id, committed).await?;
    }
    if committed.kind.is_demographic() {
        Box::pin(crate::party_binding::store::reindex_committed(
            tx,
            ctx,
            committed,
            party_facts,
        ))
        .await?;
    }
    Ok(())
}
