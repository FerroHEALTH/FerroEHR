// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The EHDS Art. 10 opt-out register and its mark.
//!
//! `access_opt_out` records when the person's opt-out from access through the
//! access services of Art. 4 and Art. 12 was recorded and when it was
//! reversed, always for the whole EHR, and `ehr.opted_out_at` is the mark the
//! read paths filter on, recomputed from the register by [`refresh_mark`].
//! A reversal writes the row rather than erasing it, since Art. 10(1) has the
//! exercise of the right be reversible and the sequence is what a supervisory
//! authority asks about (`docs/law/eu/ehds/text.html`). No openEHR spec governs
//! an opt-out: our own design/extension.

use sqlx::{PgConnection, PgPool, Row as _};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::EhrId;
use crate::storage::error::StorageError;

/// One row of the opt-out register.
#[derive(Debug, Clone, serde::Serialize)]
pub struct OptOutRow {
    /// The register row id.
    pub id: Uuid,
    /// The EHR the opt-out was recorded for.
    pub ehr_id: EhrId,
    /// When the person's opt-out was recorded.
    pub opted_out_at: jiff::Timestamp,
    /// When it was reversed, or `None` while it stands.
    pub reversed_at: Option<jiff::Timestamp>,
    /// The controller's free-text note.
    pub note: Option<String>,
}

/// Append an opt-out to the register and refresh the mark.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, including the foreign key
/// refusing an unknown EHR.
pub async fn record(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    note: Option<&str>,
) -> Result<Uuid, StorageError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO access_opt_out (ehr_id, note) VALUES ($1, $2) RETURNING id",
    )
    .bind(ehr_id)
    .bind(note)
    .fetch_one(&mut *tx)
    .await?;
    refresh_mark(tx, ehr_id).await?;
    Ok(id)
}

/// Reverse every opt-out of `ehr_id` that stands and refresh the mark;
/// returns how many register rows were reversed.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn reverse(tx: &mut PgConnection, ehr_id: EhrId) -> Result<u64, StorageError> {
    let reversed = sqlx::query(
        "UPDATE access_opt_out SET reversed_at = now() \
         WHERE ehr_id = $1 AND reversed_at IS NULL",
    )
    .bind(ehr_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    refresh_mark(tx, ehr_id).await?;
    Ok(reversed)
}

/// Recompute `ehr.opted_out_at` from the register: the earliest opt-out that
/// stands, or `NULL`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn refresh_mark(tx: &mut PgConnection, ehr_id: EhrId) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE ehr SET opted_out_at = (\
             SELECT min(o.opted_out_at) FROM access_opt_out o \
              WHERE o.ehr_id = $1 AND o.reversed_at IS NULL) \
         WHERE id = $1",
    )
    .bind(ehr_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The opt-out register rows of one EHR, newest first.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn register(pool: &PgPool, ehr_id: EhrId) -> Result<Vec<OptOutRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, ehr_id, opted_out_at, reversed_at, note FROM access_opt_out \
         WHERE ehr_id = $1 ORDER BY opted_out_at DESC, id DESC",
    )
    .bind(ehr_id)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(OptOutRow {
                id: row.try_get("id")?,
                ehr_id: row.try_get("ehr_id")?,
                opted_out_at: row
                    .try_get::<jiff_sqlx::Timestamp, _>("opted_out_at")?
                    .to_jiff(),
                reversed_at: row
                    .try_get::<Option<jiff_sqlx::Timestamp>, _>("reversed_at")?
                    .map(jiff_sqlx::Timestamp::to_jiff),
                note: row.try_get("note")?,
            })
        })
        .collect()
}

/// Whether the EHR carries the opt-out mark; `false` for an unknown EHR.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn is_opted_out(pool: &PgPool, ehr_id: EhrId) -> Result<bool, StorageError> {
    let opted_out: Option<bool> =
        sqlx::query_scalar("SELECT opted_out_at IS NOT NULL FROM ehr WHERE id = $1")
            .bind(ehr_id)
            .fetch_optional(&mut *acquire(pool).await?)
            .await?;
    Ok(opted_out.unwrap_or(false))
}

/// Whether the EHR reads as absent to the request: under a whole-EHR EHDS
/// Art. 8 access restriction while `restricted` holds, or opted out under
/// Art. 10 while `opted_out` holds; `false` for an unknown EHR.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn ehr_hidden(
    conn: &mut PgConnection,
    ehr_id: EhrId,
    restricted: bool,
    opted_out: bool,
) -> Result<bool, StorageError> {
    let hidden: Option<bool> = sqlx::query_scalar(
        "SELECT ($2 AND access_restricted_at IS NOT NULL) OR ($3 AND opted_out_at IS NOT NULL) \
         FROM ehr WHERE id = $1",
    )
    .bind(ehr_id)
    .bind(restricted)
    .bind(opted_out)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(hidden.unwrap_or(false))
}
