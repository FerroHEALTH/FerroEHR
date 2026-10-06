// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The legal marks storage carries beside the clinical content: restriction of
//! processing, the research objection, and the retention register.
//!
//! **No openEHR spec governs any of this — our own design/extension.** The RM
//! offers nothing that could stand in: `EHR_STATUS.is_queryable` limits
//! population queries and nothing else (RM ehr `master04-ehr_package.adoc` §EHR
//! Status), `EHR_ACCESS` is the RM's gateway for access decisions rather than a
//! per-object mark (§EHR Access), and content is indelible (RM common
//! `master06-change_control_package.adoc` §Logical Deletion), so a retention
//! period can never become a deletion timer here.
//!
//! Three registers and the marks they drive:
//!
//! - `restriction` records who asked, on what ground and when, and a lift is a
//!   second act on the same row rather than an erasure of it — GDPR Art. 18(3)
//!   asks that the subject "be informed by the controller before the
//!   restriction of processing is lifted", which needs the sequence to survive
//!   (`docs/law/eu/gdpr/text.html`). `vo_head.restricted_at` and
//!   `ehr.restricted_at` are the denormalised marks every read path filters on,
//!   recomputed from the register by [`refresh_restriction_marks`] so a
//!   whole-EHR lift cannot silently drop a per-object restriction that was
//!   placed separately.
//! - `ehr.research_objected_at` / `ehr.research_objection_ground` carry the
//!   Art. 21(6) objection at EHR grain.
//! - `retention_policy` / `retention_anchor` and the `retention_due` view hold
//!   the periods, their citations, and what has fallen due — a list, never a
//!   disposal.

use sqlx::{PgConnection, PgPool, Row as _};
use uuid::Uuid;

use crate::ids::{EhrId, VoId};
use crate::storage::error::StorageError;

/// One row of the restriction register.
#[derive(Debug, Clone)]
pub struct RestrictionRow {
    /// The register row id.
    pub id: Uuid,
    /// The EHR the restriction was requested for.
    pub ehr_id: EhrId,
    /// The restricted versioned object, or `None` for the whole EHR.
    pub vo_id: Option<VoId>,
    /// The GDPR Art. 18(1) point, or `national`.
    pub ground: String,
    /// When the restriction was requested.
    pub requested_at: jiff::Timestamp,
    /// When it was lifted, or `None` while it is in force.
    pub lifted_at: Option<jiff::Timestamp>,
    /// The controller's free-text note.
    pub note: Option<String>,
}

/// One retention period in the register.
#[derive(Debug, Clone)]
pub struct RetentionPolicyRow {
    /// The content category the period applies to.
    pub kind: String,
    /// The ISO 3166-1 alpha-2 jurisdiction whose rule it is.
    pub jurisdiction: String,
    /// The period, as PostgreSQL renders the `interval`.
    pub period: String,
    /// What the period is measured from.
    pub anchor: String,
    /// The legal citation the period rests on.
    pub source: String,
    /// The EHDS priority category the period is keyed on, or `None` for every
    /// object of the kind (Annex II 3.4).
    pub category: Option<String>,
}

/// One EHR whose retention period has run.
#[derive(Debug, Clone)]
pub struct RetentionDueRow {
    /// The EHR that is due.
    pub ehr_id: EhrId,
    /// The jurisdiction whose period ran out.
    pub jurisdiction: String,
    /// The content category the period covers.
    pub kind: String,
    /// The legal citation the period rests on.
    pub source: String,
    /// When the period ran out.
    pub due_at: jiff::Timestamp,
    /// Objects of that category carrying no per-object hold.
    pub objects_due: i64,
    /// Objects exempted by a per-object hold (`vo_head.retention_hold_at`).
    pub objects_held: i64,
    /// The EHDS priority category the period is keyed on, or `None` for every
    /// object of the kind.
    pub category: Option<String>,
}

/// The marks one EHR carries, as one read.
#[derive(Debug, Clone, Copy, Default)]
pub struct EhrMarks {
    /// Whether the whole EHR is under restriction of processing.
    pub restricted: bool,
    /// Whether an unoverridden research objection stands for this EHR.
    pub research_objected: bool,
}

