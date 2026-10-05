// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The usage report's database state: the installation id, the send claims and
//! the shared metrics window (`usage_report_instance`, `usage_report_window`).
//!
//! No openEHR spec governs this — our own design. Every decision that must hold
//! across replicas is one conditional `UPDATE` on the single instance row, which
//! `PostgreSQL` serializes by its row lock (PostgreSQL 18, "UPDATE", and
//! "Concurrency Control" §13.2.1,
//! <https://www.postgresql.org/docs/18/transaction-iso.html>): the first
//! replica's claim commits and every concurrent one re-evaluates the `WHERE`
//! against it and matches nothing. No replica keeps a cache of these decisions.

use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::usage_report::payload::BUCKETS;
use crate::usage_report::window::{Series, SeriesCounts, WindowCounts};

/// The installation id, and whether this call created it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Instance {
    /// The random installation id.
    pub id: Uuid,
    /// Whether this call created the row, which makes this start the first.
    pub created: bool,
}

/// What a daily claim found.
#[derive(Debug, Clone, PartialEq)]
pub enum DailyClaim {
    /// This caller holds the window: the counts it took, cleared in the store.
    Claimed(WindowCounts),
    /// Another replica sent within the window; the time it did.
    NotDue(jiff::Timestamp),
}

/// Why a store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database refused or did not answer.
    #[error("usage report store")]
    Sqlx(#[from] sqlx::Error),
    /// A window row names a series this build does not know, or carries a
    /// malformed histogram.
    #[error("usage report window row `{0}` is malformed")]
    MalformedRow(String),
    /// The server reported a `server_version_num` with no major in range.
    #[error("PostgreSQL reported server_version_num {0}")]
    ServerVersion(i32),
}

/// Returns the installation id, creating it when the database holds none.
///
/// # Errors
/// [`StoreError::Sqlx`] when a statement fails.
pub async fn ensure_instance(pool: &PgPool) -> Result<Instance, StoreError> {
    let mut conn = pool.acquire().await?;
    ensure_instance_on(&mut conn).await
}

/// [`ensure_instance`] on a connection the caller holds, which may be inside a
/// transaction.
///
/// # Errors
/// [`StoreError::Sqlx`] when a statement fails.
pub async fn ensure_instance_on(conn: &mut PgConnection) -> Result<Instance, StoreError> {
    let created: Option<Uuid> = sqlx::query_scalar(
        "INSERT INTO usage_report_instance (singleton) VALUES (true) \
         ON CONFLICT (singleton) DO NOTHING RETURNING instance_id",
    )
    .fetch_optional(&mut *conn)
    .await?;
    if let Some(id) = created {
        return Ok(Instance { id, created: true });
    }
    // A fresh statement, so READ COMMITTED sees a row a concurrent replica
    // committed while the insert above waited on it.
    let id: Uuid = sqlx::query_scalar("SELECT instance_id FROM usage_report_instance")
        .fetch_one(&mut *conn)
        .await?;
    Ok(Instance { id, created: false })
}

/// Reads the installation id without creating one.
///
/// # Errors
/// [`StoreError::Sqlx`] when the read fails.
pub async fn read_instance(pool: &PgPool) -> Result<Option<Uuid>, StoreError> {
    let id = sqlx::query_scalar("SELECT instance_id FROM usage_report_instance")
        .fetch_optional(pool)
        .await?;
    Ok(id)
}

/// Claims the start report: true when no start report was claimed in the last
/// ten minutes, in which case the claim is now recorded.
///
/// # Errors
/// [`StoreError::Sqlx`] when the update fails.
pub async fn claim_start(pool: &PgPool) -> Result<bool, StoreError> {
    let claimed: Option<Uuid> = sqlx::query_scalar(
        "UPDATE usage_report_instance SET last_start_sent_at = now() \
         WHERE last_start_sent_at IS NULL \
            OR last_start_sent_at <= now() - interval '10 minutes' \
         RETURNING instance_id",
    )
    .fetch_optional(pool)
    .await?;
    Ok(claimed.is_some())
}

