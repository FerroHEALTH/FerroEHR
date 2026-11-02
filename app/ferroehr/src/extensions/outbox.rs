// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The retention prune both domains' outboxes share.
//!
//! **No openEHR spec governs eventing: our own design/extension.**
//! `event_outbox` has one drainer, the AMQP publisher, which stamps
//! `published_at` once the broker confirms a row; nothing else reads it. The
//! prune deletes the published rows older than the retention window, so an
//! unpublished row stays until the broker has it.

use sqlx::PgPool;

use crate::db::acquire::{AcquireError, acquire};

/// A failure of the outbox prune statement.
#[derive(Debug, thiserror::Error)]
pub enum OutboxError {
    /// No connection could be obtained from the pool.
    #[error(transparent)]
    Acquire(#[from] AcquireError),
    /// The statement failed on the acquired connection.
    #[error("outbox statement failed")]
    Database(#[from] sqlx::Error),
}

/// Delete the published rows older than the retention window in the domain
/// `pool`'s own outbox. Returns the number pruned.
///
/// # Errors
///
/// [`OutboxError::Acquire`] when no connection can be obtained, and
/// [`OutboxError::Database`] when the delete fails.
pub async fn prune(pool: &PgPool, retention_days: i64) -> Result<u64, OutboxError> {
    let cutoff = format!("{retention_days} days");
    let result = sqlx::query(
        "DELETE FROM event_outbox \
         WHERE published_at IS NOT NULL \
           AND published_at < now() - $1::interval",
    )
    .bind(cutoff)
    .execute(&mut *acquire(pool).await?)
    .await?;
    let pruned = result.rows_affected();
    if pruned > 0 {
        tracing::debug!("pruned {pruned} published event rows past retention");
    }
    Ok(pruned)
}
