// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows of the `EHR_ACCESS` projection: each EHR's settings with its
//! settings class and facts (`access_settings`), and the classes
//! (`access_class`, [`crate::access_eval::class`]).
//!
//! An EHR's row is replaced in the transaction that commits its `EHR_ACCESS`
//! version, and every change to it notifies every replica on [`CHANNEL`] at
//! commit (the trigger of migration 0040); a new class notifies [`NEW_CLASS`]
//! (migration 0042). No openEHR spec governs the projection or its storage
//! SQL: our own design.

use sqlx::{PgConnection, PgPool, Row};

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// The channel a change to an EHR's settings row notifies, with the EHR id
/// as its payload, and a refill with [`EVERY_EHR`].
pub const CHANNEL: &str = "ferroehr_ehr_access";

/// The payload of a notification that drops every EHR's copy.
pub const EVERY_EHR: &str = "*";

/// The payload of a notification that a new settings class is stored.
pub const NEW_CLASS: &str = "class";

/// The `EHR_ACCESS` version an EHR's rows are projected from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Source {
    /// The EHR.
    pub ehr_id: EhrId,
    /// Its `EHR_ACCESS` versioned object.
    pub vo_id: VoId,
    /// The storage ordinal of the version.
    pub sys_version: i32,
}

/// The settings row of one EHR.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsRow {
    /// The access default code.
    pub default_access: &'static str,
    /// The settings in canonical JSON text, their class and their facts;
    /// `None` holds the EHR closed.
    pub settings: Option<Classed>,
}

/// Settings as the projection stores them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classed {
    /// The settings in canonical JSON text.
    pub settings: String,
    /// The SHA-256 of the class's JSON text.
    pub digest: Vec<u8>,
    /// The class in canonical JSON text.
    pub class: String,
    /// The facts the class sets aside, as JSON array text.
    pub facts: String,
}

/// What one EHR's rows become.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rows {
    /// The EHR's `EHR_ACCESS` carries no settings: it has no row.
    None,
    /// The settings row.
    Settings(SettingsRow),
}

/// Replaces the row of `source.ehr_id` with `rows`, on `tx`, storing its
/// class when no EHR made it before.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn replace(
    tx: &mut PgConnection,
    source: Source,
    rows: &Rows,
) -> Result<(), StorageError> {
    let Rows::Settings(row) = rows else {
        sqlx::query("DELETE FROM access_settings WHERE ehr_id = $1")
            .bind(source.ehr_id)
            .execute(&mut *tx)
            .await?;
        return Ok(());
    };
    let class_id = match &row.settings {
        Some(classed) => Some(class_id(tx, classed).await?),
        None => None,
    };
    let classed = row.settings.as_ref();
    sqlx::query(
        "INSERT INTO access_settings \
             (ehr_id, vo_id, sys_version, held_closed, default_access, settings, class_id, facts) \
         VALUES ($1, $2, $3, $4 IS NULL, $5, $4::jsonb, $6, $7::jsonb) \
         ON CONFLICT (ehr_id) DO UPDATE SET vo_id = EXCLUDED.vo_id, \
             sys_version = EXCLUDED.sys_version, held_closed = EXCLUDED.held_closed, \
             default_access = EXCLUDED.default_access, settings = EXCLUDED.settings, \
             class_id = EXCLUDED.class_id, facts = EXCLUDED.facts",
    )
    .bind(source.ehr_id)
    .bind(source.vo_id)
    .bind(source.sys_version)
    .bind(classed.map(|c| c.settings.as_str()))
    .bind(row.default_access)
    .bind(class_id)
    .bind(classed.map(|c| c.facts.as_str()))
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The id of the class of `classed`, stored on `tx` when no EHR made it
/// before. A class a concurrent commit stores first is read once that commit
/// ends: the insert waits for it, and the read after it is a statement of its
/// own (PostgreSQL 18, `INSERT … ON CONFLICT`).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and
/// [`StorageError::InvalidRows`] when no class row can be found after the
/// insert.
async fn class_id(tx: &mut PgConnection, classed: &Classed) -> Result<i64, StorageError> {
    let find = "SELECT id FROM access_class WHERE digest = $1";
    if let Some(id) = sqlx::query_scalar::<_, i64>(find)
        .bind(&classed.digest)
        .fetch_optional(&mut *tx)
        .await?
    {
        return Ok(id);
    }
    let inserted = sqlx::query_scalar::<_, i64>(
        "INSERT INTO access_class (digest, settings) VALUES ($1, $2::jsonb) \
         ON CONFLICT (digest) DO NOTHING RETURNING id",
    )
    .bind(&classed.digest)
    .bind(&classed.class)
    .fetch_optional(&mut *tx)
    .await?;
    if let Some(id) = inserted {
        return Ok(id);
    }
    sqlx::query_scalar::<_, i64>(find)
        .bind(&classed.digest)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| {
            StorageError::InvalidRows("a settings class vanished after its insert".into())
        })
}

/// One EHR's settings row as stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum StoredSettings {
    /// The EHR has no settings row.
    #[default]
    None,
    /// The EHR is held closed.
    HeldClosed,
    /// The settings in canonical JSON text.
    Settings(String),
}

