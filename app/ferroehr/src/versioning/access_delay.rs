// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The commit side of the EHDS Art. 3(3) delay of the person's own access: a
//! new COMPOSITION of a configured category is held at creation, and a
//! patient-context write to a held object is refused as absent.
//!
//! Art. 3(1) asks access "immediately after the personal electronic health
//! data have been registered", and Art. 3(3) lets a Member State delay it
//! until a professional has explained the data (`docs/law/eu/ehds/text.html`),
//! so the hold is placed in the transaction that registers the data. No
//! openEHR spec governs a delay of the person's access: our own
//! design/extension.

use sqlx::PgConnection;

use crate::ids::{EhrId, VoId};
use crate::service::error::ServiceError;
use crate::service::status::ServiceStatus;
use crate::versioning::change::Committed;
use crate::versioning::{Kind, SigningCtx};

/// Hold the first version of a COMPOSITION the configured categories cover,
/// inside its commit transaction.
///
/// A later version leaves the object as it is: once released, data stays
/// visible, and a professional holds an amended object by hand. A write the
/// person makes is never held from that person.
///
/// # Errors
/// [`ServiceError::Storage`] when the hold cannot be written.
pub(crate) async fn hold_on_creation(
    tx: &mut PgConnection,
    ctx: &SigningCtx<'_>,
    ehr_id: Option<EhrId>,
    committed: &Committed,
) -> Result<(), ServiceError> {
    let (Some(policy), Some(ehr_id)) = (ctx.access_delay, ehr_id) else {
        return Ok(());
    };
    // NOTE: EHDS Art. 3(3), the delay withholds data from the person until a professional
    // explains it, so data the person registers themself is not held from them.
    if committed.kind != Kind::Composition
        || committed.sys_version != 1
        || policy.categories().is_empty()
        || crate::system_log::access_context::access_delay_enforced()
    {
        return Ok(());
    }
    crate::storage::access_delay::hold_new_composition(
        tx,
        ehr_id,
        committed.vo_id,
        (
            committed.template_id.as_deref(),
            committed.root_archetype.as_deref(),
        ),
        policy.categories(),
        policy.hours(),
    )
    .await?;
    Ok(())
}

/// Refuse, as though the object did not exist, a patient-context write to an
/// object withheld from the person (EHDS Art. 3(3)).
///
/// # Errors
/// [`ServiceError::NotFound`] (`versioned_object_does_not_exist`) when the
/// object is withheld; [`ServiceError::Storage`] if the mark cannot be read.
pub(crate) async fn ensure_not_withheld_tx(
    tx: &mut PgConnection,
    vo_id: Option<VoId>,
) -> Result<(), ServiceError> {
    let Some(vo_id) = vo_id else {
        return Ok(());
    };
    if crate::system_log::access_context::access_delay_enforced()
        && crate::storage::access_delay::is_withheld(tx, vo_id).await?
    {
        return Err(ServiceError::new(
            ServiceStatus::VersionedObjectDoesNotExist,
            format!("versioned object {vo_id}"),
        ));
    }
    Ok(())
}
