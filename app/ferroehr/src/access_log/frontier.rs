// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The frontier of each audit log (#6242).
//!
//! The frontier is the highest id at or below which every record of the log
//! is committed or never will be. The access log's ids are
//! `audit.access_log_entry.id`, the system records' positions
//! `audit.audit_event.seq`.
//!
//! Every insert into either table takes its transaction's xid before any row
//! draws an id: each statement that inserts records reads
//! `pg_current_xact_id()` in a materialized CTE the rows are joined to. A
//! round reads each id sequence's last value `X`, then takes an xid of its
//! own in the same statement, which commits at once. A transaction that drew
//! an id at or below `X` drew it before `X` was read, so it held its xid
//! before the round took its own, and xids are assigned in increasing order:
//! its xid is below the round's. Once a later snapshot's `xmin` is above the
//! round's xid, every such transaction has ended, so each record at or below
//! `X` is visible to it or never will be. A snapshot's `xmax` would not do: it
//! is one past the newest xid that has ended, and a transaction still running
//! can hold an xid at or above it (PostgreSQL 19, "Transaction ID and
//! Snapshot Information Functions", `pg_current_snapshot`). A round takes an
//! xid only when a sequence moved since the last mark, so idle logs spend
//! none. A long writing transaction anywhere in the cluster holds `xmin`
//! back: the frontier waits and no record is lost, since a record is
//! readable from its own commit, before its response. The identity sequences
//! keep `CACHE 1`, so a value is drawn when a row takes it. The sealer and
//! the delivery read up to the frontier. No specification governs it: our
//! own design.

use std::collections::VecDeque;

use sqlx::PgConnection;

use crate::system_log::store::chains::AuditChain;

/// A value of each log's id sequence.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Reached {
    /// The access log's entry id.
    pub access_log: i64,
    /// The system records' position.
    pub system: i64,
}

impl Reached {
    /// Each log's value, the higher of `self`'s and `other`'s.
    #[must_use]
    pub fn max(self, other: Self) -> Self {
        Self {
            access_log: self.access_log.max(other.access_log),
            system: self.system.max(other.system),
        }
    }

    /// The value of `chain`.
    #[must_use]
    pub fn of(self, chain: AuditChain) -> i64 {
        match chain {
            AuditChain::AccessLog => self.access_log,
            AuditChain::System => self.system,
        }
    }

    /// `self` with the value of `chain` set to `value`.
    fn with(mut self, chain: AuditChain, value: i64) -> Self {
        match chain {
            AuditChain::AccessLog => self.access_log = value,
            AuditChain::System => self.system = value,
        }
        self
    }
}

/// The sequences' values with the xid a round took after reading them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Mark {
    /// The sequences' last values, read first.
    pub last_value: Reached,
    /// One past the xid the round took after them: a snapshot whose `xmin`
    /// reaches it shows every transaction that drew a value up to
    /// `last_value` ended.
    pub xmax: u64,
}

/// The marks read and not yet passed, the frontier they reached, and how
/// far each log is owed to the sinks and sealed.
#[derive(Debug, Default)]
pub struct Frontier {
    marks: VecDeque<Mark>,
    reached: Reached,
    marked: (Option<i64>, Option<i64>),
    owed: Reached,
    sealed: Reached,
    xids: u64,
}

impl Frontier {
    /// Notes `mark`.
    pub fn mark(&mut self, mark: Mark) {
        self.marks.push_back(mark);
    }

