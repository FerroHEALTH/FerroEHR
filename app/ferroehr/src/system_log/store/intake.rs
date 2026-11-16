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
//! deliver from the store (`migrations/audit/0041_audit_intake.sql`). No
//! openEHR spec governs audit storage: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use super::{AuditRecord, AuditStore, LearnedSets, RecordEnd};
use crate::access_log::chainer::Round;
use crate::access_log::store::AccessLogStore;
use crate::db::acquire::acquire;
use crate::system_log::AuditError;

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
    ) -> Result<(Vec<Uuid>, LearnedSets), AuditError> {
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
        let runs = self.writing.runs(records);
        let mut digests = Vec::with_capacity(runs.len());
        for (writing, _, _) in &runs {
            digests.push(self.writing_digest(writing).await?);
        }
        let mut ids = Vec::with_capacity(records.len());
        let mut learned_all = Vec::with_capacity(runs.len());
        for ((writing, start, end), policy) in runs.iter().zip(digests) {
            let run = records.get(*start..*end).unwrap_or_default();
            let run_ends = ends.get(*start..*end).unwrap_or_default();
            let (tokens, learned) = Self::batch_tokens(self, writing, conn, run).await?;
            ids.extend(Self::insert_rows(conn, run, run_ends, policy, tokens).await?);
            learned_all.push(learned);
        }
        #[cfg(feature = "fhir")]
        {
            let (uids, learned) = self.intake_entries(conn, entries).await?;
            ids.extend(uids);
            learned_all.push(learned);
        }
        Ok((ids, merged(learned_all)))
    }

    /// The access-log store the chainer links entries through: the one the
    /// server installed, which seals, or an unsealing one.
    pub(super) fn chaining_store(&self) -> AccessLogStore {
        self.access_log().map_or_else(
            || AccessLogStore::new(self.pool.clone()),
            |installed| installed.with_pool(self.pool.clone()),
        )
    }

    /// One round of the chainer: up to `limit` intake rows of each chain
    /// linked into it.
    ///
    /// # Errors
    /// [`AuditError::AccessLog`] when the access log's round fails, and
    /// [`AuditError::Store`] when the system chain's move fails.
    pub async fn chain_round(&self, limit: u32) -> Result<ChainRound, AuditError> {
        let access_log = self.chaining_store().chain_round(limit, self.owed).await?;
        let moved: i64 = sqlx::query_scalar("SELECT audit.audit_event_chain_move($1, $2, $3)")
            .bind(i32::try_from(limit).unwrap_or(i32::MAX))
            .bind(self.owed.fhir_feed)
            .bind(self.owed.syslog)
            .fetch_one(&mut *acquire(&self.pool).await?)
            .await?;
        Ok(ChainRound {
            access_log,
            system: u64::try_from(moved).unwrap_or(0),
        })
    }

    /// Chain every intake row written before the call into its chain,
    /// waiting out another chainer that holds the access log's head.
    ///
    /// # Errors
    /// As [`Self::chain_round`].
    pub async fn settle(&self) -> Result<(), AuditError> {
        self.chaining_store().settle(self.owed).await?;
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
    /// How long the oldest record of either intake has waited for the
    /// chainer; `None` when both are empty.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails.
    pub async fn intake_age(&self) -> Result<Option<std::time::Duration>, AuditError> {
        let seconds: Option<f64> = sqlx::query_scalar(
            "SELECT extract(epoch FROM now() - least( \
             (SELECT min(taken_at) FROM audit.access_log_intake), \
             (SELECT min(taken_at) FROM audit.audit_event_intake)))::float8",
        )
        .fetch_one(&mut *acquire(&self.pool).await?)
        .await?;
        Ok(seconds.map(|seconds| std::time::Duration::from_secs_f64(seconds.max(0.0))))
    }
}
