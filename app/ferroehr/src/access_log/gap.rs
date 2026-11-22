// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The fold of the access log's tombstones (#5619, #6242).
//!
//! A tombstone is kept one more retention horizon, its entry's own: as long
//! after its removal as the entry was kept before it. After that, the
//! entries of a sealed window whose records are all tombstones past their end
//! leave the log together, and the window is recorded as folded with the
//! grounds its tombstones named; its seal stays, so the seals still follow
//! one another. The imported logs fold their tombstones past their end one by
//! one, keeping each one's uid, bundle and content digest. No openEHR spec
//! governs audit retention: our own design.

use crate::access_log::expiry::Log;
use crate::access_log::store::{AccessLogError, AccessLogStore};
use sqlx::Row as _;

use crate::db::acquire::acquire;

/// What one fold removed and wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Folded {
    /// The tombstones removed.
    pub tombstones: u64,
    /// The windows or imported tombstones recorded as folded.
    pub gaps: u64,
}

impl AccessLogStore {
    /// Fold the tombstones of `log` whose own retention ended before `now`,
    /// in one transaction.
    ///
    /// On the local log, up to `limit` sealed windows whose records are all
    /// tombstones past their end fold, oldest first, through
    /// `audit.fold_access_log_tombstones()`, the one path a record leaves the
    /// log by. On the imported logs, up to `limit` tombstones past their end
    /// fold through `audit.fold_access_log_imported()`. A caller repeats the
    /// call until it folds nothing.
    ///
    /// # Errors
    /// The store errors when the statement fails.
    pub async fn fold_expired(
        &self,
        log: Log,
        now: jiff::Timestamp,
        limit: u16,
    ) -> Result<Folded, AccessLogError> {
        let statement = match log {
            Log::Local => {
                "SELECT folded, gaps FROM audit.fold_access_log_tombstones($1::text::timestamptz, $2)"
            }
            Log::Imported => {
                "SELECT tombstones AS folded, gaps \
                 FROM audit.fold_access_log_imported($1::text::timestamptz, $2)"
            }
        };
        let row = sqlx::query(statement)
            .bind(now.to_string())
            .bind(i32::from(limit))
            .fetch_one(&mut *acquire(self.pool()).await?)
            .await?;
        let count = |column: &str| -> Result<u64, AccessLogError> {
            let value: i64 = row.try_get(column)?;
            u64::try_from(value)
                .map_err(|error| AccessLogError::Store(sqlx::Error::Decode(Box::new(error))))
        };
        Ok(Folded {
            tombstones: count("folded")?,
            gaps: count("gaps")?,
        })
    }
}