/// One EHR's settings row and subject as stored.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stored {
    /// The settings.
    pub settings: StoredSettings,
    /// The promoted `ehr.subject_id`; `None` for an unknown EHR or a status
    /// naming no subject.
    pub subject: Option<String>,
    /// The uid of the `EHR_ACCESS` version the settings row was projected
    /// from; `None` when the EHR has no settings row.
    pub version: Option<String>,
}

/// One EHR's settings row and its subject, in one statement.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn settings_of(pool: &PgPool, ehr_id: EhrId) -> Result<Stored, StorageError> {
    let row = sqlx::query(
        "SELECT e.subject_id, s.ehr_id IS NOT NULL AS has_settings, s.settings::text AS settings, \
                s.vo_id, v.creating_system_id, v.trunk_version, v.branch_number, v.branch_version \
         FROM (SELECT $1::uuid AS id) k \
         LEFT JOIN ehr e ON e.id = k.id \
         LEFT JOIN access_settings s ON s.ehr_id = k.id \
         LEFT JOIN version v ON v.vo_id = s.vo_id AND v.sys_version = s.sys_version",
    )
    .bind(ehr_id)
    .fetch_one(&mut *acquire(pool).await?)
    .await?;
    Ok(stored(&row)?)
}

/// The settings row and subject one row of [`settings_of`] or
/// [`settings_of_many`] reads.
///
/// # Errors
/// [`sqlx::Error`] when a column does not decode.
fn stored(row: &sqlx::postgres::PgRow) -> Result<Stored, sqlx::Error> {
    let has_settings: bool = row.try_get("has_settings")?;
    let settings = match row.try_get::<Option<String>, _>("settings")? {
        Some(settings) => StoredSettings::Settings(settings),
        None if has_settings => StoredSettings::HeldClosed,
        None => StoredSettings::None,
    };
    let version = version_of(row)?;
    Ok(Stored {
        settings,
        subject: row.try_get("subject_id")?,
        version,
    })
}

/// The uid of the version a row names by its `vo_id`, `creating_system_id`,
/// `trunk_version`, `branch_number` and `branch_version` columns; `None` when
/// the row joined no version.
///
/// # Errors
/// [`sqlx::Error`] when a column does not decode.
pub(crate) fn version_of(row: &sqlx::postgres::PgRow) -> Result<Option<String>, sqlx::Error> {
    match (
        row.try_get::<Option<VoId>, _>("vo_id")?,
        row.try_get::<Option<String>, _>("creating_system_id")?,
    ) {
        (Some(vo_id), Some(system_id)) => Ok(Some(crate::storage::compact::version_uid(
            vo_id,
            &system_id,
            row.try_get("trunk_version")?,
            row.try_get("branch_number")?,
            row.try_get("branch_version")?,
        ))),
        _ => Ok(None),
    }
}

/// The settings rows of `ehr_ids` and their subjects, in one statement: an
/// id with no EHR answers [`Stored::default`].
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn settings_of_many(
    pool: &PgPool,
    ehr_ids: &[EhrId],
) -> Result<Vec<(EhrId, Stored)>, StorageError> {
    let ids: Vec<uuid::Uuid> = ehr_ids.iter().map(|id| id.0).collect();
    let rows = sqlx::query(
        "SELECT k.id AS ehr_id, e.subject_id, s.ehr_id IS NOT NULL AS has_settings, \
                s.settings::text AS settings, s.vo_id, v.creating_system_id, v.trunk_version, \
                v.branch_number, v.branch_version \
         FROM unnest($1::uuid[]) AS k(id) \
         LEFT JOIN ehr e ON e.id = k.id \
         LEFT JOIN access_settings s ON s.ehr_id = k.id \
         LEFT JOIN version v ON v.vo_id = s.vo_id AND v.sys_version = s.sys_version",
    )
    .bind(&ids)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|row| Ok((row.try_get("ehr_id")?, stored(row)?)))
        .collect()
}

/// Every settings class, by id, as its JSON text.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn every_class(pool: &PgPool) -> Result<Vec<(i64, String)>, StorageError> {
    let rows = sqlx::query("SELECT id, settings::text AS settings FROM access_class ORDER BY id")
        .fetch_all(&mut *acquire(pool).await?)
        .await?;
    rows.iter()
        .map(|row| Ok((row.try_get("id")?, row.try_get("settings")?)))
        .collect()
}

/// Takes the lock a refill holds: a commit's write of the rows waits for the
/// refill, and the refill for every commit that has written them.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn lock(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query(
        "LOCK TABLE access_settings, access_class, access_projection_state \
         IN SHARE ROW EXCLUSIVE MODE",
    )
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Whether the rows were filled from the stored versions, read under
/// [`lock`].
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn filled(tx: &mut PgConnection) -> Result<bool, StorageError> {
    Ok(sqlx::query_scalar(
        "SELECT filled_at IS NOT NULL FROM access_projection_state WHERE singleton",
    )
    .fetch_one(&mut *tx)
    .await?)
}

/// Deletes every row and every class, before a refill writes them again.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn clear(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query("DELETE FROM access_settings")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM access_class")
        .execute(&mut *tx)
        .await?;
    Ok(())
}

/// Records that the rows were filled, and notifies every replica to drop its
/// copy of every EHR.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn mark_filled(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query(
        "WITH filled AS (UPDATE access_projection_state SET filled_at = now() WHERE singleton) \
         SELECT pg_notify($1, $2)",
    )
    .bind(CHANNEL)
    .bind(EVERY_EHR)
    .execute(&mut *tx)
    .await?;
    Ok(())
}
