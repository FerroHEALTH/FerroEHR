// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The rows of the controller's `VERSIONED_ACCESS_POLICY`: one append-only row
//! per `ORIGINAL_VERSION` on the trunk, with its contribution and commit audit.
//!
//! A write names the trunk version it replaces and appends the next one under
//! an advisory lock, so two writers that read the same version cannot both
//! win. No openEHR spec governs the SQL, and ITS-REST defines no resource for
//! the access policy: our own design/extension.

use sqlx::{PgConnection, PgPool, Row as _};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::storage::error::StorageError;

/// One stored version of the access policy.
#[derive(Debug, Clone)]
pub struct PolicyRow {
    /// The trunk version, from `1`.
    pub version: i32,
    /// `VERSIONED_ACCESS_POLICY.uid`.
    pub vo_id: Uuid,
    /// The system the version was created on.
    pub system_id: String,
    /// The id of the version's `CONTRIBUTION`.
    pub contribution_id: Uuid,
    /// The canonical `AUDIT_DETAILS`, as JSON text.
    pub commit_audit: String,
    /// `commit_audit.time_committed`.
    pub time_committed: jiff::Timestamp,
    /// The `version_lifecycle_state` code.
    pub lifecycle_state: String,
    /// The canonical `ACCESS_POLICY`, as JSON text.
    pub data: String,
}

/// The version to append: everything but the trunk number, which the append
/// assigns.
#[derive(Debug)]
pub struct NewVersion<'a> {
    /// `VERSIONED_ACCESS_POLICY.uid`, kept from the first version.
    pub vo_id: Uuid,
    /// The system the version is created on.
    pub system_id: &'a str,
    /// The id of the version's `CONTRIBUTION`.
    pub contribution_id: Uuid,
    /// The canonical `AUDIT_DETAILS`.
    pub commit_audit: &'a str,
    /// `commit_audit.time_committed`.
    pub time_committed: jiff::Timestamp,
    /// The `version_lifecycle_state` code.
    pub lifecycle_state: &'a str,
    /// The canonical `ACCESS_POLICY`.
    pub data: &'a str,
}

/// A read of every column of the versions, followed by `$tail`.
macro_rules! select {
    ($tail:literal) => {
        concat!(
            "SELECT version, vo_id, system_id, contribution_id, commit_audit::text AS \
             commit_audit, time_committed, lifecycle_state, data::text AS data \
             FROM access_policy_version ",
            $tail
        )
    };
}

/// The newest trunk version, `0` when the policy has none; one indexed read,
/// the one the request path makes.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn head(pool: &PgPool) -> Result<i32, StorageError> {
    let newest: Option<i32> = sqlx::query_scalar("SELECT max(version) FROM access_policy_version")
        .fetch_one(&mut *acquire(pool).await?)
        .await?;
    Ok(newest.unwrap_or(0))
}

/// Trunk version `version`, or `None` when the policy has no such version.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn at(pool: &PgPool, version: i32) -> Result<Option<PolicyRow>, StorageError> {
    let row = sqlx::query(select!("WHERE version = $1"))
        .bind(version)
        .fetch_optional(&mut *acquire(pool).await?)
        .await?;
    row.as_ref().map(read_row).transpose()
}

/// The newest version, or `None` when the policy has none.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn newest(pool: &PgPool) -> Result<Option<PolicyRow>, StorageError> {
    let row = sqlx::query(select!("ORDER BY version DESC LIMIT 1"))
        .fetch_optional(&mut *acquire(pool).await?)
        .await?;
    row.as_ref().map(read_row).transpose()
}

/// Every version, newest first.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn history(pool: &PgPool) -> Result<Vec<PolicyRow>, StorageError> {
    let rows = sqlx::query(select!("ORDER BY version DESC"))
        .fetch_all(&mut *acquire(pool).await?)
        .await?;
    rows.iter().map(read_row).collect()
}

/// Take the lock every writer of the policy holds until its transaction ends,
/// and read the newest version under it.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn lock_newest(conn: &mut PgConnection) -> Result<Option<PolicyRow>, StorageError> {
    // NOTE: the advisory lock serializes the writers, so the version they replace stays the newest.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtext('access_policy_version'))")
        .execute(&mut *conn)
        .await?;
    let row = sqlx::query(select!("ORDER BY version DESC LIMIT 1"))
        .fetch_optional(&mut *conn)
        .await?;
    row.as_ref().map(read_row).transpose()
}

/// Append `new` as trunk version `version` on `conn`, which holds the lock of
/// [`lock_newest`].
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn append(
    conn: &mut PgConnection,
    version: i32,
    new: &NewVersion<'_>,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO access_policy_version (version, vo_id, system_id, contribution_id, \
         commit_audit, time_committed, lifecycle_state, data) \
         VALUES ($1, $2, $3, $4, $5::jsonb, $6, $7, $8::jsonb)",
    )
    .bind(version)
    .bind(new.vo_id)
    .bind(new.system_id)
    .bind(new.contribution_id)
    .bind(new.commit_audit)
    .bind(jiff_sqlx::Timestamp::from(new.time_committed))
    .bind(new.lifecycle_state)
    .bind(new.data)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// One policy row read off the database.
fn read_row(row: &sqlx::postgres::PgRow) -> Result<PolicyRow, StorageError> {
    let time_committed: jiff_sqlx::Timestamp = row.try_get("time_committed")?;
    Ok(PolicyRow {
        version: row.try_get("version")?,
        vo_id: row.try_get("vo_id")?,
        system_id: row.try_get("system_id")?,
        contribution_id: row.try_get("contribution_id")?,
        commit_audit: row.try_get("commit_audit")?,
        time_committed: time_committed.to_jiff(),
        lifecycle_state: row.try_get("lifecycle_state")?,
        data: row.try_get("data")?,
    })
}