    /// The frontier once a snapshot's `xmin` is `xmin`: the last values of the
    /// newest mark whose `xmax` it has reached, and never lower than before.
    pub fn reach(&mut self, xmin: u64) -> Reached {
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
    pub fn reached(&self) -> Reached {
        self.reached
    }

    /// How many xids the rounds took: one for each round that found a
    /// sequence moved, none for a round over idle logs.
    #[must_use]
    pub fn xids_taken(&self) -> u64 {
        self.xids
    }

    /// Whether the frontier of `chain` moved past the records already owed
    /// to the sinks.
    #[must_use]
    pub fn owe_due(&self, chain: AuditChain) -> bool {
        self.reached.of(chain) > self.owed.of(chain)
    }

    /// Notes that every record of `chain` up to the frontier is owed to the
    /// sinks.
    pub fn owed(&mut self, chain: AuditChain) {
        self.owed = self.owed.with(chain, self.reached.of(chain));
    }

    /// Whether the frontier of `chain` moved past the records already
    /// sealed.
    #[must_use]
    pub fn seal_due(&self, chain: AuditChain) -> bool {
        self.reached.of(chain) > self.sealed.of(chain)
    }

    /// Notes that every record of `chain` up to the frontier is sealed.
    pub fn sealed(&mut self, chain: AuditChain) {
        self.sealed = self.sealed.with(chain, self.reached.of(chain));
    }

    /// One round on `conn`, outside a transaction, in one statement that
    /// commits at once: the sequences' last values, then, when either moved
    /// since the last mark, an xid of the round's own, which marks the
    /// values, and the `xmin` of the statement's snapshot, which moves the
    /// frontier past earlier marks; returns the frontier. Idle logs take no
    /// xid.
    ///
    /// # Errors
    /// The store errors when a statement fails, and a decode error when the
    /// server answers a snapshot bound that is not a number.
    pub async fn round(&mut self, conn: &mut PgConnection) -> Result<Reached, sqlx::Error> {
        // NOTE: our own design (#6242), the xid is taken after the values are read: the CASE
        // reads the CTE's row, so the scan that reads the values runs first.
        let (access_log, system, taken, xmin): (Option<i64>, Option<i64>, Option<String>, String) =
            sqlx::query_as(ROUND)
                .bind(self.marked.0)
                .bind(self.marked.1)
                .fetch_one(&mut *conn)
                .await?;
        let bound = |text: &str| {
            text.parse::<u64>()
                .map_err(|e| sqlx::Error::Decode(Box::new(e)))
        };
        let xmin = bound(&xmin)?;
        let reached = self.reach(xmin);
        if let Some(taken) = taken {
            let last_value = Reached {
                access_log: access_log.unwrap_or(0),
                system: system.unwrap_or(0),
            };
            self.xids = self.xids.saturating_add(1);
            self.marked = (access_log, system);
            self.mark(Mark {
                last_value,
                xmax: bound(&taken)?.saturating_add(1),
            });
        }
        Ok(reached)
    }
}

/// One round: each log's last id, the round's own xid when either moved from
/// `$1` (the access log) or `$2` (the system records), the last ids marked,
/// and the `xmin` of the statement's snapshot.
const ROUND: &str = "WITH x AS MATERIALIZED (SELECT \
       pg_sequence_last_value(pg_get_serial_sequence('audit.access_log_entry', 'id')::regclass) \
         AS a, \
       pg_sequence_last_value(pg_get_serial_sequence('audit.audit_event', 'seq')::regclass) \
         AS s) \
     SELECT x.a, x.s, \
            CASE WHEN x.a IS DISTINCT FROM $1::bigint OR x.s IS DISTINCT FROM $2::bigint \
                 THEN pg_current_xact_id()::text END, \
            pg_snapshot_xmin(pg_current_snapshot())::text \
       FROM x";

/// The last value of each log's id sequence, `NULL` before the first.
pub const LAST_VALUES: &str = "SELECT \
     pg_sequence_last_value(pg_get_serial_sequence('audit.access_log_entry', 'id')::regclass), \
     pg_sequence_last_value(pg_get_serial_sequence('audit.audit_event', 'seq')::regclass)";

#[cfg(test)]
mod tests {
    use super::*;

    fn at(id: i64) -> Reached {
        Reached {
            access_log: id,
            system: id.saturating_mul(2),
        }
    }

    #[test]
    fn a_mark_counts_once_every_transaction_below_its_xmax_ended() {
        let mut frontier = Frontier::default();
        frontier.mark(Mark {
            last_value: at(10),
            xmax: 100,
        });
        assert_eq!(
            frontier.reach(99),
            at(0),
            "a transaction below 100 may still run"
        );
        frontier.mark(Mark {
            last_value: at(20),
            xmax: 105,
        });
        assert_eq!(frontier.reach(100), at(10));
        assert_eq!(frontier.reach(104), at(10));
        assert_eq!(frontier.reach(105), at(20));
        assert_eq!(frontier.reach(90), at(20), "the frontier never falls");
    }

    #[test]
    fn a_snapshot_with_nothing_in_progress_passes_its_own_mark() {
        let mut frontier = Frontier::default();
        frontier.mark(Mark {
            last_value: at(7),
            xmax: 50,
        });
        assert_eq!(
            frontier.reach(50),
            at(7),
            "xmin equal to xmax: nothing was running"
        );
    }

    #[test]
    fn each_log_is_owed_and_sealed_apart() {
        let mut frontier = Frontier::default();
        frontier.mark(Mark {
            last_value: at(3),
            xmax: 10,
        });
        frontier.reach(10);
        assert!(frontier.owe_due(AuditChain::AccessLog));
        frontier.owed(AuditChain::AccessLog);
        assert!(!frontier.owe_due(AuditChain::AccessLog));
        assert!(
            frontier.owe_due(AuditChain::System),
            "the system records still owed"
        );
        frontier.sealed(AuditChain::System);
        assert!(!frontier.seal_due(AuditChain::System));
        assert!(frontier.seal_due(AuditChain::AccessLog));
    }
}