// ── restriction ──────────────────────────────────────────────────────────────

/// Append a restriction to the register and refresh the marks it implies.
///
/// `vo_id = None` restricts the whole EHR. The call is not idempotent by
/// design: each request is its own evidence row, and two grounds may stand at
/// once (Art. 18(1) lists four independent ones).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, including the `ground` CHECK
/// refusing a ground outside the closed list and the foreign key refusing an
/// unknown EHR.
pub async fn record_restriction(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    vo_id: Option<VoId>,
    ground: &str,
    note: Option<&str>,
) -> Result<Uuid, StorageError> {
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO restriction (ehr_id, vo_id, ground, note) \
         VALUES ($1, $2, $3, $4) RETURNING id",
    )
    .bind(ehr_id)
    .bind(vo_id)
    .bind(ground)
    .bind(note)
    .fetch_one(&mut *tx)
    .await?;
    refresh_restriction_marks(tx, ehr_id).await?;
    Ok(id)
}

/// Lift every in-force restriction of `ehr_id` at the given grain and refresh
/// the marks. Returns how many register rows were lifted.
///
/// `vo_id = None` lifts the whole-EHR restrictions only; a per-object
/// restriction placed separately survives, which is why the marks are
/// recomputed rather than cleared.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn lift_restriction(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    vo_id: Option<VoId>,
) -> Result<u64, StorageError> {
    let lifted = sqlx::query(
        "UPDATE restriction SET lifted_at = now() \
         WHERE ehr_id = $1 AND vo_id IS NOT DISTINCT FROM $2 AND lifted_at IS NULL",
    )
    .bind(ehr_id)
    .bind(vo_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    refresh_restriction_marks(tx, ehr_id).await?;
    Ok(lifted)
}

/// Recompute `ehr.restricted_at` and every `vo_head.restricted_at` of one EHR
/// from the register.
///
/// Derived rather than assigned: the mark of an object is the earliest in-force
/// request that reaches it, whether it named the object or the whole EHR. That
/// makes set and lift idempotent and makes a whole-EHR lift unable to drop a
/// per-object restriction standing beside it.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn refresh_restriction_marks(
    tx: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<(), StorageError> {
    sqlx::query(
        "UPDATE ehr SET restricted_at = (\
             SELECT min(r.requested_at) FROM restriction r \
              WHERE r.ehr_id = $1 AND r.vo_id IS NULL AND r.lifted_at IS NULL) \
         WHERE id = $1",
    )
    .bind(ehr_id)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
        "UPDATE vo_head h SET restricted_at = (\
             SELECT min(r.requested_at) FROM restriction r \
              WHERE r.ehr_id = $1 AND r.lifted_at IS NULL \
                AND (r.vo_id IS NULL OR r.vo_id = h.vo_id)) \
         WHERE h.ehr_id = $1",
    )
    .bind(ehr_id)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The register rows of one EHR, newest request first.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn restrictions(
    pool: &PgPool,
    ehr_id: EhrId,
) -> Result<Vec<RestrictionRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT id, ehr_id, vo_id, ground, requested_at, lifted_at, note \
         FROM restriction WHERE ehr_id = $1 ORDER BY requested_at DESC, id DESC",
    )
    .bind(ehr_id)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(RestrictionRow {
                id: row.try_get("id")?,
                ehr_id: row.try_get("ehr_id")?,
                vo_id: row.try_get("vo_id")?,
                ground: row.try_get("ground")?,
                requested_at: row
                    .try_get::<jiff_sqlx::Timestamp, _>("requested_at")?
                    .to_jiff(),
                lifted_at: row
                    .try_get::<Option<jiff_sqlx::Timestamp>, _>("lifted_at")?
                    .map(jiff_sqlx::Timestamp::to_jiff),
                note: row.try_get("note")?,
            })
        })
        .collect()
}

