// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows of the `EHR_ACCESS` projection: each EHR's settings, its
//! directives and its labels (`access_settings`, `access_directive`,
//! `access_label`).
//!
//! An EHR's rows are replaced whole in the transaction that commits its
//! `EHR_ACCESS` version, and every change to its settings row notifies every
//! replica on [`CHANNEL`] at commit (the trigger of migration 0040). No
//! openEHR spec governs the projection or its storage SQL: our own design.

use serde::Serialize;
use sqlx::{PgConnection, PgPool, Row};

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// The channel a change to an EHR's settings row notifies, with the EHR id
/// as its payload, and a refill with [`EVERY_EHR`].
pub const CHANNEL: &str = "ferroehr_ehr_access";

/// The payload of a notification that drops every EHR's copy.
pub const EVERY_EHR: &str = "*";

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
    /// The settings in canonical JSON text; `None` holds the EHR closed.
    pub settings: Option<String>,
}

/// One `access_directive` row, as the write binds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DirectiveRow {
    /// The position in a depth-first walk of the settings.
    pub ordinal: i32,
    /// The ordinal of the directive this one is an exception of.
    pub parent: Option<i32>,
    /// The access directive kind code.
    pub kind: &'static str,
    /// The access effect code.
    pub effect: &'static str,
    /// Whether an emergency lifts the directive.
    pub emergency_override: bool,
    /// The party id values of the actor scopes.
    pub parties: Vec<String>,
    /// The roles of the actor scopes, as `terminology::code`.
    pub roles: Vec<String>,
    /// The access relationship codes.
    pub relationships: Vec<&'static str>,
    /// The access caller kind codes.
    pub caller_kinds: Vec<&'static str>,
    /// The access operation codes.
    pub operations: Vec<&'static str>,
    /// The purposes, as `terminology::code`.
    pub purposes: Vec<String>,
    /// The categories of the data scopes, as `terminology::code`.
    pub categories: Vec<String>,
    /// The sensitivities of the data scopes, as `terminology::code`.
    pub sensitivities: Vec<String>,
    /// The `tstzrange` literal of the validity; `None` when unreadable.
    pub validity: Option<String>,
}

/// One `access_label` row, as the write binds it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct LabelRow {
    /// The position in `ACCESS_DIRECTIVE_SETTINGS.labels`.
    pub ordinal: i32,
    /// The labelled object's `OBJECT_REF` namespace, type and id value.
    pub target_namespace: String,
    /// The `OBJECT_REF.type`.
    pub target_type: String,
    /// The `OBJECT_REF.id` value.
    pub target_id: String,
    /// The sensitivities, as `terminology::code`.
    pub sensitivities: Vec<String>,
}

/// What one EHR's rows become.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Rows {
    /// The EHR's `EHR_ACCESS` carries no settings: it has no row.
    None,
    /// The settings row and the rows of its directives and labels.
    Settings {
        /// The settings row.
        settings: SettingsRow,
        /// The directive rows.
        directives: Vec<DirectiveRow>,
        /// The label rows.
        labels: Vec<LabelRow>,
    },
}

