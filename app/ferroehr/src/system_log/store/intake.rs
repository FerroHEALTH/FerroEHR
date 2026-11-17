// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The store's intake and its chainer (#5747, #5745).
//!
//! A request writes each of its records into the intake in its own
//! transaction before it answers: an access the access log covers as its
//! entry with its index row (`audit.access_log_intake`), any other record by
//! its columns (`audit.audit_event_intake`). A record is durable once that
//! transaction commits, and no request waits on the head of a chain. The
//! chainer moves the intake into the chains: the access log through
//! [`AccessLogStore::chain_round`], the system chain through
//! `audit.audit_event_chain_move`, each owing the record to the sinks that
//! deliver from the store (`migrations/audit/0042_audit_intake.sql`). No
//! openEHR spec governs audit storage: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use super::{AuditRecord, AuditStore, LearnedSets, RecordEnd};
use crate::access_log::chainer::{Round, Seal};
use crate::access_log::store::AccessLogStore;
use crate::db::acquire::acquire;
use crate::system_log::AuditError;

/// Which chains a write gave intake rows, or a chainer round must look at.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Chains {
    /// The access log.
    pub access_log: bool,
    /// The system chain.
    pub system: bool,
}

impl Chains {
    /// Both chains: a look for the rows other replicas wrote.
    pub const BOTH: Self = Self {
        access_log: true,
        system: true,
    };
}

/// The chains this process wrote intake rows into since the chainer's last
/// round, so a round skips the transaction of a chain nobody wrote.
#[derive(Debug, Default)]
pub(super) struct Pending {
    access_log: std::sync::atomic::AtomicBool,
    system: std::sync::atomic::AtomicBool,
}

impl Pending {
    /// Note the chains a committed write gave rows.
    pub(super) fn mark(&self, wrote: Chains) {
        use std::sync::atomic::Ordering;
        if wrote.access_log {
            self.access_log.store(true, Ordering::Release);
        }
        if wrote.system {
            self.system.store(true, Ordering::Release);
        }
    }

    /// Whether any chain is marked.
    fn is_marked(&self) -> bool {
        use std::sync::atomic::Ordering;
        self.access_log.load(Ordering::Acquire) || self.system.load(Ordering::Acquire)
    }

    /// The chains written since the last take, cleared.
    fn take(&self) -> Chains {
        use std::sync::atomic::Ordering;
        Chains {
            access_log: self.access_log.swap(false, Ordering::AcqRel),
            system: self.system.swap(false, Ordering::AcqRel),
        }
    }
}

/// The reservations whose writes committed, waiting for the chainer's next
/// round to release their records inside its own transaction (#5762).
#[derive(Debug, Default)]
pub(super) struct Releases(std::sync::Mutex<Vec<Uuid>>);

impl Releases {
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Uuid>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Queue `ids` for the next round.
    pub(super) fn queue(&self, ids: &[Uuid]) {
        self.lock().extend_from_slice(ids);
    }

    /// The queued ids, cleared.
    fn take(&self) -> Vec<Uuid> {
        std::mem::take(&mut *self.lock())
    }

    /// Put back `ids` a round did not release.
    fn restore(&self, ids: Vec<Uuid>) {
        self.lock().extend(ids);
    }
}

/// What one round of the chainer did on each chain.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainRound {
    /// The access log's round.
    pub access_log: Round,
    /// The records the round moved into the system chain.
    pub system: u64,
}

impl ChainRound {
    /// Whether the round chained anything on either chain.
    #[must_use]
    pub fn chained(self) -> usize {
        let entries = match self.access_log {
            Round::Chained { entries, .. } => entries,
            Round::Busy => 0,
        };
        entries.saturating_add(usize::try_from(self.system).unwrap_or(usize::MAX))
    }