/// Whether a versioned object carries a restriction mark, without reading it.
///
/// The write guard's question: a write to a restricted object is refused,
/// because Art. 18(2) leaves storage as the only processing the restriction
/// admits.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn is_restricted(tx: &mut PgConnection, vo_id: VoId) -> Result<bool, StorageError> {
    let restricted: Option<bool> =
        sqlx::query_scalar("SELECT restricted_at IS NOT NULL FROM vo_head WHERE vo_id = $1")
            .bind(vo_id)
            .fetch_optional(&mut *tx)
            .await?;
    Ok(restricted.unwrap_or(false))
}

// ── the research objection ───────────────────────────────────────────────────

/// Record the subject's objection to research processing, or the controller's
/// override of one.
///
/// `ground = None` places the objection; `ground = Some(...)` records the
/// Art. 21(6) public-interest ground that overrides it, which leaves the
/// objection instant in place so the sequence stays readable.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn set_research_objection(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    ground: Option<&str>,
) -> Result<bool, StorageError> {
    let updated = sqlx::query(
        "UPDATE ehr SET research_objected_at = coalesce(research_objected_at, now()), \
                        research_objection_ground = $2 \
         WHERE id = $1",
    )
    .bind(ehr_id)
    .bind(ground)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(updated > 0)
}

/// Withdraw the objection entirely: the subject asked for it and then did not.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn clear_research_objection(
    tx: &mut PgConnection,
    ehr_id: EhrId,
) -> Result<bool, StorageError> {
    let updated = sqlx::query(
        "UPDATE ehr SET research_objected_at = NULL, research_objection_ground = NULL \
         WHERE id = $1",
    )
    .bind(ehr_id)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(updated > 0)
}

/// The marks one EHR carries, or `None` when the EHR does not exist.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn ehr_marks(pool: &PgPool, ehr_id: EhrId) -> Result<Option<EhrMarks>, StorageError> {
    let row = sqlx::query(
        "SELECT restricted_at IS NOT NULL AS restricted, \
                (research_objected_at IS NOT NULL AND research_objection_ground IS NULL) \
                    AS objected \
         FROM ehr WHERE id = $1",
    )
    .bind(ehr_id)
    .fetch_optional(pool)
    .await?;
    row.as_ref()
        .map(|row| {
            Ok(EhrMarks {
                restricted: row.try_get("restricted")?,
                research_objected: row.try_get("objected")?,
            })
        })
        .transpose()
}

// ── retention ────────────────────────────────────────────────────────────────

/// Declare (or re-declare) the retention period of one content kind in one
/// jurisdiction, optionally keyed on one EHDS priority category (Annex II 3.4).
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, including the CHECKs
/// refusing an unknown kind, an unknown category, an unknown anchor rule or a
/// non-positive period.
pub async fn put_retention_policy(
    tx: &mut PgConnection,
    kind: &str,
    jurisdiction: &str,
    category: Option<&str>,
    period: &str,
    anchor: &str,
    source: &str,
) -> Result<(), StorageError> {
    sqlx::query(
        "INSERT INTO retention_policy (kind, jurisdiction, category, period, anchor, source) \
         VALUES ($1, $2, $3, $4::interval, $5, $6) \
         ON CONFLICT (kind, jurisdiction, category) DO UPDATE \
            SET period = EXCLUDED.period, anchor = EXCLUDED.anchor, source = EXCLUDED.source",
    )
    .bind(kind)
    .bind(jurisdiction)
    .bind(category)
    .bind(period)
    .bind(anchor)
    .bind(source)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// The whole retention register, ordered so a rendered page is stable.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn retention_policies(pool: &PgPool) -> Result<Vec<RetentionPolicyRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT kind, jurisdiction, category, period::text AS period, anchor, source \
         FROM retention_policy ORDER BY jurisdiction, kind, category NULLS FIRST",
    )
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(RetentionPolicyRow {
                kind: row.try_get("kind")?,
                jurisdiction: row.try_get("jurisdiction")?,
                period: row.try_get("period")?,
                anchor: row.try_get("anchor")?,
                source: row.try_get("source")?,
                category: row.try_get("category")?,
            })
        })
        .collect()
}