/// Replaces the rows of `source.ehr_id` with `rows`, on `tx`, in one
/// statement.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn replace(
    tx: &mut PgConnection,
    source: Source,
    rows: &Rows,
) -> Result<(), StorageError> {
    let Rows::Settings {
        settings,
        directives,
        labels,
    } = rows
    else {
        sqlx::query(
            "WITH directives AS (DELETE FROM access_directive WHERE ehr_id = $1), \
                  labels AS (DELETE FROM access_label WHERE ehr_id = $1) \
             DELETE FROM access_settings WHERE ehr_id = $1",
        )
        .bind(source.ehr_id)
        .execute(&mut *tx)
        .await?;
        return Ok(());
    };
    sqlx::query(
        "WITH header AS ( \
             INSERT INTO access_settings \
                 (ehr_id, vo_id, sys_version, held_closed, default_access, settings) \
             VALUES ($1, $2, $3, $4 IS NULL, $5, $4::jsonb) \
             ON CONFLICT (ehr_id) DO UPDATE SET vo_id = EXCLUDED.vo_id, \
                 sys_version = EXCLUDED.sys_version, held_closed = EXCLUDED.held_closed, \
                 default_access = EXCLUDED.default_access, settings = EXCLUDED.settings), \
         gone_directives AS (DELETE FROM access_directive WHERE ehr_id = $1), \
         gone_labels AS (DELETE FROM access_label WHERE ehr_id = $1), \
         directives AS ( \
             INSERT INTO access_directive \
                 (ehr_id, vo_id, ordinal, parent, kind, effect, emergency_override, parties, \
                  roles, relationships, caller_kinds, operations, purposes, categories, \
                  sensitivities, validity) \
             SELECT $1, $2, t.ordinal, t.parent, t.kind, t.effect, t.emergency_override, \
                    t.parties, t.roles, t.relationships, t.caller_kinds, t.operations, \
                    t.purposes, t.categories, t.sensitivities, t.validity::tstzrange \
             FROM jsonb_to_recordset($6::jsonb) AS t( \
                 ordinal integer, parent integer, kind text, effect text, \
                 emergency_override boolean, parties text[], roles text[], \
                 relationships text[], caller_kinds text[], operations text[], \
                 purposes text[], categories text[], sensitivities text[], validity text)) \
         INSERT INTO access_label \
             (ehr_id, vo_id, ordinal, target_namespace, target_type, target_id, sensitivities) \
         SELECT $1, $2, t.ordinal, t.target_namespace, t.target_type, t.target_id, \
                t.sensitivities \
         FROM jsonb_to_recordset($7::jsonb) AS t( \
             ordinal integer, target_namespace text, target_type text, target_id text, \
             sensitivities text[])",
    )
    .bind(source.ehr_id)
    .bind(source.vo_id)
    .bind(source.sys_version)
    .bind(settings.settings.as_deref())
    .bind(settings.default_access)
    .bind(sqlx::types::Json(directives))
    .bind(sqlx::types::Json(labels))
    .execute(&mut *tx)
    .await?;
    Ok(())
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
}

/// One EHR's settings row and its subject, in one statement.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn settings_of(pool: &PgPool, ehr_id: EhrId) -> Result<Stored, StorageError> {
    let row = sqlx::query(
        "SELECT e.subject_id, s.ehr_id IS NOT NULL AS has_settings, s.settings::text AS settings \
         FROM (SELECT $1::uuid AS id) k \
         LEFT JOIN ehr e ON e.id = k.id \
         LEFT JOIN access_settings s ON s.ehr_id = k.id",
    )
    .bind(ehr_id)
    .fetch_one(&mut *acquire(pool).await?)
    .await?;
    let has_settings: bool = row.try_get("has_settings")?;
    let settings = match row.try_get::<Option<String>, _>("settings")? {
        Some(settings) => StoredSettings::Settings(settings),
        None if has_settings => StoredSettings::HeldClosed,
        None => StoredSettings::None,
    };
    Ok(Stored {
        settings,
        subject: row.try_get("subject_id")?,
    })
}

/// Every EHR's settings row, as `(ehr_id, settings)`, a held-closed EHR's
/// settings `None`, ordered by EHR.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and the acquire failure
/// when no connection can be had.
pub async fn every_settings(pool: &PgPool) -> Result<Vec<(EhrId, Option<String>)>, StorageError> {
    let rows = sqlx::query(
        "SELECT ehr_id, settings::text AS settings FROM access_settings ORDER BY ehr_id",
    )
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|row| Ok((row.try_get("ehr_id")?, row.try_get("settings")?)))
        .collect()
}

/// Takes the lock a refill holds: a commit's write of the rows waits for the
/// refill, and the refill for every commit that has written them.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn lock(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query(
        "LOCK TABLE access_settings, access_directive, access_label, access_projection_state \
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

/// Deletes every row, before a refill writes them again.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn clear(tx: &mut PgConnection) -> Result<(), StorageError> {
    sqlx::query(
        "WITH directives AS (DELETE FROM access_directive), labels AS (DELETE FROM access_label) \
         DELETE FROM access_settings",
    )
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
