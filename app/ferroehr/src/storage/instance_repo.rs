// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows of the deployment's running instances (`instance`, migration
//! `clinical/0044`): what each must share with the others, and when it was
//! last seen ([`crate::instance`]). No openEHR spec governs the process
//! model or its storage SQL: our own design (#5589).

use sqlx::{PgPool, Row};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// One instance's row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstanceRow {
    /// The instance.
    pub instance_id: Uuid,
    /// When it started.
    pub started_at: jiff::Timestamp,
    /// The FerroEHR release it runs.
    pub version: String,
    /// The fingerprint of its signing key, or `digest`.
    pub signing_key: String,
    /// The digest of its shared configuration.
    pub configuration: String,
}

/// Writes or touches the row of `own`, deletes every row no instance touched
/// for a day, and reads every other row touched within `live_secs`, in one
/// statement, oldest instance first.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn touch(
    pool: &PgPool,
    own: &InstanceRow,
    live_secs: i64,
) -> Result<Vec<InstanceRow>, StorageError> {
    let rows = sqlx::query(
        "WITH own AS ( \
             INSERT INTO instance (instance_id, version, signing_key, configuration) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (instance_id) DO UPDATE SET seen_at = now(), \
                 version = EXCLUDED.version, signing_key = EXCLUDED.signing_key, \
                 configuration = EXCLUDED.configuration \
             RETURNING instance_id \
         ), gone AS ( \
             DELETE FROM instance WHERE seen_at < now() - interval '1 day' \
         ) \
         SELECT instance_id, started_at, version, signing_key, configuration FROM instance \
         WHERE instance_id <> $1 AND seen_at >= now() - make_interval(secs => $5) \
         ORDER BY started_at, instance_id",
    )
    .bind(own.instance_id)
    .bind(&own.version)
    .bind(&own.signing_key)
    .bind(&own.configuration)
    .bind(live_secs)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(InstanceRow {
                instance_id: row.try_get("instance_id")?,
                started_at: row
                    .try_get::<jiff_sqlx::Timestamp, _>("started_at")?
                    .to_jiff(),
                version: row.try_get("version")?,
                signing_key: row.try_get("signing_key")?,
                configuration: row.try_get("configuration")?,
            })
        })
        .collect()
}

/// Deletes the row of `instance_id`: the instance stops.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn leave(pool: &PgPool, instance_id: Uuid) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM instance WHERE instance_id = $1")
        .bind(instance_id)
        .execute(&mut *acquire(pool).await?)
        .await?;
    Ok(())
}
