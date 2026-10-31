// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The register and mark of the EHDS Art. 3(3) delay of the person's own
//! access.
//!
//! Regulation (EU) 2025/327 Art. 3(3) lets a Member State delay a person's
//! access to their data "for a limited period of time until a health
//! professional is able to properly communicate and explain" it
//! (`docs/law/eu/ehds/text.html`). `access_delay` holds each hold on one
//! versioned object, and `vo_head.person_withheld_until` is the mark the
//! patient-context read paths filter on: the latest end of the holds not
//! released. Every comparison with the clock runs in the database, so the
//! mark and the reads agree on one `now()`. No openEHR spec governs a delay of
//! the person's access: our own design/extension.

use sqlx::{PgConnection, PgPool, Row as _};
use uuid::Uuid;

use crate::db::acquire::acquire;
use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// The deployment's configured category held the object at its creation.
pub const BASIS_CATEGORY: &str = "category";
/// A health professional placed the hold.
pub const BASIS_PROFESSIONAL: &str = "professional";

/// One row of the access-delay register, serialized as the admin register
/// route answers it.
#[derive(Debug, Clone, serde::Serialize)]
pub struct AccessDelayRow {
    /// The register row id.
    pub id: Uuid,
    /// The EHR the held object belongs to.
    pub ehr_id: EhrId,
    /// The held versioned object.
    pub vo_id: VoId,
    /// [`BASIS_CATEGORY`] or [`BASIS_PROFESSIONAL`].
    pub basis: String,
    /// When the hold began.
    pub withheld_at: jiff::Timestamp,
    /// When the hold ends by itself.
    pub withheld_until: jiff::Timestamp,
    /// When a professional released it, or `None`.
    pub released_at: Option<jiff::Timestamp>,
    /// The professional's free-text note.
    pub note: Option<String>,
}

/// One hold whose end passed without a release.
#[derive(Debug, Clone, Copy)]
pub struct ExpiredHold {
    /// The EHR the object belongs to.
    pub ehr_id: EhrId,
    /// The object the hold no longer withholds.
    pub vo_id: VoId,
}

/// Hold a newly created COMPOSITION for `hours` when the category map places
/// it in one of `categories`; returns whether it was held.
///
/// The object's categories are read as the retention register reads them:
/// the template's mapping first, the root archetype's when the template has
/// none (`category_map`, the boot mirror of `[audit.categories]`).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn hold_new_composition(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    vo_id: VoId,
    content: (Option<&str>, Option<&str>),
    categories: &[String],
    hours: i32,
) -> Result<bool, StorageError> {
    let (template_id, root_archetype) = content;
    let held: Option<Uuid> = sqlx::query_scalar(
        "WITH c AS (SELECT coalesce(\
             (SELECT array_agg(m.category) FROM category_map m \
               WHERE m.key_kind = 'template' AND m.key = lower($3::text COLLATE \"C\")), \
             (SELECT array_agg(m.category) FROM category_map m \
               WHERE m.key_kind = 'archetype' AND m.key = lower($4::text COLLATE \"C\"))) AS cats), \
         held AS (INSERT INTO access_delay (ehr_id, vo_id, basis, withheld_until) \
             SELECT $1, $2, 'category', now() + make_interval(hours => $6) FROM c \
              WHERE c.cats && $5::text[] RETURNING vo_id, withheld_until) \
         UPDATE vo_head h SET person_withheld_until = held.withheld_until FROM held \
          WHERE h.vo_id = held.vo_id RETURNING h.vo_id",
    )
    .bind(ehr_id)
    .bind(vo_id)
    .bind(template_id)
    .bind(root_archetype)
    .bind(categories)
    .bind(hours)
    .fetch_optional(&mut *tx)
    .await?;
    Ok(held.is_some())
}

