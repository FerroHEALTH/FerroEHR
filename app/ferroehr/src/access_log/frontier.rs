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
    marked: Option<i64>,
    owed: i64,
    sealed: i64,
    xids: u64,
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

    /// How many xids the rounds took: one for each round that found the
    /// sequence moved, none for a round over an idle log.
    #[must_use]
    pub fn xids_taken(&self) -> u64 {
        self.xids
    }

    /// Whether the frontier moved past the entries already owed to the
    /// sinks.
    #[must_use]
    pub fn owe_due(&self) -> bool {
        self.reached > self.owed
    }

    /// Notes that every entry up to the frontier is owed to the sinks.
    pub fn owed(&mut self) {
        self.owed = self.reached;
    }

    /// Whether the frontier moved past the entries already sealed.
    #[must_use]
    pub fn seal_due(&self) -> bool {
        self.reached > self.sealed
    }

    /// Notes that every entry up to the frontier is sealed.
    pub fn sealed(&mut self) {
        self.sealed = self.reached;
    }

    /// One round on `conn`, outside a transaction, in one statement that
    /// commits at once: the sequence's last value, then, when it moved since
    /// the last mark, an xid of the round's own, which marks the value, and
    /// the `xmin` of the statement's snapshot, which moves the frontier past
    /// earlier marks; returns the frontier. An idle log takes no xid.
    ///
    /// # Errors
    /// The store errors when a statement fails, and a decode error when the
    /// server answers a snapshot bound that is not a number.
    pub async fn round(&mut self, conn: &mut PgConnection) -> Result<i64, sqlx::Error> {
        // NOTE: our own design (#6242), the xid is taken after the value is read: the CASE
        // reads the CTE's row, so the scan that reads the value runs first.
        let (last_value, taken, xmin): (Option<i64>, Option<String>, String) = sqlx::query_as(
            "WITH x AS MATERIALIZED (SELECT pg_sequence_last_value( \
                 pg_get_serial_sequence('audit.access_log_entry', 'id')::regclass) AS v) \
             SELECT x.v, \
                    CASE WHEN x.v IS DISTINCT FROM $1 THEN pg_current_xact_id()::text END, \
                    pg_snapshot_xmin(pg_current_snapshot())::text \
               FROM x",
        )
        .bind(self.marked)
        .fetch_one(&mut *conn)
        .await?;
        let bound = |text: &str| {
            text.parse::<u64>()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))
        };
        let xmin = bound(&xmin)?;
        let reached = self.reach(xmin);
        if let (Some(last_value), Some(taken)) = (last_value, taken) {
            self.xids = self.xids.saturating_add(1);
            self.marked = Some(last_value);
            self.mark(Mark {
                last_value,
                xmax: bound(&taken)?.saturating_add(1),
            });
        }
        Ok(reached)
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
