// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The restriction gates every commit passes inside its transaction: GDPR
//! Art. 18 restriction of processing and the EHDS Art. 8 restriction of
//! access. No openEHR spec governs either: our own design/extension.

use sqlx::PgConnection;

use crate::ids::{EhrId, VoId};
use crate::service::error::ServiceError;
use crate::service::status::ServiceStatus;

/// Refuse a write that would process an object whose processing is restricted.
///
/// GDPR Art. 18(2) leaves storage as the only processing a restricted object
/// admits (`docs/law/eu/gdpr/text.html`), so a new version of it, a logical
/// delete of it or an attestation appended to it is refused — while the rows
/// already stored stay exactly where they are. The mark is read inside the
/// commit transaction, like the `is_modifiable` gate of the commit engine, so a
/// concurrent lift either commits first and is seen or waits for this commit.
/// Both grains are checked: a whole-EHR restriction reaches an object that has
/// no mark of its own yet, because it is created after the restriction was
/// recorded. No openEHR spec governs restriction of processing — our own
/// design/extension.
///
/// # Errors
/// [`ServiceError::Restricted`] (`403`) when either grain is marked;
/// [`ServiceError::Database`] if the marks cannot be read.
pub(crate) async fn ensure_not_restricted_tx(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: Option<VoId>,
) -> Result<(), ServiceError> {
    if let Some(ehr_id) = ehr_id {
        let restricted: Option<bool> =
            sqlx::query_scalar("SELECT restricted_at IS NOT NULL FROM ehr WHERE id = $1")
                .bind(ehr_id)
                .fetch_optional(&mut *tx)
                .await?;
        if restricted == Some(true) {
            return Err(ServiceError::restricted(&format!("EHR {ehr_id}")));
        }
    }
    if let Some(vo_id) = vo_id
        && crate::storage::marks::is_restricted(&mut *tx, vo_id).await?
    {
        return Err(ServiceError::restricted(&format!(
            "versioned object {vo_id}"
        )));
    }
    ensure_not_access_restricted_tx(tx, ehr_id, vo_id).await?;
    crate::versioning::access_delay::ensure_not_withheld_tx(tx, vo_id).await
}

/// Refuse, as though the target did not exist, a write a health professional
/// makes to data under an EHDS Art. 8 access restriction, whose fact is "not
/// visible to healthcare providers" (`docs/law/eu/ehds/text.html` Art. 8),
/// while the request enforces it. No openEHR spec governs restriction of
/// access — our own design/extension.
///
/// # Errors
/// [`ServiceError::NotFound`] (`ehr_id_does_not_exist` or
/// `versioned_object_does_not_exist`) when either grain is marked;
/// [`ServiceError::Database`] if the marks cannot be read.
async fn ensure_not_access_restricted_tx(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: Option<VoId>,
) -> Result<(), ServiceError> {
    if !crate::system_log::access_context::access_restriction_enforced() {
        return Ok(());
    }
    if let Some(ehr_id) = ehr_id {
        let restricted: Option<bool> =
            sqlx::query_scalar("SELECT access_restricted_at IS NOT NULL FROM ehr WHERE id = $1")
                .bind(ehr_id)
                .fetch_optional(&mut *tx)
                .await?;
        if restricted == Some(true) {
            return Err(ServiceError::new(
                ServiceStatus::EhrIdDoesNotExist,
                format!("EHR {ehr_id}"),
            ));
        }
    }
    if let Some(vo_id) = vo_id
        && crate::storage::marks::is_access_restricted(&mut *tx, vo_id).await?
    {
        return Err(ServiceError::new(
            ServiceStatus::VersionedObjectDoesNotExist,
            format!("versioned object {vo_id}"),
        ));
    }
    Ok(())
}