/// Hold one COMPOSITION on a professional's act and refresh its mark.
///
/// The hold ends at `until`, or `hours` from now when no end is given. `None`
/// when nothing was held: the object is no COMPOSITION, or `until` is not
/// after now and within `hours` of now.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn hold(
    tx: &mut PgConnection,
    target: (EhrId, VoId),
    until: Option<jiff::Timestamp>,
    hours: i32,
    note: Option<&str>,
) -> Result<Option<Uuid>, StorageError> {
    let (ehr_id, vo_id) = target;
    let id: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO access_delay (ehr_id, vo_id, basis, withheld_until, note) \
         SELECT $1, $2, 'professional', coalesce($3::timestamptz, \
                now() + make_interval(hours => $4)), $5 \
          WHERE EXISTS (SELECT 1 FROM vo_head h WHERE h.vo_id = $2 AND h.kind = 'COMPOSITION') \
            AND ($3::timestamptz IS NULL OR ($3::timestamptz > now() \
                 AND $3::timestamptz <= now() + make_interval(hours => $4))) \
         RETURNING id",
    )
    .bind(ehr_id)
    .bind(vo_id)
    .bind(until.map(jiff_sqlx::Timestamp::from))
    .bind(hours)
    .bind(note)
    .fetch_optional(&mut *tx)
    .await?;
    if id.is_some() {
        refresh_mark(tx, vo_id).await?;
    }
    Ok(id)
}

/// Release every hold of one object still in force and refresh its mark;
/// returns how many holds were released.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn release(tx: &mut PgConnection, vo_id: VoId) -> Result<u64, StorageError> {
    let released = sqlx::query(
        "UPDATE access_delay SET released_at = now() \
         WHERE vo_id = $1 AND released_at IS NULL AND withheld_until > now()",
    )
    .bind(vo_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    refresh_mark(tx, vo_id).await?;
    Ok(released)
}

/// Recompute the mark of one object: the latest end of its holds not
/// released, or `NULL`.
async fn refresh_mark(tx: &mut PgConnection, vo_id: VoId) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE vo_head h SET person_withheld_until = (\
             SELECT max(d.withheld_until) FROM access_delay d \
              WHERE d.vo_id = h.vo_id AND d.released_at IS NULL) \
         WHERE h.vo_id = $1",
    )
    .bind(vo_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Whether the object is withheld from the person now; `false` for an
/// unknown object.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn is_withheld(conn: &mut PgConnection, vo_id: VoId) -> Result<bool, StorageError> {
    let withheld: Option<bool> = sqlx::query_scalar(
        "SELECT coalesce(person_withheld_until > now(), false) FROM vo_head WHERE vo_id = $1",
    )
    .bind(vo_id)
    .fetch_optional(&mut *conn)
    .await?;
    Ok(withheld.unwrap_or(false))
}

/// The register rows of one EHR, newest hold first.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn register(pool: &PgPool, ehr_id: EhrId) -> Result<Vec<AccessDelayRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, ehr_id, vo_id, basis, withheld_at, withheld_until, released_at, note \
         FROM access_delay WHERE ehr_id = $1 ORDER BY withheld_at DESC, id DESC",
    )
    .bind(ehr_id)
    .fetch_all(&mut *acquire(pool).await?)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(AccessDelayRow {
                id: row.try_get("id")?,
                ehr_id: row.try_get("ehr_id")?,
                vo_id: row.try_get("vo_id")?,
                basis: row.try_get("basis")?,
                withheld_at: row
                    .try_get::<jiff_sqlx::Timestamp, _>("withheld_at")?
                    .to_jiff(),
                withheld_until: row
                    .try_get::<jiff_sqlx::Timestamp, _>("withheld_until")?
                    .to_jiff(),
                released_at: row
                    .try_get::<Option<jiff_sqlx::Timestamp>, _>("released_at")?
                    .map(jiff_sqlx::Timestamp::to_jiff),
                note: row.try_get("note")?,
            })
        })
        .collect()
}

/// Stamp every hold whose end passed unreleased and unrecorded, and return
/// them, so each expiry is recorded once; the caller commits once the records
/// are written.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn take_expired(tx: &mut PgConnection) -> Result<Vec<ExpiredHold>, StorageError> {
    let rows = sqlx::query(
        "UPDATE access_delay SET expiry_recorded_at = now() \
         WHERE released_at IS NULL AND expiry_recorded_at IS NULL AND withheld_until <= now() \
         RETURNING ehr_id, vo_id",
    )
    .fetch_all(&mut *tx)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(ExpiredHold {
                ehr_id: row.try_get("ehr_id")?,
                vo_id: row.try_get("vo_id")?,
            })
        })
        .collect()
}
