// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A `pg_snapshot` as a copy holds it: the transactions whose writes it
//! reads (#6074).
//!
//! A copy of the party store remembers the snapshot it read under, and the
//! store stamps each changed party with the xid of the transaction that
//! changed it, so the next read takes exactly the stamps that snapshot did
//! not see: `stamped_xid >= pg_snapshot_xmin(held) AND NOT
//! pg_visible_in_snapshot(stamped_xid, held)` (PostgreSQL 19, "Transaction ID
//! and Snapshot Information Functions"). No specification governs the copy:
//! our own design.

use std::fmt;

use sqlx::PgConnection;

use crate::storage::error::StorageError;

/// A snapshot read with `pg_current_snapshot()::text`: `xmin:xmax:xip_list`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// The lowest xid still running when it was taken.
    xmin: u64,
    /// One past the highest xid that had ended.
    xmax: u64,
    /// The xids at or above `xmin` and below `xmax` still running, sorted.
    xip: Vec<u64>,
    /// The text it was read from, which a statement binds back.
    text: Box<str>,
}

/// A snapshot text that is not `xmin:xmax:xip_list`.
#[derive(Debug, thiserror::Error)]
#[error("{text:?} is not a pg_snapshot")]
pub struct NotASnapshot {
    text: String,
    /// The xid that does not read, when one does not.
    #[source]
    xid: Option<std::num::ParseIntError>,
}

impl std::str::FromStr for Snapshot {
    type Err = NotASnapshot;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let refused = || NotASnapshot {
            text: text.to_owned(),
            xid: None,
        };
        let mut parts = text.splitn(3, ':');
        let mut next = || {
            parts
                .next()
                .ok_or_else(refused)
                .map(str::trim)
                .map(str::to_owned)
        };
        let (xmin, xmax, xip) = (next()?, next()?, next()?);
        let xid = |part: &str| {
            part.parse::<u64>().map_err(|xid| NotASnapshot {
                text: text.to_owned(),
                xid: Some(xid),
            })
        };
        let mut xip = xip
            .split(',')
            .filter(|part| !part.is_empty())
            .map(xid)
            .collect::<Result<Vec<u64>, _>>()?;
        xip.sort_unstable();
        Ok(Self {
            xmin: xid(&xmin)?,
            xmax: xid(&xmax)?,
            xip,
            text: text.trim().into(),
        })
    }
}

impl fmt::Display for Snapshot {
    /// The text `pg_snapshot` reads back.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text)
    }
}

impl Snapshot {
    /// The text `pg_snapshot` reads back, bound as `$n::pg_snapshot`.
    #[must_use]
    pub fn text(&self) -> &str {
        &self.text
    }

    /// Whether the writes of the transaction `xid` are visible to it, as
    /// `pg_visible_in_snapshot` answers for a transaction that committed.
    #[must_use]
    pub fn sees(&self, xid: u64) -> bool {
        xid < self.xmin || (xid < self.xmax && self.xip.binary_search(&xid).is_err())
    }

    /// Whether it sees every transaction `older` sees, so a copy read under
    /// it holds every stamp a copy read under `older` holds.
    ///
    /// Conservative: `false` when `older` reaches past its `xmax`, even if
    /// every xid between them was still running under `older`.
    #[must_use]
    pub fn covers(&self, older: &Self) -> bool {
        older.xmax <= self.xmax && self.xip.iter().all(|xid| !older.sees(*xid))
    }
}

/// The snapshot `text` names, as the store wrote it.
///
/// # Errors
/// [`StorageError::InvalidRows`] when it does not read.
pub fn read(text: &str) -> Result<Snapshot, StorageError> {
    text.parse()
        .map_err(|refused: NotASnapshot| StorageError::InvalidRows(refused.to_string()))
}

/// The snapshot the running statement reads under on `conn`, the
/// transaction's own in a repeatable-read one.
///
/// # Errors
/// [`StorageError::Database`] on a driver failure, and
/// [`StorageError::InvalidRows`] for a snapshot that does not read.
pub async fn current(conn: &mut PgConnection) -> Result<Snapshot, StorageError> {
    let text: String = sqlx::query_scalar("SELECT pg_current_snapshot()::text")
        .fetch_one(&mut *conn)
        .await?;
    read(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(text: &str) -> Snapshot {
        text.parse().expect("a snapshot")
    }

    #[test]
    fn a_snapshot_reads_back_as_its_text() {
        for text in ["10:10:", "10:20:10,14,15", "748:750:748"] {
            assert_eq!(snapshot(text).to_string(), text);
        }
        assert!("10:20".parse::<Snapshot>().is_err());
        assert!("10:x:".parse::<Snapshot>().is_err());
    }

    #[test]
    fn a_snapshot_sees_what_ended_before_it_was_taken() {
        let held = snapshot("10:20:10,14,15");
        assert!(held.sees(9));
        assert!(!held.sees(10), "running");
        assert!(held.sees(11));
        assert!(!held.sees(14), "running");
        assert!(!held.sees(20), "started after");
    }

    #[test]
    fn a_later_snapshot_covers_an_earlier_one_only_once_its_running_ones_ended() {
        let older = snapshot("10:20:10,14");
        assert!(older.covers(&older));
        assert!(snapshot("14:25:14,21").covers(&older), "14 runs in both");
        assert!(snapshot("30:30:").covers(&older));
        assert!(
            !snapshot("10:25:10,12").covers(&older),
            "12 ended before older was taken and runs here"
        );
        assert!(!older.covers(&snapshot("30:30:")), "never backwards");
        assert!(!snapshot("10:18:").covers(&older));
    }
}
