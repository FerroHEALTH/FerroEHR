// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The reads behind an EHR Extract: which version containers of an EHR it may
//! carry, their kinds, and the stored versions of each.
//!
//! A restricted object is never carried (GDPR Art. 18(2),
//! `docs/law/eu/gdpr/text.html`); an object under an EHDS Art. 8 restriction,
//! or withheld from the person under Art. 3(3), is left out while the caller
//! enforces it (`docs/law/eu/ehds/text.html`). No openEHR spec governs the
//! SQL: our own design.

use sqlx::{PgPool, Row as _};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::access_facts::HeadFacts;
use crate::storage::error::StorageError;

/// Which access marks a read honours besides the restriction register, which
/// it always honours, and the access evaluator's decision, which it always
/// applies ([`crate::storage::access_facts`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Enforced {
    /// The EHDS Art. 3(3) delay of the person's own access
    /// (`vo_head.person_withheld_until`).
    pub access_delay: bool,
}

/// Every version container of `ehr_id` the extract may carry, with its kind,
/// in id order.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn ehr_objects(
    pool: &PgPool,
    ehr_id: EhrId,
    enforced: Enforced,
) -> Result<Vec<(VoId, String)>, StorageError> {
    let rows = sqlx::query(concat!(
        "SELECT version.vo_id, version.kind, ",
        head_facts!(),
        " FROM version \
         JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
         WHERE version.ehr_id = $1 AND h.restricted_at IS NULL \
         AND (NOT $2 OR (h.person_withheld_until > now()) IS NOT TRUE) ORDER BY version.vo_id"
    ))
    .bind(ehr_id)
    .bind(enforced.access_delay)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    let mut objects = Vec::with_capacity(rows.len());
    for row in &rows {
        let vo_id: VoId = row.try_get("vo_id")?;
        if HeadFacts::read(row, vo_id)?.admitted() {
            objects.push((vo_id, row.try_get("kind")?));
        }
    }
    Ok(objects)
}

/// The kind of the version container `vo_id` of `ehr_id`, or `None` when the
/// extract may not carry it.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn ehr_object_kind(
    pool: &PgPool,
    ehr_id: EhrId,
    vo_id: VoId,
    enforced: Enforced,
) -> Result<Option<String>, StorageError> {
    let row = sqlx::query(concat!(
        "SELECT version.kind, ",
        head_facts!(),
        " FROM version \
         JOIN vo_head h ON h.vo_id = version.vo_id AND h.trunk_head_sys_version = version.sys_version \
         WHERE version.vo_id = $1 AND version.ehr_id = $2 AND h.restricted_at IS NULL \
         AND (NOT $3 OR (h.person_withheld_until > now()) IS NOT TRUE)"
    ))
    .bind(vo_id)
    .bind(ehr_id)
    .bind(enforced.access_delay)
    .fetch_optional(&mut *acquire(pool).await?)
    .await?;
    match row {
        Some(row) if HeadFacts::read(&row, vo_id)?.admitted() => Ok(Some(row.try_get("kind")?)),
        _ => Ok(None),
    }
}

/// The `sys_version`s of a version container, in order, each flagged with
/// whether it is the current trunk version (`vo_head.trunk_head_sys_version`).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn version_numbers(pool: &PgPool, vo_id: VoId) -> Result<Vec<(i32, bool)>, StorageError> {
    let rows = sqlx::query(
        "SELECT sys_version, EXISTS(SELECT 1 FROM vo_head h \
             WHERE h.vo_id = version.vo_id \
               AND h.trunk_head_sys_version = version.sys_version) AS is_current \
         FROM version WHERE vo_id = $1 ORDER BY sys_version",
    )
    .bind(vo_id)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|r| Ok((r.try_get("sys_version")?, r.try_get("is_current")?)))
        .collect()
}

/// The stored-version count of each of `vo_ids` that has a version, in one
/// statement.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn version_counts(
    pool: &PgPool,
    vo_ids: &[VoId],
) -> Result<Vec<(VoId, i64)>, StorageError> {
    if vo_ids.is_empty() {
        return Ok(Vec::new());
    }
    let ids: Vec<Uuid> = vo_ids.iter().map(|v| v.0).collect();
    let rows = sqlx::query(
        "SELECT vo_id, count(*) AS n FROM version WHERE vo_id = ANY($1) GROUP BY vo_id",
    )
    .bind(&ids)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|r| Ok((r.try_get("vo_id")?, r.try_get("n")?)))
        .collect()
}
