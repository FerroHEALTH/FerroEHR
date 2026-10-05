// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `vo_attestation` row I/O: attach an `ATTESTATION` to a stored version, look
//! up the version an attestation item addresses, and enumerate an object's
//! attestations for `REVISION_HISTORY` assembly.
//!
//! No openEHR spec governs the SQL — our own design. The attestation semantics realized are RM common master06
//! §Attestation; the completed canonical `ATTESTATION` is stored verbatim.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

use serde_json::Value;
use sqlx::{PgConnection, PgPool, Row};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// Insert one `ATTESTATION` row for a version (master06 §Attestation).
///
/// Stores the completed canonical `ATTESTATION` verbatim in `data` (no
/// synthetic fields); `vo_attestation.time_committed` takes the transaction
/// timestamp (`now()`), equal to the `data.time_committed` stamped by the
/// versioning layer with the same commit-act time.
///
/// `at_committal` records whether the attestation was on the version at the act
/// of committal (`true`) or added afterwards (`false`) — the flag that decides
/// whether it sits inside the version's signed canonical form (RM common
/// master06 §Digital Signature vs §Attestation; see the column comment in the
/// baseline migration).
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver/insert failure.
pub async fn insert_attestation(
    tx: &mut PgConnection,
    vo_id: VoId,
    sys_version: i32,
    contribution_id: Uuid,
    at_committal: bool,
    data: &Value,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO vo_attestation (vo_id, sys_version, contribution_id, at_committal, data) \
         VALUES ($1, $2, $3, $4, $5)",
    )
    .bind(vo_id)
    .bind(sys_version)
    .bind(contribution_id)
    .bind(at_committal)
    .bind(data)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The row an attestation attaches to: the addressed `(vo_id, tree, kind)`
/// version's owner, ordinal and `creating_system_id`.
#[derive(Debug, Clone)]
pub struct AttestTargetRow {
    /// The owning EHR, or `None` for a demographic versioned object.
    pub ehr_id: Option<EhrId>,
    /// The addressed version's storage commit ordinal — the attestation's
    /// foreign key.
    pub sys_version: i32,
    /// The addressed version's stored `creating_system_id`.
    pub creating_system_id: String,
    /// Whether the addressed version is an `IMPORTED_VERSION` (a stored
    /// `wrapped_original`) — the `is_original_version(a_ver_id)` half of the
    /// `VERSIONED_OBJECT.commit_attestation` precondition
    /// (`docs/specs/openehr/RM/docs/UML/classes/org.openehr.rm.common.versioned_object.adoc`
    /// §Functions: "Attestations can only be added to Original versions").
    pub imported: bool,
}

/// Look up the target version of a `666|attestation|` item by its tree id and
/// kind text. `None` when no such version exists.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn attestation_target(
    tx: &mut PgConnection,
    vo_id: VoId,
    tree: (i32, i32, i32),
    kind: &str,
) -> Result<Option<AttestTargetRow>, StorageError> {
    // Attesting an archived version appends a row that references it, so the
    // leading CTEs thaw the object back to the hot tier first. The lookup needs
    // no union to see those rows: `version` is partitioned by tier, so naming
    // the parent relation reads both tiers, and the thaw's own effects being
    // invisible to the sibling scans
    // (<https://www.postgresql.org/docs/18/queries-with.html>) does not matter
    // when the pre-statement snapshot already carries them.
    let (t, b, v) = tree;
    let row = sqlx::query(
        "WITH tv AS (UPDATE version SET tier = 'hot' \
                     WHERE vo_id = $1 AND tier = 'cold'), \
         th AS (UPDATE vo_head SET tier = 'hot', archived_at = NULL, archive_reason = NULL \
                WHERE vo_id = $1 AND tier = 'cold') \
         SELECT ehr_id, sys_version, creating_system_id, \
         (wrapped_original IS NOT NULL) AS imported FROM version \
         WHERE vo_id = $1 AND trunk_version = $2 AND branch_number = $3 \
         AND branch_version = $4 AND kind = $5",
    )
    .bind(vo_id)
    .bind(t)
    .bind(b)
    .bind(v)
    .bind(kind)
    .fetch_optional(&mut *tx)
    .await?;
    row.map(|row| -> Result<AttestTargetRow, StorageError> {
        Ok(AttestTargetRow {
            ehr_id: row.try_get("ehr_id")?,
            sys_version: row.try_get("sys_version")?,
            creating_system_id: row.try_get("creating_system_id")?,
            imported: row.try_get("imported")?,
        })
    })
    .transpose()
}

/// All `ATTESTATION`s of an object keyed by version ordinal, in commit order
/// (for `REVISION_HISTORY` assembly).
///
/// Ordered by `time_committed, id`: attestations committed in the same
/// transaction share `now()`, so the `uuidv7()` `id` breaks ties in insertion
/// order.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn read_attestations_all(
    pool: &PgPool,
    vo_id: VoId,
) -> Result<Vec<(i32, Value)>, StorageError> {
    let rows = sqlx::query(
        "SELECT sys_version, data FROM vo_attestation WHERE vo_id = $1 \
         ORDER BY time_committed, id",
    )
    .bind(vo_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| Ok((row.try_get("sys_version")?, row.try_get("data")?)))
        .collect()
}
