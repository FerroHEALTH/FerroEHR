// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The store's intake and its chainer (#5747, #5745, #6242).
//!
//! A request writes each of its records in its own transaction before it
//! answers: an access the access log covers as its entry with its index row,
//! once, into `audit.access_log_entry`, which every reader, the sealer and
//! the delivery read from that row (held in `audit.access_log_hold` while its
//! write is unconfirmed), and any other record by its columns, written once
//! into `audit.audit_event` with an `audit.audit_event_intake` row naming
//! it. A record is durable once that transaction commits, and no request
//! waits on a head. The chainer releases the holds of confirmed writes and
//! moves the system intake into the system chain through
//! `audit.audit_event_chain_move`, owing each record to the sinks that
//! deliver from the store (`migrations/audit/0006_baseline_intake.sql`). No
//! openEHR spec governs audit storage: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use super::writers::Pinned;
use super::{AuditRecord, AuditStore, LearnedSets, RecordEnd};
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

/// What one round of the chainer did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChainRound {
    /// The records of confirmed writes the round released.
    pub released: u64,
    /// The records the round moved into the system chain.
    pub system: u64,
}

impl ChainRound {
    /// How many records the round released or chained.
    #[must_use]
    pub fn chained(self) -> usize {
        usize::try_from(self.released.saturating_add(self.system)).unwrap_or(usize::MAX)
    }

    /// Whether the round took a whole batch of `limit` on the system chain,
    /// so rows may be waiting behind it.
    #[must_use]
    pub fn filled(self, limit: u32) -> bool {
        self.system >= u64::from(limit)
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
        writing: &Pinned,
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
        let entered = self.with_entries(writing, records).await?;
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
        let runs = writing.runs(records);
        let digests = runs
            .iter()
            .map(|(policy, _, _)| Self::registered_digest(policy))
            .collect::<Result<Vec<_>, _>>()?;
        let mut ids = Vec::with_capacity(records.len());
        let mut learned_all = Vec::with_capacity(runs.len());
        for ((under, start, end), policy) in runs.iter().zip(digests) {
            let run = records.get(*start..*end).unwrap_or_default();
            let run_ends = ends.get(*start..*end).unwrap_or_default();
            let (tokens, learned) = Self::batch_tokens(self, under, conn, run).await?;
            ids.extend(Self::insert_rows(conn, run, run_ends, policy, tokens, reserved_by).await?);
            learned_all.push(learned);
        }
        #[cfg(feature = "fhir")]
        {
            let (uids, learned) = self
                .intake_entries(conn, writing, entries, reserved_by)
                .await?;
            ids.extend(uids);
            learned_all.push(learned);
        }
        Ok((ids, merged(learned_all), wrote))
    }

    /// The access-log store the sealer seals through: the one the server
    /// installed, which signs, or an unsigning one.
    pub(crate) fn chaining_store(&self) -> AccessLogStore {
        self.access_log().map_or_else(
            || AccessLogStore::new(self.pool.clone()),
            |installed| installed.with_pool(self.pool.clone()),
        )
    }

