// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The usage report's database state: `ferrohealth-usage-report`'s [`Store`]
//! over the clinical pool every replica shares.
//!
//! It holds the installation id, the start and daily claims, and the counts
//! of every replica since the last daily report (`usage_report_instance`,
//! `usage_report_window`, `usage_report_counter`).
//!
//! No openEHR spec governs this: our own design. Every decision that must hold
//! across replicas is one conditional `UPDATE` on the single instance row, which
//! `PostgreSQL` serializes by its row lock (PostgreSQL 18, "UPDATE", and
//! "Concurrency Control" §13.2.1,
//! <https://www.postgresql.org/docs/18/transaction-iso.html>): the first
//! replica's claim commits and every concurrent one re-evaluates the `WHERE`
//! against it and matches nothing. No replica keeps a cache of these decisions.

use std::collections::BTreeMap;

use ferrohealth_usage_report::envelope::{BUCKETS, Key};
use ferrohealth_usage_report::state::{DailyClaim, Instance, Store, StoreFuture};
use ferrohealth_usage_report::window::{Counts, GroupCounts};
use sqlx::Connection as _;
use sqlx::{PgConnection, PgPool};
use uuid::Uuid;

use crate::db::acquire::acquire;

/// Why a store operation failed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// The database refused or did not answer.
    #[error("usage report store")]
    Sqlx(#[from] sqlx::Error),
    /// No connection could be obtained from the pool.
    #[error("usage report store unavailable")]
    Acquire(#[from] crate::db::acquire::AcquireError),
    /// A stored row names a key outside the envelope's pattern, or carries a
    /// malformed histogram or a negative count.
    #[error("usage report row `{0}` is malformed")]
    MalformedRow(String),
    /// The server reported a `server_version_num` with no major in range.
    #[error("PostgreSQL reported server_version_num {0}")]
    ServerVersion(i32),
}

/// The usage report state of one installation, in the clinical database.
#[derive(Debug, Clone)]
pub struct PgStore {
    pool: PgPool,
}

impl PgStore {
    /// The store over the clinical pool `pool`.
    #[must_use]
    pub const fn new(pool: PgPool) -> Self {
        Self { pool }
    }

    /// Reads every replica's counts since the last daily report, without
    /// taking them.
    ///
    /// # Errors
    /// [`StoreError::Sqlx`] when the read fails, and
    /// [`StoreError::MalformedRow`] when a row cannot be read back.
    pub async fn peek(&self) -> Result<Counts, StoreError> {
        let mut conn = acquire(&self.pool).await?;
        let groups: Vec<GroupRow> =
            sqlx::query_as("SELECT series, counts, errors_5xx, max_ms FROM usage_report_window")
                .fetch_all(&mut *conn)
                .await?;
        let counters: Vec<CounterRow> =
            sqlx::query_as("SELECT counter, count FROM usage_report_counter")
                .fetch_all(&mut *conn)
                .await?;
        counts_of(groups, counters)
    }

    async fn claim_daily_now(&self, local: &Counts) -> Result<DailyClaim, StoreError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        add_counts(&mut tx, local).await?;
        // 23 hours: below a day by the reporter's scheduling spread, so a
        // replica that wakes a little early still claims.
        let claimed: Option<Uuid> = sqlx::query_scalar(
            "UPDATE usage_report_instance SET last_daily_sent_at = now() \
             WHERE last_daily_sent_at IS NULL \
                OR last_daily_sent_at <= now() - interval '23 hours' \
             RETURNING instance_id",
        )
        .fetch_optional(&mut *tx)
        .await?;
        if claimed.is_none() {
            tx.commit().await?;
            return Ok(DailyClaim::Sent);
        }
        let groups: Vec<GroupRow> = sqlx::query_as(
            "DELETE FROM usage_report_window RETURNING series, counts, errors_5xx, max_ms",
        )
        .fetch_all(&mut *tx)
        .await?;
        let counters: Vec<CounterRow> =
            sqlx::query_as("DELETE FROM usage_report_counter RETURNING counter, count")
                .fetch_all(&mut *tx)
                .await?;
        let counts = counts_of(groups, counters)?;
        tx.commit().await?;
        Ok(DailyClaim::Taken(counts))
    }

    async fn restore_now(&self, counts: &Counts) -> Result<(), StoreError> {
        let mut conn = acquire(&self.pool).await?;
        let mut tx = conn.begin().await?;
        add_counts(&mut tx, counts).await?;
        // NOTE: no specification governs this: our own design; a daily report that
        // never arrived releases its claim, so the reporter's retry an hour later
        // claims again on whichever replica wakes first.
        sqlx::query("UPDATE usage_report_instance SET last_daily_sent_at = NULL")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
}

impl Store for PgStore {
    fn instance(&self) -> StoreFuture<'_, Instance> {
        Box::pin(async move {
            ensure_instance(&self.pool)
                .await
                .map_err(ferrohealth_usage_report::state::StoreError::new)
        })
    }

    fn claim_start(&self, _now: jiff::Timestamp) -> StoreFuture<'_, bool> {
        Box::pin(async move {
            claim_start(&self.pool)
                .await
                .map_err(ferrohealth_usage_report::state::StoreError::new)
        })
    }

    fn claim_daily(&self, _now: jiff::Timestamp, local: Counts) -> StoreFuture<'_, DailyClaim> {
        Box::pin(async move {
            self.claim_daily_now(&local)
                .await
                .map_err(ferrohealth_usage_report::state::StoreError::new)
        })
    }

    fn restore(&self, counts: Counts) -> StoreFuture<'_, Option<Counts>> {
        Box::pin(async move {
            self.restore_now(&counts)
                .await
                .map(|()| None)
                .map_err(ferrohealth_usage_report::state::StoreError::new)
        })
    }
}