/// Replace the boot mirror of the `[audit.categories]` map (`category_map`)
/// with `rows`, `(key kind, case-folded key, category)`, in one transaction.
///
/// The table lock serializes replicas booting at once, so the mirror is always
/// one map whole. No openEHR spec governs the map — our own design/extension.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, including the CHECKs
/// refusing a key kind, an unfolded key or a category spelling the register
/// does not know.
pub async fn replace_category_map(
    pool: &PgPool,
    rows: &[(&str, String, String)],
) -> Result<(), StorageError> {
    let mut tx = pool.begin().await?;
    sqlx::query("LOCK TABLE category_map IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM category_map")
        .execute(&mut *tx)
        .await?;
    let kinds: Vec<&str> = rows.iter().map(|(kind, _, _)| *kind).collect();
    let keys: Vec<&str> = rows.iter().map(|(_, key, _)| key.as_str()).collect();
    let categories: Vec<&str> = rows.iter().map(|(_, _, c)| c.as_str()).collect();
    sqlx::query(
        "INSERT INTO category_map (key_kind, key, category) \
         SELECT * FROM UNNEST($1::text[], $2::text[], $3::text[])",
    )
    .bind(kinds)
    .bind(keys)
    .bind(categories)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Record one EHR's jurisdiction, anchor instant and any EHR-wide hold.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, including the CHECK refusing
/// a hold instant without a ground.
pub async fn put_retention_anchor(
    tx: &mut PgConnection,
    ehr_id: EhrId,
    jurisdiction: &str,
    anchored_at: Option<&str>,
    hold: Option<(&str, &str)>,
) -> Result<(), StorageError> {
    let (hold_at, hold_ground) = match hold {
        Some((at, ground)) => (Some(at), Some(ground)),
        None => (None, None),
    };
    sqlx::query(
        "INSERT INTO retention_anchor (ehr_id, jurisdiction, anchored_at, hold_at, hold_ground) \
         VALUES ($1, $2, $3::timestamptz, $4::timestamptz, $5) \
         ON CONFLICT (ehr_id) DO UPDATE \
            SET jurisdiction = EXCLUDED.jurisdiction, \
                anchored_at = EXCLUDED.anchored_at, \
                hold_at = EXCLUDED.hold_at, \
                hold_ground = EXCLUDED.hold_ground",
    )
    .bind(ehr_id)
    .bind(jurisdiction)
    .bind(anchored_at)
    .bind(hold_at)
    .bind(hold_ground)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

/// Place or release the per-object retention hold that exempts one versioned
/// object from disposal (EPDV Art. 10 Abs. 2 lit. b,
/// `docs/law/ch/epdv/text-de.html`). Returns whether the object exists.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn set_retention_hold(
    tx: &mut PgConnection,
    vo_id: VoId,
    held: bool,
) -> Result<bool, StorageError> {
    let updated = sqlx::query(
        "UPDATE vo_head SET retention_hold_at = CASE WHEN $2 \
             THEN coalesce(retention_hold_at, now()) ELSE NULL END \
         WHERE vo_id = $1",
    )
    .bind(vo_id)
    .bind(held)
    .execute(&mut *tx)
    .await?
    .rows_affected();
    Ok(updated > 0)
}

/// What has fallen due, oldest first, bounded by `limit`.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure.
pub async fn retention_due(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<RetentionDueRow>, StorageError> {
    let rows = sqlx::query(
        "SELECT ehr_id, jurisdiction, kind, category, source, due_at, objects_due, \
         objects_held \
         FROM retention_due ORDER BY due_at, ehr_id, kind, category NULLS FIRST LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    rows.iter()
        .map(|row| {
            Ok(RetentionDueRow {
                ehr_id: row.try_get("ehr_id")?,
                jurisdiction: row.try_get("jurisdiction")?,
                kind: row.try_get("kind")?,
                source: row.try_get("source")?,
                due_at: row.try_get::<jiff_sqlx::Timestamp, _>("due_at")?.to_jiff(),
                objects_due: row.try_get("objects_due")?,
                objects_held: row.try_get("objects_held")?,
                category: row.try_get("category")?,
            })
        })
        .collect()
}
