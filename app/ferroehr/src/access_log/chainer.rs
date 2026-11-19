// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The chainer of the access log: the entries the intake rows name linked
//! into the chain in one transaction that holds the head only for what needs
//! it (#5745).
//!
//! A round locks the head without waiting, so any replica may run it and one
//! does at a time; reads the entries of the oldest intake rows with their
//! content digests; computes each link in Rust; writes the links, the outbox
//! rows and the new head through `audit.access_log_chain_move`, whose insert
//! trigger checks the links against the head, and which removes the intake
//! rows (#5968); ends with the signed checkpoint of the last
//! link; and commits. The checkpoint is signed on the blocking pool while the
//! records are written. No openEHR spec governs the chain's concurrency: our
//! own design.

use std::time::{Duration, Instant};

use sqlx::Connection;
use uuid::Uuid;

use crate::access_log::chain::link_digest;
use crate::access_log::store::{AccessLogError, AccessLogStore};
use crate::db::acquire::acquire;

/// The sinks that deliver from the store, owed each record a round chains.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Owed {
    /// The ITI-20 ATX: FHIR Feed.
    pub fhir_feed: bool,
    /// The ITI-20 syslog route.
    pub syslog: bool,
}

/// Whether a round seals the last link it writes in a signed checkpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seal {
    /// The round ends with the checkpoint of its last link.
    Now,
    /// The round leaves its links to a later checkpoint
    /// ([`AccessLogStore::seal_head`] or a later round's).
    Later,
}

/// What one round of the chainer did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Round {
    /// Another chainer held the head, so this one did nothing.
    Busy,
    /// The round chained this many entries, and held the head this long.
    Chained {
        /// The entries chained.
        entries: usize,
        /// How long the round held the head.
        held: Duration,
    },
}

impl AccessLogStore {
    /// Chain the entries of the oldest intake rows, whole rows until `limit`
    /// entries are reached, owing each to the
    /// sinks `owed` names, after releasing in the same transaction the
    /// records held under the reservations `release`, whose writes committed
    /// (#5762); a round that finds the head held releases nothing.
    ///
    /// # Errors
    /// The store errors when a statement fails, a link the trigger refuses
    /// among them, and the sealing errors of the checkpoint.
    pub async fn chain_round(
        &self,
        limit: u32,
        owed: Owed,
        seal: Seal,
        release: &[Uuid],
    ) -> Result<Round, AccessLogError> {
        let mut conn = acquire(self.pool()).await?;
        let mut tx = conn.begin().await?;
        // NOTE: PostgreSQL 18 §19.7.1 (enable_seqscan); the intake's deleted rows leave it many
        // pages for few live rows, so the planner scanned 266 MB and sorted under the head where
        // the primary key reads the oldest rows in 0.4 ms (#5770).
        sqlx::query("SET LOCAL enable_seqscan = off")
            .execute(&mut *tx)
            .await?;
        let head: Option<(i64, String)> =
            sqlx::query_as("SELECT head_seq, head_digest FROM audit.access_log_chain_take()")
                .fetch_optional(&mut *tx)
                .await?;
        let Some((mut sequence, mut previous)) = head else {
            return Ok(Round::Busy);
        };
        let taken = Instant::now();
        if !release.is_empty() {
            sqlx::query("SELECT audit.audit_intake_release($1)")
                .bind(release)
                .execute(&mut *tx)
                .await?;
        }
        let rows: Vec<(i64, i64, String)> = sqlx::query_as(INTAKE_ROWS)
            .bind(i64::from(limit))
            .fetch_all(&mut *tx)
            .await?;
        if rows.is_empty() {
            // NOTE: our own design (#6050), a round that seals and chains nothing seals the head a
            // stopped chainer left to a later checkpoint.
            let sealed =
                seal == Seal::Now && self.seal_unsealed(&mut tx, sequence, &previous).await?;
            if sealed || !release.is_empty() {
                // NOTE: our own design (#5762), the audit domain's own store, whose held rows this
                // transaction released and whose head it sealed.
                tx.commit().await?;
            } else {
                tx.rollback().await?;
            }
            return Ok(Round::Chained {
                entries: 0,
                held: taken.elapsed(),
            });
        }
        let mut batches: Vec<i64> = Vec::new();
        let mut ids = Vec::with_capacity(rows.len());
        let mut sequences = Vec::with_capacity(rows.len());
        let mut previous_digests = Vec::with_capacity(rows.len());
        let mut digests = Vec::with_capacity(rows.len());
        for (batch, id, content_digest) in &rows {
            if batches.last() != Some(batch) {
                batches.push(*batch);
            }
            sequence = sequence.checked_add(1).ok_or(AccessLogError::Exhausted)?;
            let digest = link_digest(&previous, sequence, content_digest);
            ids.push(*id);
            sequences.push(sequence);
            previous_digests.push(std::mem::replace(&mut previous, digest.clone()));
            digests.push(digest);
        }
        let seal = match seal {
            Seal::Now => self.begin_seal(sequence, &previous)?,
            Seal::Later => None,
        };
        sqlx::query("SELECT audit.access_log_chain_move($1, $2, $3, $4, $5, $6, $7)")
            .bind(&batches)
            .bind(&ids)
            .bind(&sequences)
            .bind(&previous_digests)
            .bind(&digests)
            .bind(owed.fhir_feed)
            .bind(owed.syslog)
            .execute(&mut *tx)
            .await?;
        if let Some(seal) = seal {
            seal.write(&mut tx).await?;
        }
        // NOTE: our own design (#5762), the audit domain's own store, whose records are this transaction's
        // rows.
        tx.commit().await?;
        Ok(Round::Chained {
            entries: rows.len(),
            held: taken.elapsed(),
        })
    }

