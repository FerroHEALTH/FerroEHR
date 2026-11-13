// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads of the `EHR_ACCESS` objects the access rules decide by.
//!
//! Every EHR's current settings body, one EHR's on a commit's own
//! transaction, and the EHRs that hold none. No openEHR spec governs the
//! storage SQL: our own design.

use sqlx::{PgPool, Row};

use crate::db::acquire::{acquire, lease};
use crate::ids::EhrId;
use crate::storage::error::StorageError;

/// Every EHR's current `EHR_ACCESS` body that names `settings`, as
/// `(ehr_id, canonical JSON text)`.
///
/// The text match only prefilters: the caller parses each body and keeps the
/// settings it understands; a logically deleted version has no body. It runs
/// on a [`lease`], as every waiting query restarts an abandoned load.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn ehr_access_bodies_with_settings(
    pool: &PgPool,
) -> Result<Vec<(EhrId, String)>, StorageError> {
    const SQL: &str = "SELECT version.ehr_id, version.body \
                       FROM version \
                       JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
                       WHERE version.kind = 'EHR_ACCESS' AND strpos(version.body, '\"settings\"') > 0";
    let mut leased = lease(pool).await?;
    let rows = leased.run(|conn| sqlx::query(SQL).fetch_all(conn)).await?;
    rows.iter()
        .map(|row| Ok((row.try_get("ehr_id")?, row.try_get("body")?)))
        .collect()
}

/// Every EHR that holds no `EHR_ACCESS` versioned object.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn ehrs_without_ehr_access(pool: &PgPool) -> Result<Vec<EhrId>, StorageError> {
    const SQL: &str = "SELECT e.id FROM ehr e WHERE NOT EXISTS \
                       (SELECT 1 FROM vo_head h WHERE h.ehr_id = e.id AND h.kind = 'EHR_ACCESS') \
                       ORDER BY e.id";
    Ok(sqlx::query_scalar(SQL)
        .fetch_all(&mut *acquire(pool).await?)
        .await?)
}

/// The current `EHR_ACCESS` body of `ehr_id` as canonical JSON text, read on
/// the commit's own transaction; `None` when the EHR has none or its current
/// version is a logical deletion, which has no body.
///
/// # Errors
/// Returns [`StorageError::Database`] on a driver failure.
pub async fn current_ehr_access_body(
    conn: &mut sqlx::PgConnection,
    ehr_id: EhrId,
) -> Result<Option<String>, StorageError> {
    const SQL: &str = "SELECT version.body \
                       FROM vo_head h \
                       JOIN version ON version.vo_id = h.vo_id AND version.sys_version = h.trunk_head_sys_version \
                       WHERE h.ehr_id = $1 AND h.kind = 'EHR_ACCESS'";
    let row: Option<Option<String>> = sqlx::query_scalar(SQL)
        .bind(ehr_id)
        .fetch_optional(conn)
        .await?;
    Ok(row.flatten())
}
