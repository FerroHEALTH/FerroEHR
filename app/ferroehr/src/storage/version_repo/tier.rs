// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The cold archival storage tier: the move and its reverse.
//!
//! No openEHR spec governs storage tiering — our own design/extension. It
//! moves selected EHRs and parties to archival storage and back.
//!
//! The tier is a PARTITION, not a second set of relations: `version`, `node`
//! and `vo_attestation` are each `PARTITION BY LIST (tier)` with a `hot` and a
//! `cold` partition. Archiving is therefore one `UPDATE` of the partition key,
//! which PostgreSQL performs as a move between partitions (PostgreSQL 18,
//! "Partitioning", <https://www.postgresql.org/docs/18/ddl-partitioning.html>),
//! and the `node` and `vo_attestation` foreign keys carry their rows across
//! with `ON UPDATE CASCADE`. Two consequences the mirror-table design could not
//! have: every foreign key holds across the tier, and every read reaches an
//! archived object by naming the parent relation, with no union view and no
//! second statement.
//!
//! A write still thaws first (the admin restore path calls [`thaw`]; the commit
//! path's thaw rides the merged placement read,
//! `crate::storage::version_repo::placement::next_placement`), so a versioned
//! object is never split across tiers.

use sqlx::PgConnection;

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// Moves every version of `vo_ids` to the cold tier, and records the move on
/// the head row.
///
/// Runs inside the caller's transaction, so the rows and the head marker move
/// together. Objects already cold match nothing and are silently skipped, so
/// re-archiving is idempotent.
///
/// The node and attestation rows are NOT named: their foreign keys into
/// `version` carry them across with the version row.
///
/// # Errors
/// Returns [`StorageError::Database`] on any driver/statement failure.
pub async fn freeze(
    tx: &mut PgConnection,
    vo_ids: &[VoId],
    reason: Option<&str>,
) -> Result<(), StorageError> {
    if vo_ids.is_empty() {
        return Ok(());
    }
    sqlx::query("UPDATE version SET tier = 'cold' WHERE vo_id = ANY($1) AND tier = 'hot'")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE vo_head SET tier = 'cold', archived_at = now(), archive_reason = $2 \
         WHERE vo_id = ANY($1) AND tier = 'hot'",
    )
    .bind(vo_ids)
    .bind(reason)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Moves every version of `vo_ids` back to the hot tier and clears the archive
/// marker — the exact reverse of [`freeze`].
///
/// # Errors
/// Returns [`StorageError::Database`] on any driver/statement failure.
pub async fn thaw(tx: &mut PgConnection, vo_ids: &[VoId]) -> Result<(), StorageError> {
    if vo_ids.is_empty() {
        return Ok(());
    }
    sqlx::query("UPDATE version SET tier = 'hot' WHERE vo_id = ANY($1) AND tier = 'cold'")
        .bind(vo_ids)
        .execute(&mut *tx)
        .await?;
    sqlx::query(
        "UPDATE vo_head SET tier = 'hot', archived_at = NULL, archive_reason = NULL \
         WHERE vo_id = ANY($1) AND tier = 'cold'",
    )
    .bind(vo_ids)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Whether each of `vo_ids` is currently archived, as a parallel answer set.
///
/// One primary-key probe per object on the head row, which is also where the
/// archive marker lives.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn archived(tx: &mut PgConnection, vo_ids: &[VoId]) -> Result<Vec<VoId>, StorageError> {
    if vo_ids.is_empty() {
        return Ok(Vec::new());
    }
    let rows: Vec<uuid::Uuid> =
        sqlx::query_scalar("SELECT vo_id FROM vo_head WHERE vo_id = ANY($1) AND tier = 'cold'")
            .bind(vo_ids)
            .fetch_all(&mut *tx)
            .await?;
    Ok(rows.into_iter().map(VoId).collect())
}

/// The kind text of a demographic (ehr-less) versioned object, read from any
/// of its versions in either tier, or `None` when no such object exists.
///
/// The archive, the restore and the physical erasure all address a party
/// wherever it lives, so the three answer one way whichever tier holds it.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn demographic_kind(
    conn: &mut PgConnection,
    vo_id: VoId,
) -> Result<Option<String>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT kind FROM version WHERE vo_id = $1 AND ehr_id IS NULL LIMIT 1")
            .bind(vo_id)
            .fetch_optional(conn)
            .await?,
    )
}

/// The versioned objects of `ehr_id` that are in the hot tier, the ones an
/// archive of the EHR moves; an object already cold is not among them, which
/// is what makes archiving twice a no-op.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn hot_objects(
    conn: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<Vec<VoId>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT vo_id FROM vo_head WHERE ehr_id = $1 AND tier = 'hot'")
            .bind(ehr_id)
            .fetch_all(conn)
            .await?,
    )
}

/// The versioned objects of `ehr_id` that are in the cold tier, the ones a
/// restore of the EHR moves back.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn cold_objects(
    conn: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<Vec<VoId>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT vo_id FROM vo_head WHERE ehr_id = $1 AND tier = 'cold'")
            .bind(ehr_id)
            .fetch_all(conn)
            .await?,
    )
}

/// Lock the head row of `vo_id` when the object is in the cold tier and
/// return its archive reason (`Some(None)` for an archive without one), or
/// `None` when the object is hot.
///
/// A rebuild thaws an archived object first, since a write never splits an
/// object across tiers; the marker read here is re-stamped when it freezes the
/// object again.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn lock_cold_marker(
    tx: &mut PgConnection,
    vo_id: VoId,
) -> Result<Option<Option<String>>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT archive_reason FROM vo_head WHERE vo_id = $1 AND tier = 'cold' FOR UPDATE",
    )
    .bind(vo_id)
    .fetch_optional(tx)
    .await?)
}