    /// Chain every intake row written before the call, waiting out another
    /// chainer that holds the head.
    ///
    /// # Errors
    /// As [`Self::chain_round`].
    pub async fn settle(&self, owed: Owed) -> Result<(), AccessLogError> {
        loop {
            match self.chain_round(SETTLE_BATCH, owed, Seal::Now, &[]).await? {
                Round::Busy => tokio::time::sleep(Duration::from_millis(1)).await,
                Round::Chained { entries: 0, .. } => return Ok(()),
                Round::Chained { .. } => {}
            }
        }
    }
}

impl AccessLogStore {
    /// Seal the head of the chain in a signed checkpoint, unless one seals it
    /// already; `false` when another chainer holds the head, which seals its
    /// own rounds.
    ///
    /// # Errors
    /// The store errors when a statement fails, and the sealing errors of the
    /// checkpoint.
    pub async fn seal_head(&self) -> Result<bool, AccessLogError> {
        let mut conn = acquire(self.pool()).await?;
        let mut tx = conn.begin().await?;
        let head: Option<(i64, String)> =
            sqlx::query_as("SELECT head_seq, head_digest FROM audit.access_log_chain_take()")
                .fetch_optional(&mut *tx)
                .await?;
        let Some((sequence, digest)) = head else {
            return Ok(false);
        };
        self.seal_unsealed(&mut tx, sequence, &digest).await?;
        // NOTE: our own design (#5762), the audit domain's own store, sealing the head no request
        // moved.
        tx.commit().await?;
        Ok(true)
    }

    /// Seal the head `digest` at `sequence`, which `tx` holds, unless the log
    /// is empty or a checkpoint seals it already; whether it wrote one.
    async fn seal_unsealed(
        &self,
        tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
        sequence: i64,
        digest: &str,
    ) -> Result<bool, AccessLogError> {
        if sequence == 0 {
            return Ok(false);
        }
        let sealed: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM audit.access_log_checkpoint WHERE sequence = $1)",
        )
        .bind(sequence)
        .fetch_one(&mut **tx)
        .await?;
        if sealed {
            return Ok(false);
        }
        let Some(seal) = self.begin_seal(sequence, digest)? else {
            return Ok(false);
        };
        seal.write(tx).await?;
        Ok(true)
    }
}

/// The entries of the oldest intake rows that are not held, each with its
/// intake row and content digest, in the order they are linked: whole rows,
/// as long as the entries before a row number fewer than `$1`.
const INTAKE_ROWS: &str = "SELECT b.id, n.id, encode(e.content_digest, 'hex') \
     FROM (SELECT i.id, i.entries, \
                  sum(cardinality(i.entries)) OVER (ORDER BY i.id) - cardinality(i.entries) AS before \
             FROM audit.access_log_intake i WHERE i.reserved_by IS NULL ORDER BY i.id LIMIT $1) AS b \
     CROSS JOIN LATERAL unnest(b.entries) WITH ORDINALITY AS n(id, k) \
     JOIN audit.access_log_entry e ON e.id = n.id \
     WHERE b.before < $1 ORDER BY b.id, n.k";

/// How many entries one round of [`AccessLogStore::settle`] chains, rounded
/// up to whole intake rows.
const SETTLE_BATCH: u32 = 512;