    /// Whether the round took a whole batch of `limit` on either chain, so
    /// rows may be waiting behind it.
    #[must_use]
    pub fn filled(self, limit: u32) -> bool {
        let entries = match self.access_log {
            Round::Chained { entries, .. } => u64::try_from(entries).unwrap_or(u64::MAX),
            Round::Busy => 0,
        };
        entries >= u64::from(limit) || self.system >= u64::from(limit)
    }
}

impl AuditStore {
    /// Write `records` into the intake on `conn`, each with its retention end
    /// (`ends`, one per record, or empty); returns each record's id, the
    /// entry's uid for an access the log covers, and the shared token sets
    /// learned. The caller's transaction commits them.
    ///
    /// # Errors
    /// [`AuditError::RetentionEnds`] when `ends` is neither empty nor one end
    /// per record, [`AuditError::Store`] when a statement fails, and
    /// [`AuditError::AccessLog`] when an entry has no stored form.
    pub(super) async fn take_in(
        &self,
        conn: &mut PgConnection,
        records: Vec<AuditRecord>,
        ends: &[RecordEnd],
        reserved_by: Option<Uuid>,
    ) -> Result<(Vec<Uuid>, LearnedSets, Chains), AuditError> {
        if !ends.is_empty() && ends.len() != records.len() {
            return Err(AuditError::RetentionEnds {
                ends: ends.len(),
                records: records.len(),
            });
        }
        #[cfg(feature = "fhir")]
        let entered = self.with_entries(records).await?;
        #[cfg(feature = "fhir")]
        let (records, row_ends, entries) = super::entries::split(entered, ends);
        #[cfg(feature = "fhir")]
        let ends = row_ends.as_slice();
        let records = records.as_slice();
        #[cfg(feature = "fhir")]
        let wrote = Chains {
            access_log: !entries.is_empty(),
            system: !records.is_empty(),
        };
        #[cfg(not(feature = "fhir"))]
        let wrote = Chains {
            access_log: false,
            system: !records.is_empty(),
        };
        let runs = self.writing.runs(records);
        let mut digests = Vec::with_capacity(runs.len());
        for (writing, _, _) in &runs {
            digests.push(self.writing_digest(writing, Some(&mut *conn)).await?);
        }
        let mut ids = Vec::with_capacity(records.len());
        let mut learned_all = Vec::with_capacity(runs.len());
        for ((writing, start, end), policy) in runs.iter().zip(digests) {
            let run = records.get(*start..*end).unwrap_or_default();
            let run_ends = ends.get(*start..*end).unwrap_or_default();
            let (tokens, learned) = Self::batch_tokens(self, writing, conn, run).await?;
            ids.extend(Self::insert_rows(conn, run, run_ends, policy, tokens, reserved_by).await?);
            learned_all.push(learned);
        }
        #[cfg(feature = "fhir")]
        {
            let (uids, learned) = self.intake_entries(conn, entries, reserved_by).await?;
            ids.extend(uids);
            learned_all.push(learned);
        }
        Ok((ids, merged(learned_all), wrote))
    }

    /// The access-log store the chainer links entries through: the one the
    /// server installed, which seals, or an unsealing one.
    pub(crate) fn chaining_store(&self) -> AccessLogStore {
        self.access_log().map_or_else(
            || AccessLogStore::new(self.pool.clone()),
            |installed| installed.with_pool(self.pool.clone()),
        )
    }

