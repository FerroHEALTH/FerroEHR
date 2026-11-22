// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The frontier of the access log: the highest entry id at or below which
//! every entry is committed or never will be (#6242).
//!
//! Every insert into `audit.access_log_entry` takes its transaction's xid
//! before any row draws an id: both statements that insert entries read
//! `pg_current_xact_id()` in a materialized CTE the rows are joined to. A
//! round reads the id sequence's last value `X`, then takes an xid of its
//! own in a statement of its own, which commits at once. A transaction that
//! drew an id at or below `X` drew it before `X` was read, so it held its xid
//! before the round took its own, and xids are assigned in increasing order:
//! its xid is below the round's. Once a later snapshot's `xmin` is above the
//! round's xid, every such transaction has ended, so each entry at or below
//! `X` is visible to it or never will be. A snapshot's `xmax` would not do: it
//! is one past the newest xid that has ended, and a transaction still running
//! can hold an xid at or above it (PostgreSQL 19, "Transaction ID and
//! Snapshot Information Functions", `pg_current_snapshot`). Each round
//! spends one xid. A long writing transaction anywhere in the cluster holds
//! `xmin` back: the frontier waits and no entry is lost, since an entry is
//! readable from its own commit, before its response. The identity sequence
//! keeps `CACHE 1`, so a value is drawn when a row takes it. The sealer and
//! the delivery read up to the frontier. No specification governs it: our
//! own design.

use std::collections::VecDeque;

use sqlx::PgConnection;

/// A sequence value with the xid a round took after reading it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    /// The sequence's last value, read first.
    pub last_value: i64,
    /// One past the xid the round took after it: a snapshot whose `xmin`
    /// reaches it shows every transaction that drew a value up to
    /// `last_value` ended.
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

    /// One round on `conn`, outside a transaction: the sequence's last value,
    /// then an xid of the round's own in a statement that commits at once,
    /// which marks the value, then a snapshot whose `xmin` moves the frontier;
    /// returns the frontier.
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
        let taken: String = sqlx::query_scalar("SELECT pg_current_xact_id()::text")
            .fetch_one(&mut *conn)
            .await?;
        let xmin: String =
            sqlx::query_scalar("SELECT pg_snapshot_xmin(pg_current_snapshot())::text")
                .fetch_one(&mut *conn)
                .await?;
        let bound = |text: &str| {
            text.parse::<u64>()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))
        };
        let (taken, xmin) = (bound(&taken)?, bound(&xmin)?);
        self.mark(Mark {
            last_value: last_value.unwrap_or(0),
            xmax: taken.saturating_add(1),
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
