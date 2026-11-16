// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The chainer of the access log: the intake rows linked into the chain in
//! one transaction that holds the head only for what needs it (#5745).
//!
//! A round locks the head without waiting, so any replica may run it and one
//! does at a time; reads the oldest intake rows' content digests; computes
//! each link in Rust; writes the records, their index and outbox rows and the
//! new head through `audit.access_log_chain_move`, whose insert trigger checks
//! every link against the head; ends with the signed checkpoint of the last
//! link; and commits. The checkpoint is signed on the blocking pool while the
//! records are written. No openEHR spec governs the chain's concurrency: our
//! own design.

use std::time::{Duration, Instant};

use sqlx::Connection;

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
    /// Chain up to `limit` of the oldest intake rows, owing each to the
    /// sinks `owed` names.
    ///
    /// # Errors
    /// The store errors when a statement fails, a link the trigger refuses
    /// among them, and the sealing errors of the checkpoint.
    pub async fn chain_round(&self, limit: u32, owed: Owed) -> Result<Round, AccessLogError> {
        let mut conn = acquire(self.pool()).await?;
        let mut tx = conn.begin().await?;
        let head: Option<(i64, String)> =
            sqlx::query_as("SELECT head_seq, head_digest FROM audit.access_log_chain_take()")
                .fetch_optional(&mut *tx)
                .await?;
        let Some((mut sequence, mut previous)) = head else {
            return Ok(Round::Busy);
        };
        let taken = Instant::now();
        let rows: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, encode(content_digest, 'hex') FROM audit.access_log_intake \
             WHERE reserved_by IS NULL ORDER BY id LIMIT $1",
        )
        .bind(i64::from(limit))
        .fetch_all(&mut *tx)
        .await?;
        if rows.is_empty() {
            tx.rollback().await?;
            return Ok(Round::Chained {
                entries: 0,
                held: taken.elapsed(),
            });
        }
        let mut ids = Vec::with_capacity(rows.len());
        let mut sequences = Vec::with_capacity(rows.len());
        let mut previous_digests = Vec::with_capacity(rows.len());
        let mut digests = Vec::with_capacity(rows.len());
        for (id, content_digest) in &rows {
            sequence = sequence.checked_add(1).ok_or(AccessLogError::Exhausted)?;
            let digest = link_digest(&previous, sequence, content_digest);
            ids.push(*id);
            sequences.push(sequence);
            previous_digests.push(std::mem::replace(&mut previous, digest.clone()));
            digests.push(digest);
        }
        let seal = self.begin_seal(sequence, &previous)?;
        sqlx::query("SELECT audit.access_log_chain_move($1, $2, $3, $4, $5, $6)")
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
            match self.chain_round(SETTLE_BATCH, owed).await? {
                Round::Busy => tokio::time::sleep(Duration::from_millis(1)).await,
                Round::Chained { entries: 0, .. } => return Ok(()),
                Round::Chained { .. } => {}
            }
        }
    }
}

/// How many intake rows one round of [`AccessLogStore::settle`] chains.
const SETTLE_BATCH: u32 = 512;
