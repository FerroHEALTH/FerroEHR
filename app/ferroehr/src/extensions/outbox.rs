// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The outbox reader registry and the retention prune both domains share.
//!
//! **No openEHR spec governs eventing: our own design/extension.**
//! `event_outbox` has one drainer (the AMQP publisher, which stamps
//! `published_at`) and any number of cursor readers, each keeping a `last_seq`
//! high-water mark in `event_outbox_reader`. The prune deletes a published row
//! only once it is older than the retention window AND at or below every
//! active reader's cursor, in one statement, so a slow or paused reader never
//! loses rows. Which readers are active is reconciled from the configuration
//! at boot, so a reader that was switched off cannot hold the outbox forever.
//!
//! The instance is single-tenant, so a reader is exactly one row and one pass
//! drains the whole store.

use sqlx::PgPool;

/// A registered cursor reader over `event_outbox`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutboxReader(&'static str);

impl OutboxReader {
    /// Names a reader by its registry key.
    ///
    /// The registry refuses a key outside `^[a-z][a-z0-9-]{0,62}$` (the
    /// `ck_event_outbox_reader_name` check) when a call writes it.
    #[must_use]
    pub const fn new(name: &'static str) -> Self {
        Self(name)
    }

    /// The reader's registry key.
    #[must_use]
    pub const fn name(self) -> &'static str {
        self.0
    }
}

/// Record whether `reader` runs in this deployment, keeping its cursor.
///
/// Called at boot for every known reader. The row is created if missing, so a
/// reader that never advanced still holds (or releases) the floor.
///
/// # Errors
///
/// Returns the database error when the registry cannot be written.
pub async fn reconcile(
    pool: &PgPool,
    reader: OutboxReader,
    active: bool,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO event_outbox_reader (reader, active) VALUES ($1, $2) \
         ON CONFLICT (reader) DO UPDATE \
            SET active = EXCLUDED.active, updated_at = now()",
    )
    .bind(reader.name())
    .bind(active)
    .execute(pool)
    .await?;
    Ok(())
}

/// The reader's cursor: the highest `seq` it has fully processed, `0` before
/// its first advance.
///
/// # Errors
///
/// Returns the database error when the registry cannot be read.
pub async fn cursor(pool: &PgPool, reader: OutboxReader) -> Result<i64, sqlx::Error> {
    let last: Option<i64> =
        sqlx::query_scalar("SELECT last_seq FROM event_outbox_reader WHERE reader = $1")
            .bind(reader.name())
            .fetch_optional(pool)
            .await?;
    Ok(last.unwrap_or(0))
}

/// Advance the reader's cursor to `seq`, never backwards.
///
/// A reader that advances is running, so the row is registered active by this
/// call; a concurrent or delayed pass cannot move the mark back, which bounds
/// the at-least-once duplication window instead of leaving it open.
///
/// # Errors
///
/// Returns the database error when the registry cannot be written.
pub async fn advance(pool: &PgPool, reader: OutboxReader, seq: i64) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO event_outbox_reader (reader, last_seq, active) \
         VALUES ($1, $2, true) \
         ON CONFLICT (reader) DO UPDATE \
            SET last_seq = GREATEST(event_outbox_reader.last_seq, EXCLUDED.last_seq), \
                active = true, updated_at = now()",
    )
    .bind(reader.name())
    .bind(seq)
    .execute(pool)
    .await?;
    Ok(())
}

/// Delete the published rows older than the retention window that every active
/// reader has passed, in the domain `pool`'s own outbox. Returns the number
/// pruned.
///
/// The floor is the lowest cursor among the readers registered active, read in
/// the same statement; with none the window alone decides.
///
/// # Errors
///
/// Returns the database error when the delete fails.
pub async fn prune(pool: &PgPool, retention_days: i64) -> Result<u64, sqlx::Error> {
    let cutoff = format!("{retention_days} days");
    let result = sqlx::query(
        "DELETE FROM event_outbox \
         WHERE published_at IS NOT NULL \
           AND published_at < now() - $1::interval \
           AND seq <= COALESCE( \
                 (SELECT min(last_seq) FROM event_outbox_reader WHERE active), seq)",
    )
    .bind(cutoff)
    .execute(pool)
    .await?;
    let pruned = result.rows_affected();
    if pruned > 0 {
        tracing::debug!("pruned {pruned} published event rows past retention");
    }
    Ok(pruned)
}
