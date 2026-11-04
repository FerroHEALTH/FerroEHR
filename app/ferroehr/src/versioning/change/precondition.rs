// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The preconditions of a single-object write, judged inside its
//! transaction: the EHR's `is_modifiable` gate (RM ehr master04 §EHR Active
//! Status), the per-object lock with the tip it holds, and the `If-Match`
//! precondition against that tip (ITS-REST overview `Requests_and_responses.md`
//! §"If-Match and accidental overwrites").

use openehr_base::prelude::ObjectVersionId;
use openehr_base::v1_3::base_types::identification::lexical::composite_ids_equal;
use sqlx::PgConnection;

use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::ids::{EhrId, VoId};
use crate::versioning::Kind;
use crate::versioning::object_version_id::{TreeId, object_version_id};

/// The `409 Conflict` for a content write to a deactivated EHR
/// (`EHR_STATUS.is_modifiable = false`) — RM ehr master04 §EHR Active Status
/// forbids writes to everything except the `EHR_STATUS` object. ITS-REST
/// 1.1.0 enumerates no status code for this outcome; `409` is the closest
/// HTTP semantics (RFC 9110 §15.5.10 — the write conflicts with the current
/// state of the target resource).
pub(crate) fn not_modifiable_error(ehr_id: EhrId) -> ServiceError {
    ServiceError::conflict(format!(
        "EHR {ehr_id} is not modifiable (EHR_STATUS.is_modifiable = false); its \
         contents cannot be created, updated or deleted (RM ehr master04 §EHR Active \
         Status). Set EHR_STATUS.is_modifiable = true to reactivate it."
    ))
}

/// The content-write gate, evaluated INSIDE the commit transaction.
///
/// Reads the promoted `ehr.is_modifiable` under a row lock
/// ([`crate::storage::ehr_repo::ehr_is_modifiable_locked`]), so a concurrent
/// `EHR_STATUS` flip either commits first and is seen or waits for this commit.
/// A missing EHR row reads as modifiable; the existence gate answers 404 on its
/// own.
///
/// NOTE: RM ehr master04 §EHR Active Status defines what the flag forbids and no
/// released text states when it is evaluated relative to a commit, so the
/// in-transaction snapshot is our own recorded semantics.
///
/// # Errors
/// [`ServiceError::Conflict`] when the EHR is not modifiable;
/// [`ServiceError::Database`] if the locked read fails.
pub(crate) async fn ensure_content_writable_tx(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    exclusive: bool,
) -> Result<(), ServiceError> {
    let modifiable = crate::storage::ehr_repo::ehr_is_modifiable_locked(tx, ehr_id, exclusive)
        .await?
        .unwrap_or(true);
    if modifiable {
        Ok(())
    } else {
        Err(not_modifiable_error(ehr_id))
    }
}

/// Take the per-object lock of `vo_id` and read its current trunk tip, the
/// placement a single-object modify or delete then continues
/// ([`super::Write::placed`]), so its preconditions are judged against the tip the
/// write holds.
///
/// # Errors
/// [`ServiceError::NotFound`] when the object has no current version or is
/// not a `kind` in `ehr_id`; [`ServiceError::Database`] if the read fails.
pub(crate) async fn lock_current(
    tx: &mut PgConnection,
    ehr_id: Option<EhrId>,
    vo_id: VoId,
    kind: Kind,
) -> Result<crate::storage::version_repo::placement::Placement, ServiceError> {
    crate::storage::version_repo::commit::advisory_lock(tx, vo_id).await?;
    let placement =
        crate::storage::version_repo::placement::next_placement(tx, vo_id, None).await?;
    if placement
        .tip
        .as_ref()
        .is_some_and(|tip| tip.ehr_id == ehr_id && tip.kind == kind.as_str())
    {
        Ok(placement)
    } else {
        Err(ServiceError::not_found(
            Missing::VersionedObject,
            format!("{} {vo_id} in EHR {ehr_id:?}", kind.as_str()),
        ))
    }
}

/// The full `OBJECT_VERSION_ID` of the tip a [`lock_current`] placement holds.
pub(crate) fn tip_uid(
    vo_id: VoId,
    tip: &crate::storage::version_repo::placement::TipRow,
) -> String {
    object_version_id(
        vo_id,
        &tip.creating_system_id,
        TreeId::from_columns(tip.trunk_version, tip.branch_number, tip.branch_version),
    )
}

/// The `If-Match` precondition of a single-object write, judged once against
/// the tip the write locked: `preceding` must name that version in full,
/// compared case-insensitively (ITS-REST overview `Requests_and_responses.md`
/// §"If-Match and accidental overwrites"; BASE `base_types` master05
/// §"Composite Identifiers and Case").
///
/// # Errors
/// [`ServiceError::VersionConflict`] (`412`) when `preceding` names another
/// version.
pub(crate) fn ensure_preceding(
    tip_uid: &str,
    preceding: Option<&ObjectVersionId>,
) -> Result<(), ServiceError> {
    match preceding {
        Some(pre) if !composite_ids_equal(tip_uid, pre.value()) => {
            Err(ServiceError::version_conflict(format!(
                "If-Match {:?} does not match the current latest version {tip_uid:?}",
                pre.value()
            )))
        }
        _ => Ok(()),
    }
}