/// Returns the installation id, creating it when the database holds none.
///
/// # Errors
/// [`StoreError::Sqlx`] when a statement fails.
pub async fn ensure_instance(pool: &PgPool) -> Result<Instance, StoreError> {
    let mut conn = acquire(pool).await?;
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
        .fetch_optional(&mut *acquire(pool).await?)
        .await?;
    Ok(id)
}

/// Claims the start report: true when no start report was claimed in the last
/// ten minutes, in which case the claim is now recorded.
///
/// # Errors
/// [`StoreError::Sqlx`] when the update fails.
async fn claim_start(pool: &PgPool) -> Result<bool, StoreError> {
    let claimed: Option<Uuid> = sqlx::query_scalar(
        "UPDATE usage_report_instance SET last_start_sent_at = now() \
         WHERE last_start_sent_at IS NULL \
            OR last_start_sent_at <= now() - interval '10 minutes' \
         RETURNING instance_id",
    )
    .fetch_optional(&mut *acquire(pool).await?)
    .await?;
    Ok(claimed.is_some())
}

/// The `PostgreSQL` major version the pool reaches.
///
/// # Errors
/// [`StoreError::Sqlx`] when the read fails, and
/// [`StoreError::ServerVersion`] when the server reports a version number
/// with no major in range.
pub async fn postgres_major(pool: &PgPool) -> Result<u16, StoreError> {
    let version_num: i32 = sqlx::query_scalar("SELECT current_setting('server_version_num')::int")
        .fetch_one(&mut *acquire(pool).await?)
        .await?;
    version_num
        .checked_div(10_000)
        .and_then(|major| u16::try_from(major).ok())
        .ok_or(StoreError::ServerVersion(version_num))
}

/// Adds `counts` to the stored window, one statement per group and counter,
/// inside the caller's transaction.
async fn add_counts(conn: &mut PgConnection, counts: &Counts) -> Result<(), StoreError> {
    for (group, add) in &counts.groups {
        let buckets: Vec<i64> = add.buckets.iter().map(|count| to_i64(*count)).collect();
        sqlx::query(
            "INSERT INTO usage_report_window (series, counts, errors_5xx, max_ms) \
             VALUES ($1, $2, $3, $4) \
             ON CONFLICT (series) DO UPDATE SET \
                counts = ARRAY( \
                    SELECT stored + added \
                    FROM unnest(usage_report_window.counts, EXCLUDED.counts) \
                         WITH ORDINALITY AS pair(stored, added, position) \
                    ORDER BY position), \
                errors_5xx = usage_report_window.errors_5xx + EXCLUDED.errors_5xx, \
                max_ms = GREATEST(usage_report_window.max_ms, EXCLUDED.max_ms)",
        )
        .bind(group.as_str())
        .bind(&buckets)
        .bind(to_i64(add.errors_5xx))
        .bind(add.max_ms)
        .execute(&mut *conn)
        .await?;
    }
    for (counter, count) in &counts.counters {
        sqlx::query(
            "INSERT INTO usage_report_counter (counter, count) VALUES ($1, $2) \
             ON CONFLICT (counter) DO UPDATE SET \
                count = usage_report_counter.count + EXCLUDED.count",
        )
        .bind(counter.as_str())
        .bind(to_i64(*count))
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

/// One stored group row.
#[derive(Debug, sqlx::FromRow)]
struct GroupRow {
    series: String,
    counts: Vec<i64>,
    errors_5xx: i64,
    max_ms: f64,
}

/// One stored counter row.
#[derive(Debug, sqlx::FromRow)]
struct CounterRow {
    counter: String,
    count: i64,
}

/// Reads stored rows back into window counts.
fn counts_of(groups: Vec<GroupRow>, counters: Vec<CounterRow>) -> Result<Counts, StoreError> {
    let mut read = Counts::default();
    for row in groups {
        let malformed = || StoreError::MalformedRow(row.series.clone());
        let key = Key::new(&row.series).ok_or_else(malformed)?;
        if row.counts.len() != BUCKETS {
            return Err(malformed());
        }
        let mut buckets = [0_u64; BUCKETS];
        for (bucket, stored) in buckets.iter_mut().zip(&row.counts) {
            *bucket = u64::try_from(*stored).map_err(|_negative| malformed())?;
        }
        let group = GroupCounts {
            buckets,
            errors_5xx: u64::try_from(row.errors_5xx).map_err(|_negative| malformed())?,
            max_ms: row.max_ms,
        };
        read.groups.insert(key, group);
    }
    let mut counts = BTreeMap::new();
    for row in counters {
        let malformed = || StoreError::MalformedRow(row.counter.clone());
        let key = Key::new(&row.counter).ok_or_else(malformed)?;
        counts.insert(
            key,
            u64::try_from(row.count).map_err(|_negative| malformed())?,
        );
    }
    read.counters = counts;
    Ok(read)
}

/// A count as `bigint`, saturating at its maximum.
fn to_i64(count: u64) -> i64 {
    i64::try_from(count).unwrap_or(i64::MAX)
}