    /// One round of the chainer: the holds of the writes confirmed since the
    /// last round released, and up to `limit` system intake rows linked into
    /// the system chain when `look` names it or this process wrote one.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the release or the system chain's move
    /// fails; what the round did not finish is looked at again in the next.
    pub async fn chain_round(&self, limit: u32, look: Chains) -> Result<ChainRound, AuditError> {
        let written = self.pending.take();
        let released = self.releases.take();
        let look_system = look.system || written.system || !released.is_empty();
        let round = self.round(limit, look_system, &released).await;
        // NOTE: our own design (#5762), a release that failed is tried again by the next round.
        if round.is_err() && !released.is_empty() {
            self.releases.restore(released);
        }
        // NOTE: our own design (#5770), a chain the round did not empty is looked at again.
        let unfinished = match &round {
            Ok(round) => look_system && round.system >= u64::from(limit),
            Err(_) => look_system,
        };
        self.pending.mark(Chains {
            access_log: false,
            system: unfinished,
        });
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
        look_system: bool,
        release: &[Uuid],
    ) -> Result<ChainRound, AuditError> {
        let released = if release.is_empty() {
            0
        } else {
            let mut conn = acquire(&self.pool).await?;
            Self::release(&mut conn, release).await?
        };
        let moved: i64 = if look_system {
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
            released,
            system: u64::try_from(moved).unwrap_or(0),
        })
    }

    /// Chain every system intake row written before the call when `look`
    /// names the system chain, and seal every access-log entry committed
    /// before it when `look` names the access log.
    ///
    /// # Errors
    /// As [`Self::chain_round`], and [`AuditError::AccessLog`] when sealing
    /// fails.
    pub async fn settle(&self, look: Chains) -> Result<(), AuditError> {
        if look.access_log {
            self.chaining_store().seal_committed().await?;
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

    /// Release the records held under the reservations `ids` on `conn`, a
    /// connection of the store's pool: the reconciler's release of a write
    /// whose process stopped first (#5762); returns how many. Once the
    /// release commits, [`Self::mark_released`] wakes the chainer for them.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn release(conn: &mut PgConnection, ids: &[Uuid]) -> Result<u64, AuditError> {
        let released: i64 = sqlx::query_scalar("SELECT audit.audit_intake_release($1)")
            .bind(ids)
            .fetch_one(conn)
            .await?;
        Ok(u64::try_from(released).unwrap_or(0))
    }

    /// Marks both chains owed rows: records released outside the chainer's
    /// round wait to be chained.
    pub fn mark_released(&self) {
        self.pending.mark(Chains::BOTH);
    }

    /// Discard the records held under the reservations `ids`, whose writes
    /// never committed, on `conn`, a connection of the store's pool;
    /// returns how many.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the statement fails.
    pub async fn discard(conn: &mut PgConnection, ids: &[Uuid]) -> Result<u64, AuditError> {
        let discarded: i64 = sqlx::query_scalar("SELECT audit.audit_intake_discard($1)")
            .bind(ids)
            .fetch_one(conn)
            .await?;
        Ok(u64::try_from(discarded).unwrap_or(0))
    }

    /// The reservations whose records have been held longer than `grace`,
    /// each with how long, oldest first.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails.
    pub async fn held_longer_than(
        conn: &mut PgConnection,
        grace: std::time::Duration,
    ) -> Result<Vec<(Uuid, std::time::Duration)>, AuditError> {
        let rows: Vec<(Uuid, f64)> = sqlx::query_as(
            "SELECT h.reserved_by, extract(epoch FROM now() - min(h.taken_at))::float8 \
             FROM (SELECT reserved_by, taken_at FROM audit.access_log_hold \
                   UNION ALL SELECT reserved_by, taken_at FROM audit.audit_event_intake \
                   WHERE reserved_by IS NOT NULL) AS h \
             GROUP BY h.reserved_by \
             HAVING min(h.taken_at) < now() - make_interval(secs => $1) \
             ORDER BY 2 DESC",
        )
        .bind(grace.as_secs_f64())
        .fetch_all(conn)
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
    pub async fn held_among(
        conn: &mut PgConnection,
        ids: &[Uuid],
    ) -> Result<Vec<Uuid>, AuditError> {
        let held: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT h.reserved_by FROM ( \
             SELECT reserved_by FROM audit.access_log_hold WHERE reserved_by = ANY($1) \
             UNION ALL SELECT reserved_by FROM audit.audit_event_intake WHERE reserved_by = ANY($1) \
             ) AS h",
        )
        .bind(ids)
        .fetch_all(conn)
        .await?;
        Ok(held)
    }
}

#[cfg(test)]
mod tests {
    use super::ChainRound;

    /// A round that took its whole batch of the system chain may have left
    /// rows behind, so the chainer runs again at once; one that took less
    /// waits for the next round's start (#5770).
    #[test]
    fn a_round_is_filled_when_it_took_a_whole_system_batch() {
        let round = |released, system| ChainRound { released, system };
        assert!(round(0, 512).filled(512));
        assert!(!round(512, 3).filled(512));
        assert!(!round(511, 511).filled(512));
    }
}
