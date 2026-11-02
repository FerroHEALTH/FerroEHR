// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Row I/O for `archetype_store`, the ADL 1.4 source archetypes keyed by their
//! `ARCHETYPE_ID`.
//!
//! Identity is case-insensitive while storage is case-preserving (BASE
//! `master05-identification_package.adoc` §Composite Identifiers and Case), so
//! every lookup compares `lower(archetype_id) = lower($1)`. No openEHR spec
//! governs the SQL: our own design.

use sqlx::Connection as _;
use sqlx::PgPool;

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// Store the ADL 1.4 source `adl` under `archetype_id`, removing any
/// case-variant of the id in the same transaction, so a re-upload replaces.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn replace_archetype(
    pool: &PgPool,
    archetype_id: &str,
    adl: &str,
) -> Result<(), StorageError> {
    let mut conn = acquire(pool).await?;
    let mut tx = conn.begin().await?;
    sqlx::query("DELETE FROM archetype_store WHERE lower(archetype_id) = lower($1)")
        .bind(archetype_id)
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO archetype_store (archetype_id, adl) VALUES ($1, $2)")
        .bind(archetype_id)
        .bind(adl)
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(())
}

/// The ADL 1.4 source stored under `archetype_id`, or `None`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn archetype_adl(
    pool: &PgPool,
    archetype_id: &str,
) -> Result<Option<String>, StorageError> {
    Ok(
        sqlx::query_scalar("SELECT adl FROM archetype_store WHERE lower(archetype_id) = lower($1)")
            .bind(archetype_id)
            .fetch_optional(&mut *acquire(pool).await?)
            .await?,
    )
}

/// The stored archetype ids in id order, `offset` rows skipped and at most
/// `limit` returned (`None` returns every row).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn archetype_ids(
    pool: &PgPool,
    offset: i64,
    limit: Option<i64>,
) -> Result<Vec<String>, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT archetype_id FROM archetype_store ORDER BY archetype_id OFFSET $1 LIMIT $2",
    )
    .bind(offset)
    .bind(limit)
    .fetch_all(&mut *acquire(pool).await?)
    .await?)
}

/// Delete the archetype stored under `archetype_id`; `false` when none was.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn delete_archetype(pool: &PgPool, archetype_id: &str) -> Result<bool, StorageError> {
    let deleted = sqlx::query("DELETE FROM archetype_store WHERE lower(archetype_id) = lower($1)")
        .bind(archetype_id)
        .execute(&mut *acquire(pool).await?)
        .await?
        .rows_affected();
    Ok(deleted > 0)
}