/// Claims the daily report and takes the shared window with it, in one
/// transaction.
///
/// The claim succeeds when no daily report was claimed within the interval;
/// the window rows are then deleted and returned, so the next window starts
/// empty. Otherwise the window stays and the time of the last claim is
/// returned for scheduling.
///
/// # Errors
/// [`StoreError::Sqlx`] when a statement fails, and
/// [`StoreError::MalformedRow`] when a window row cannot be read back.
pub async fn claim_daily(pool: &PgPool) -> Result<DailyClaim, StoreError> {
    let mut tx = pool.begin().await?;
    // 23 hours: below a day by the scheduling jitter, so a replica that wakes
    // a little early still claims.
    let claimed: Option<Uuid> = sqlx::query_scalar(
        "UPDATE usage_report_instance SET last_daily_sent_at = now() \
         WHERE last_daily_sent_at IS NULL \
            OR last_daily_sent_at <= now() - interval '23 hours' \
         RETURNING instance_id",
    )
    .fetch_optional(&mut *tx)
    .await?;
    if claimed.is_none() {
        let last: Option<Option<jiff_sqlx::Timestamp>> =
            sqlx::query_scalar("SELECT last_daily_sent_at FROM usage_report_instance")
                .fetch_optional(&mut *tx)
                .await?;
        let last = last.flatten();
        tx.commit().await?;
        let last = last.map_or_else(jiff::Timestamp::now, jiff_sqlx::Timestamp::to_jiff);
        return Ok(DailyClaim::NotDue(last));
    }
    let rows: Vec<WindowRow> = sqlx::query_as(
        "DELETE FROM usage_report_window \
         RETURNING series, counts, errors_5xx, slow, max_ms",
    )
    .fetch_all(&mut *tx)
    .await?;
    let counts = window_counts(rows)?;
    tx.commit().await?;
    Ok(DailyClaim::Claimed(counts))
}

/// Reads the shared window without taking it.
///
/// # Errors
/// [`StoreError::Sqlx`] when the read fails, and
/// [`StoreError::MalformedRow`] when a row cannot be read back.
pub async fn peek_window(pool: &PgPool) -> Result<WindowCounts, StoreError> {
    let rows: Vec<WindowRow> =
        sqlx::query_as("SELECT series, counts, errors_5xx, slow, max_ms FROM usage_report_window")
            .fetch_all(pool)
            .await?;
    window_counts(rows)
}

/// Adds `counts` to the shared window, one statement per series in one
/// transaction.
///
/// # Errors
/// [`StoreError::Sqlx`] when a statement fails; nothing is added then.
pub async fn add_to_window(pool: &PgPool, counts: &WindowCounts) -> Result<(), StoreError> {
    if counts.is_empty() {
        return Ok(());
    }
    let mut tx = pool.begin().await?;
    for (series, add) in &counts.series {
        let buckets: Vec<i64> = add.buckets.iter().map(|count| to_i64(*count)).collect();
        sqlx::query(
            "INSERT INTO usage_report_window (series, counts, errors_5xx, slow, max_ms) \
             VALUES ($1, $2, $3, $4, $5) \
             ON CONFLICT (series) DO UPDATE SET \
                counts = ARRAY( \
                    SELECT stored + added \
                    FROM unnest(usage_report_window.counts, EXCLUDED.counts) \
                         WITH ORDINALITY AS pair(stored, added, position) \
                    ORDER BY position), \
                errors_5xx = usage_report_window.errors_5xx + EXCLUDED.errors_5xx, \
                slow = usage_report_window.slow + EXCLUDED.slow, \
                max_ms = GREATEST(usage_report_window.max_ms, EXCLUDED.max_ms)",
        )
        .bind(series.as_str())
        .bind(&buckets)
        .bind(to_i64(add.errors_5xx))
        .bind(to_i64(add.slow))
        .bind(add.max_ms)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// The `PostgreSQL` major version the pool reaches.
///
/// # Errors
/// [`StoreError::Sqlx`] when the read fails, and
/// [`StoreError::ServerVersion`] when the server reports a version number
/// with no major in range.
pub async fn postgres_major(pool: &PgPool) -> Result<u16, StoreError> {
    let version_num: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(pool)
        .await?;
    version_num
        .checked_div(10_000)
        .and_then(|major| u16::try_from(major).ok())
        .ok_or(StoreError::ServerVersion(version_num))
}

/// One stored window row.
#[derive(Debug, sqlx::FromRow)]
struct WindowRow {
    series: String,
    counts: Vec<i64>,
    errors_5xx: i64,
    slow: i64,
    max_ms: f64,
}

/// Reads stored rows back into window counts.
fn window_counts(rows: Vec<WindowRow>) -> Result<WindowCounts, StoreError> {
    let mut window = WindowCounts::default();
    for row in rows {
        let malformed = || StoreError::MalformedRow(row.series.clone());
        let series = Series::from_key(&row.series).ok_or_else(malformed)?;
        let mut buckets = [0_u64; BUCKETS];
        if row.counts.len() != BUCKETS {
            return Err(malformed());
        }
        for (bucket, stored) in buckets.iter_mut().zip(&row.counts) {
            *bucket = u64::try_from(*stored).map_err(|_negative| malformed())?;
        }
        let counts = SeriesCounts {
            buckets,
            errors_5xx: u64::try_from(row.errors_5xx).map_err(|_negative| malformed())?,
            slow: u64::try_from(row.slow).map_err(|_negative| malformed())?,
            max_ms: row.max_ms,
        };
        window.add(series, &counts);
    }
    Ok(window)
}

/// A count as `bigint`, saturating at its maximum.
fn to_i64(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}
