// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The gaps of the access log's chain: each run of expired tombstones folded
//! into one record of the fold (#5619).
//!
//! A tombstone keeps an expired entry's link so the records after it verify.
//! It is kept one more retention horizon, its entry's own: as long after its
//! removal as the entry was kept before it. After that, a run of consecutive
//! tombstones that are all past it folds into one gap, which keeps the
//! positions the run held, the link digest of its last record, that link's
//! signature and the grounds its tombstones named. The record after the run
//! links to the gap's link, so the chain verifies across it, and a missing
//! position no gap accounts for is still a deletion. A run never spans a
//! checkpoint's position except as its last, so every checkpoint still names
//! a link the log holds.
//!
//! The draft access log defines no gap: the gap is FerroEHR's own record, and
//! no openEHR spec governs audit retention: our own design.

use serde::{Deserialize, Serialize};
use sqlx::Row;
use sqlx::postgres::PgRow;

use crate::access_log::expiry::Log;
use crate::access_log::store::{AccessLogError, AccessLogStore};
use crate::db::acquire::acquire;
use crate::versioning::audit::dv_date_time;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::v1_2::data_types::quantity::date_time::dv_date_time::DvDateTime;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

/// A run of positions of one system's access log whose tombstones were
/// folded: the record of the fold.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChainGap {
    /// The first position the run held.
    pub first_sequence: i64,
    /// The last position the run held.
    pub last_sequence: i64,
    /// `ACCESS_LOG_LINK.digest` of the run's last record, which the record
    /// after the run links to.
    pub link: String,
    /// `ACCESS_LOG_LINK.signature` of the run's last record, the signature of
    /// `link`, when that link was signed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub signature: Option<String>,
    /// Every distinct ground the run's tombstones named.
    pub grounds: NonEmptyVec<DvCodedText>,
    /// When the run was folded.
    pub time_folded: DvDateTime,
}

/// What one fold removed and wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Folded {
    /// The tombstones removed.
    pub tombstones: u64,
    /// The gaps written, one per run.
    pub gaps: u64,
}

impl AccessLogStore {
    /// Fold the tombstones of `log` whose own retention ended before `now`
    /// into one gap per run, in one transaction.
    ///
    /// On the local chain, up to `limit` tombstones fold, oldest end first,
    /// through `audit.fold_access_log_tombstones()`, the one path a record
    /// leaves the log by; a caller repeats the call until fewer than `limit`
    /// come back. On the imported logs, per exporting system and within each
    /// span between two imported checkpoints, the positions before the first
    /// record still kept fold in whole runs that hold a tombstone, oldest
    /// first, up to about `limit` positions, through
    /// `audit.fold_access_log_imported()` (#5664); a caller repeats the call
    /// until it writes no gap. Replicas serialise on each function's advisory
    /// lock, so two runs never fold the same position.
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

    /// The gaps that hold a position from `from` to `to`, both included, in
    /// chain order.
    ///
    /// # Errors
    /// The store errors when the statement fails, and
    /// [`AccessLogError::Unreadable`] when a stored gap does not read.
    pub async fn gaps(&self, from: i64, to: i64) -> Result<Vec<ChainGap>, AccessLogError> {
        let rows = sqlx::query(
            "SELECT first_seq, last_seq, link, signature, grounds::text AS grounds, time_folded \
             FROM audit.access_log_chain_gap WHERE last_seq >= $1 AND first_seq <= $2 \
             ORDER BY first_seq",
        )
        .bind(from)
        .bind(to)
        .fetch_all(&mut *acquire(self.pool()).await?)
        .await?;
        rows.iter().map(gap).collect()
    }
}

/// The gap `row` holds: `first_seq`, `last_seq`, `link`, `signature`,
/// `grounds` as text and `time_folded`.
///
/// # Errors
/// [`AccessLogError::Unreadable`] when its grounds do not read.
pub(crate) fn gap(row: &PgRow) -> Result<ChainGap, AccessLogError> {
    let first_sequence: i64 = row.try_get("first_seq")?;
    let grounds: String = row.try_get("grounds")?;
    let grounds = serde_json::from_str(&grounds).map_err(|source| AccessLogError::Unreadable {
        chain_seq: first_sequence,
        source,
    })?;
    let time_folded = row
        .try_get::<jiff_sqlx::Timestamp, _>("time_folded")?
        .to_jiff();
    Ok(ChainGap {
        first_sequence,
        last_sequence: row.try_get("last_seq")?,
        link: row.try_get("link")?,
        signature: row.try_get("signature")?,
        grounds,
        time_folded: dv_date_time(&time_folded),
    })
}
