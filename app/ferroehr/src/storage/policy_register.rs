// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operational-policy admin registers storage keeps: one append-only row
//! per version of each register, with its committer.
//!
//! A register with no row holds its default, which is version `0`. A write
//! names the version it replaces and appends the next one, so two writers that
//! read the same version cannot both win: the primary key refuses the second.
//! No openEHR spec governs operational policy: our own design/extension.

use sqlx::{PgConnection, PgPool, Row as _};

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// One stored version of a register.
#[derive(Debug, Clone)]
pub struct RegisterRow {
    /// The version, from `1`.
    pub version: i64,
    /// The register document, as JSON text.
    pub document: String,
    /// Who committed the version.
    pub committer: String,
    /// When it was committed.
    pub committed_at: jiff::Timestamp,
}

/// The newest version of every register that has one, as `(register,
/// version)`; one indexed read, the one the request path makes.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn heads(pool: &PgPool) -> Result<Vec<(String, i64)>, StorageError> {
    let rows = sqlx::query(
        "SELECT register, max(version) AS version FROM policy_register GROUP BY register",
    )
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|row| Ok((row.try_get("register")?, row.try_get("version")?)))
        .collect()
}

/// The stored version `version` of `register`, or `None` when it has none.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn at(
    pool: &PgPool,
    register: &str,
    version: i64,
) -> Result<Option<RegisterRow>, StorageError> {
    let row = sqlx::query(
        "SELECT version, document::text AS document, committer, committed_at \
         FROM policy_register WHERE register = $1 AND version = $2",
    )
    .bind(register)
    .bind(version)
    .fetch_optional(&mut *acquire(pool).await?)
    .await?;
    row.as_ref().map(read_row).transpose()
}

/// The newest stored version of `register`, or `None` when it has none.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn newest(pool: &PgPool, register: &str) -> Result<Option<RegisterRow>, StorageError> {
    let row = sqlx::query(
        "SELECT version, document::text AS document, committer, committed_at \
         FROM policy_register WHERE register = $1 ORDER BY version DESC LIMIT 1",
    )
    .bind(register)
    .fetch_optional(&mut *acquire(pool).await?)
    .await?;
    row.as_ref().map(read_row).transpose()
}

/// Every stored version of `register`, newest first.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn history(pool: &PgPool, register: &str) -> Result<Vec<RegisterRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT version, document::text AS document, committer, committed_at \
         FROM policy_register WHERE register = $1 ORDER BY version DESC",
    )
    .bind(register)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter().map(read_row).collect()
}

/// Append version `replaces + 1` of `register` on `conn`, when `replaces` is
/// still the newest version; `None` when it is not.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn append(
    conn: &mut PgConnection,
    register: &str,
    replaces: i64,
    document: &str,
    committer: &str,
) -> Result<Option<i64>, StorageError> {
    // NOTE: the advisory lock serializes writers of one register, so the version check holds.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('policy_register:' || $1))")
        .bind(register)
        .execute(&mut *conn)
        .await?;
    let newest: Option<i64> =
        sqlx::query_scalar("SELECT max(version) FROM policy_register WHERE register = $1")
            .bind(register)
            .fetch_one(&mut *conn)
            .await?;
    if newest.unwrap_or(0) != replaces {
        return Ok(None);
    }
    let Some(version) = replaces.checked_add(1) else {
        return Ok(None);
    };
    sqlx::query(
        "INSERT INTO policy_register (register, version, document, committer) \
         VALUES ($1, $2, $3::jsonb, $4)",
    )
    .bind(register)
    .bind(version)
    .bind(document)
    .bind(committer)
    .execute(&mut *conn)
    .await?;
    Ok(Some(version))
}

/// One register row read off the database.
fn read_row(row: &sqlx::postgres::PgRow) -> Result<RegisterRow, StorageError> {
    let committed_at: jiff_sqlx::Timestamp = row.try_get("committed_at")?;
    Ok(RegisterRow {
        version: row.try_get("version")?,
        document: row.try_get("document")?,
        committer: row.try_get("committer")?,
        committed_at: committed_at.to_jiff(),
    })
}
