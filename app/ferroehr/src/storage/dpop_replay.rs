// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `DPoP` proof replay store every replica shares: the `ferrohealth-smart`
//! [`ReplayStore`] over the clinical `dpop_proof` table.
//!
//! RFC 9449 §11.1 has a resource server accept each proof once within the
//! window it accepts proofs in, and notes that a single-use check "may not
//! always be feasible in practice, e.g., when multiple servers behind a single
//! endpoint have no shared state". The replicas of one instance share the
//! clinical pool, so one insert there decides for all of them: a proof whose
//! record stands is a replay on any replica. A record past the end of its
//! window may be replaced, since the proof check refuses a proof that old
//! before it asks the store, and every [`SWEEP_EVERY`]th record sweeps up to
//! [`SWEEP_BATCH`] ended records, so the table holds about one window of
//! proofs and an idle server does no work for it. A store that cannot answer
//! fails the request with `503` (the crate's `consume`), so a database fault
//! never admits a proof. No openEHR spec governs authentication: our own
//! design/extension.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::SystemTime;

use ferrohealth_smart::sender::replay::{ProofId, ReplayStore, ReplayStoreError};
use sqlx::PgPool;

use crate::service::FerroEhrService;

/// How many records one replica writes between two sweeps.
pub const SWEEP_EVERY: u64 = 64;

/// The most ended records one sweep deletes.
pub const SWEEP_BATCH: i64 = 512;

/// Why the replay store could not answer.
#[derive(Debug, thiserror::Error)]
pub enum DpopReplayError {
    /// The end of the proof's window is not an instant `PostgreSQL` holds.
    #[error("the end of the proof's window is out of range")]
    Instant(#[source] jiff::Error),
    /// The database refused or could not be reached.
    #[error("the DPoP replay store's database failed")]
    Database(#[source] sqlx::Error),
}

/// The [`ReplayStore`] over the clinical `dpop_proof` table.
#[derive(Debug)]
pub struct PgReplayStore {
    /// The clinical pool every replica shares.
    pool: PgPool,
    /// The records this replica wrote, which paces its sweeps.
    recorded: AtomicU64,
}

impl PgReplayStore {
    /// Returns the store over the `dpop_proof` table of `service`'s clinical
    /// pool, the pool every replica of the instance shares.
    #[must_use]
    pub fn of(service: &FerroEhrService) -> Self {
        Self {
            pool: service.pool.clone(),
            recorded: AtomicU64::new(0),
        }
    }

    /// Records `proof` until `until` and returns whether it was unseen: one
    /// insert that either writes the record, replaces a record whose window
    /// ended, or finds a standing record and writes nothing.
    ///
    /// # Errors
    /// [`DpopReplayError::Instant`] for an `until` outside the range of an
    /// instant, and [`DpopReplayError::Database`] when the insert or the sweep
    /// fails.
    pub async fn record(
        &self,
        proof: &ProofId,
        until: SystemTime,
    ) -> Result<bool, DpopReplayError> {
        let until = jiff::Timestamp::try_from(until).map_err(DpopReplayError::Instant)?;
        let first: Option<bool> = sqlx::query_scalar(
            "INSERT INTO dpop_proof (proof, expires_at) VALUES ($1, $2) \
             ON CONFLICT (proof) DO UPDATE SET expires_at = EXCLUDED.expires_at \
             WHERE dpop_proof.expires_at < now() \
             RETURNING true",
        )
        .bind(proof.as_str())
        .bind(jiff_sqlx::Timestamp::from(until))
        .fetch_optional(&self.pool)
        .await
        .map_err(DpopReplayError::Database)?;
        if self
            .recorded
            .fetch_add(1, Ordering::Relaxed)
            .is_multiple_of(SWEEP_EVERY)
        {
            self.sweep().await?;
        }
        Ok(first.is_some())
    }

    /// Deletes up to [`SWEEP_BATCH`] records whose window ended, skipping the
    /// rows another replica's sweep holds, and returns how many it deleted.
    ///
    /// # Errors
    /// [`DpopReplayError::Database`] when the delete fails.
    pub async fn sweep(&self) -> Result<u64, DpopReplayError> {
        let deleted = sqlx::query(
            "DELETE FROM dpop_proof WHERE proof IN ( \
                 SELECT proof FROM dpop_proof WHERE expires_at < now() \
                 ORDER BY expires_at LIMIT $1 FOR UPDATE SKIP LOCKED)",
        )
        .bind(SWEEP_BATCH)
        .execute(&self.pool)
        .await
        .map_err(DpopReplayError::Database)?
        .rows_affected();
        Ok(deleted)
    }
}

impl ReplayStore for PgReplayStore {
    fn first_use<'a>(
        &'a self,
        proof: &'a ProofId,
        until: SystemTime,
    ) -> Pin<Box<dyn Future<Output = Result<bool, ReplayStoreError>> + Send + 'a>> {
        Box::pin(async move {
            self.record(proof, until)
                .await
                .map_err(ReplayStoreError::new)
        })
    }
}