    /// One round of the chainer: up to `limit` intake rows linked into each
    /// chain of `look`, and of each chain this process wrote since the last
    /// round; a chain neither names costs no transaction.
    ///
    /// # Errors
    /// [`AuditError::AccessLog`] when the access log's round fails, and
    /// [`AuditError::Store`] when the system chain's move fails; the chains
    /// the round did not finish are looked at again in the next.
    pub async fn chain_round(
        &self,
        limit: u32,
        look: Chains,
        seal: Seal,
    ) -> Result<ChainRound, AuditError> {
        let written = self.pending.take();
        let released = self.releases.take();
        let releasing = !released.is_empty();
        let look = Chains {
            access_log: look.access_log || written.access_log || releasing,
            system: look.system || written.system || releasing,
        };
        let round = self.round(limit, look, seal, &released).await;
        // NOTE: our own design (#5762), a round that did not take the head released nothing.
        if releasing && !matches!(&round, Ok(round) if round.access_log != Round::Busy) {
            self.releases.restore(released);
            self.pending.mark(Chains::BOTH);
        }
        // NOTE: our own design (#5770), a chain the round did not empty is looked at again.
        let unfinished = match &round {
            Ok(round) => Chains {
                access_log: look.access_log
                    && match round.access_log {
                        Round::Busy => true,
                        Round::Chained { entries, .. } => {
                            u32::try_from(entries).unwrap_or(u32::MAX) >= limit
                        }
                    },
                system: look.system && round.system >= u64::from(limit),
            },
            Err(_) => look,
        };
        self.pending.mark(unfinished);
        round
    }

    /// Whether a chain is left for the next round: one this process wrote
    /// since the last, or one a round did not empty.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        self.pending.is_marked()
    }

    async fn round(
        &self,
        limit: u32,
        look: Chains,
        seal: Seal,
        release: &[Uuid],
    ) -> Result<ChainRound, AuditError> {
        let access_log = if look.access_log {
            self.chaining_store()
                .chain_round(limit, self.owed, seal, release)
                .await?
        } else {
            Round::Chained {
                entries: 0,
                held: std::time::Duration::ZERO,
            }
        };
        let moved: i64 = if look.system {
            sqlx::query_scalar("SELECT audit.audit_event_chain_move($1, $2, $3)")
                .bind(i32::try_from(limit).unwrap_or(i32::MAX))
                .bind(self.owed.fhir_feed)
                .bind(self.owed.syslog)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?
        } else {
            0
        };
        Ok(ChainRound {
            access_log,
            system: u64::try_from(moved).unwrap_or(0),
        })
    }

    /// Chain every intake row of the chains `look` names written before the
    /// call, waiting out another chainer that holds the access log's head;
    /// a write settles the chains it wrote and leaves the other to its
    /// chainer.
    ///
    /// # Errors
    /// As [`Self::chain_round`].
    pub async fn settle(&self, look: Chains) -> Result<(), AuditError> {
        if look.access_log {
            self.chaining_store().settle(self.owed).await?;
        }
        if !look.system {
            return Ok(());
        }
        loop {
            let moved: i64 = sqlx::query_scalar("SELECT audit.audit_event_chain_move($1, $2, $3)")
                .bind(SETTLE_BATCH)
                .bind(self.owed.fhir_feed)
                .bind(self.owed.syslog)
                .fetch_one(&mut *acquire(&self.pool).await?)
                .await?;
            if moved == 0 {
                return Ok(());
            }
        }
    }
}

/// How many system-intake rows one move of [`AuditStore::settle`] takes.
const SETTLE_BATCH: i32 = 512;

/// The shared sets of every part of one write.
#[cfg(feature = "fhir")]
fn merged(parts: Vec<LearnedSets>) -> LearnedSets {
    parts
        .into_iter()
        .fold(LearnedSets::default(), LearnedSets::merge)
}

/// A slim build learns no set.
#[cfg(not(feature = "fhir"))]
fn merged(_parts: Vec<LearnedSets>) -> LearnedSets {}

