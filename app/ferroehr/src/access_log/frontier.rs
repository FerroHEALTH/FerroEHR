// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The frontier of the access log: the highest entry id at or below which
//! every entry is committed or never will be (#6242).
//!
//! Every insert into `audit.access_log_entry` takes its transaction's xid
//! before any row draws an id: both statements that insert entries read
//! `pg_current_xact_id()` in a materialized CTE the rows are joined to.
//! A round reads the id sequence's last value `X`, then, in a statement of
//! its own, a snapshot. A transaction that drew an id at or below `X` drew it
//! before `X` was read, so it held its xid before that snapshot was taken
//! and lies below its `xmax`. Once a later snapshot's `xmin` reaches that
//! `xmax`, every such transaction has ended, so each entry at or below `X` is
//! visible to it or never will be. A long writing transaction anywhere in
//! the cluster holds `xmin` back: the frontier waits and no entry is lost,
//! since an entry is readable from its own commit, before its response. The
//! identity sequence keeps `CACHE 1`, so a value is drawn when a row takes
//! it. The sealer and the delivery read up to the frontier. No specification
//! governs it: our own design.

use std::collections::VecDeque;

use sqlx::PgConnection;

/// A sequence value read before a snapshot, with that snapshot's `xmax`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    /// The sequence's last value, read first.
    pub last_value: i64,
    /// The `xmax` of a snapshot taken after it.
    pub xmax: u64,
}

/// The marks read and not yet passed, and the frontier they reached.
#[derive(Debug, Default)]
pub struct Frontier {
    marks: VecDeque<Mark>,
    reached: i64,
}

impl Frontier {
    /// Notes `mark`.
    pub fn mark(&mut self, mark: Mark) {
        self.marks.push_back(mark);
    }

    /// The frontier once a snapshot's `xmin` is `xmin`: the last value of the
    /// newest mark whose `xmax` it has reached, and never lower than before.
    pub fn reach(&mut self, xmin: u64) -> i64 {
        while let Some(mark) = self.marks.front().copied() {
            if mark.xmax > xmin {
                break;
            }
            self.reached = self.reached.max(mark.last_value);
            self.marks.pop_front();
        }
        self.reached
    }

    /// The frontier reached so far.
    #[must_use]
    pub fn reached(&self) -> i64 {
        self.reached
    }

    /// One round on `conn`: the sequence's last value, then a snapshot in a
    /// statement of its own, whose `xmin` moves the frontier and whose
    /// `xmax` marks the value; returns the frontier.
    ///
    /// # Errors
    /// The store errors when a statement fails, and a decode error when the
    /// server answers a snapshot bound that is not a number.
    pub async fn round(&mut self, conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
        let last_value: Option<i64> = sqlx::query_scalar(
            "SELECT pg_sequence_last_value(pg_get_serial_sequence('audit.access_log_entry', 'id')::regclass)",
        )
        .fetch_one(&mut *conn)
        .await?;
        let (xmin, xmax): (String, String) = sqlx::query_as(
            "SELECT pg_snapshot_xmin(s)::text, pg_snapshot_xmax(s)::text FROM pg_current_snapshot() AS s",
        )
        .fetch_one(&mut *conn)
        .await?;
        let bound = |text: &str| {
            text.parse::<u64>()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))
        };
        let (xmin, xmax) = (bound(&xmin)?, bound(&xmax)?);
        self.mark(Mark {
            last_value: last_value.unwrap_or(0),
            xmax,
        });
        Ok(self.reach(xmin))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_mark_counts_once_every_transaction_below_its_xmax_ended() {
        let mut frontier = Frontier::default();
        frontier.mark(Mark {
            last_value: 10,
            xmax: 100,
        });
        assert_eq!(
            frontier.reach(99),
            0,
            "a transaction below 100 may still run"
        );
        frontier.mark(Mark {
            last_value: 20,
            xmax: 105,
        });
        assert_eq!(frontier.reach(100), 10);
        assert_eq!(frontier.reach(104), 10);
        assert_eq!(frontier.reach(105), 20);
        assert_eq!(frontier.reach(90), 20, "the frontier never falls");
    }

    #[test]
    fn a_snapshot_with_nothing_in_progress_passes_its_own_mark() {
        let mut frontier = Frontier::default();
        frontier.mark(Mark {
            last_value: 7,
            xmax: 50,
        });
        assert_eq!(
            frontier.reach(50),
            7,
            "xmin equal to xmax: nothing was running"
        );
    }
}