impl AuditStore {
    /// How long the oldest record of either intake that is not held has
    /// waited for the chainer; `None` when there is none.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails.
    pub async fn intake_age(&self) -> Result<Option<std::time::Duration>, AuditError> {
        let seconds: Option<f64> = sqlx::query_scalar(
            "SELECT extract(epoch FROM now() - least( \
             (SELECT min(taken_at) FROM audit.access_log_intake WHERE reserved_by IS NULL), \
             (SELECT min(taken_at) FROM audit.audit_event_intake WHERE reserved_by IS NULL)))::float8",
        )
        .fetch_one(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(seconds.map(|seconds| std::time::Duration::from_secs_f64(seconds.max(0.0))))
    }
}

impl AuditStore {
    /// Queue the reservations `ids`, whose writes committed, for the
    /// chainer's next round, which releases their records in its own
    /// transaction (#5762).
    pub fn queue_release(&self, ids: &[Uuid]) {
        self.releases.queue(ids);
        self.pending.mark(Chains::BOTH);
    }

    /// Release the records held under the reservations `ids` now, in a
    /// transaction of their own: the reconciler's release of a write whose
    /// process stopped first (#5762); returns how many.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn release(&self, ids: &[Uuid]) -> Result<u64, AuditError> {
        let released: i64 = sqlx::query_scalar("SELECT audit.audit_intake_release($1)")
            .bind(ids)
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        self.pending.mark(Chains::BOTH);
        Ok(u64::try_from(released).unwrap_or(0))
    }

    /// Discard the records held under the reservations `ids`, whose writes
    /// never committed; returns how many.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn discard(&self, ids: &[Uuid]) -> Result<u64, AuditError> {
        let discarded: i64 = sqlx::query_scalar("SELECT audit.audit_intake_discard($1)")
            .bind(ids)
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        Ok(u64::try_from(discarded).unwrap_or(0))
    }

    /// The reservations whose records have been held longer than `grace`,
    /// each with how long, oldest first.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails.
    pub async fn held_longer_than(
        &self,
        grace: std::time::Duration,
    ) -> Result<Vec<(Uuid, std::time::Duration)>, AuditError> {
        let rows: Vec<(Uuid, f64)> = sqlx::query_as(
            "SELECT h.reserved_by, extract(epoch FROM now() - min(h.taken_at))::float8 \
             FROM (SELECT reserved_by, taken_at FROM audit.access_log_intake \
                   WHERE reserved_by IS NOT NULL \
                   UNION ALL SELECT reserved_by, taken_at FROM audit.audit_event_intake \
                   WHERE reserved_by IS NOT NULL) AS h \
             GROUP BY h.reserved_by \
             HAVING min(h.taken_at) < now() - make_interval(secs => $1) \
             ORDER BY 2 DESC",
        )
        .bind(grace.as_secs_f64())
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(rows
            .into_iter()
            .map(|(id, age)| (id, std::time::Duration::from_secs_f64(age.max(0.0))))
            .collect())
    }
}

impl AuditStore {
    /// Which of the reservations `ids` still hold records in the intake.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails.
    pub async fn held_among(&self, ids: &[Uuid]) -> Result<Vec<Uuid>, AuditError> {
        let held: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT h.reserved_by FROM ( \
             SELECT reserved_by FROM audit.access_log_intake WHERE reserved_by = ANY($1) \
             UNION ALL SELECT reserved_by FROM audit.audit_event_intake WHERE reserved_by = ANY($1) \
             ) AS h",
        )
        .bind(ids)
        .fetch_all(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(held)
    }
}

#[cfg(test)]
mod tests {
    use super::ChainRound;
    use crate::access_log::chainer::Round;

    /// A round that took its whole batch on either chain may have left rows
    /// behind, so the chainer runs again at once; one that took less waits
    /// for the next round's start (#5770).
    #[test]
    fn a_round_is_filled_when_either_chain_took_its_batch() {
        let round = |entries, system| ChainRound {
            access_log: Round::Chained {
                entries,
                held: std::time::Duration::ZERO,
            },
            system,
        };
        assert!(round(512, 0).filled(512));
        assert!(round(3, 512).filled(512));
        assert!(!round(511, 511).filled(512));
        let busy = ChainRound {
            access_log: Round::Busy,
            system: 0,
        };
        assert!(!busy.filled(512));
    }
}
