// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The store's write and the release of its holds (#5747, #5745, #6242).
//!
//! A request writes each of its records in its own transaction before it
//! answers: an access the access log covers as its entry with its index row,
//! once, into `audit.access_log_entry`, and any other record by its columns,
//! once, into `audit.audit_event`, each with its content digest. Every
//! reader, the sealer and the delivery read that row; a record a write holds
//! (`audit.access_log_hold`, `audit.audit_event_hold`) while its commit is
//! unconfirmed is in none of them. A record is durable once that transaction
//! commits, and no request waits on a head. The chainer's round releases the
//! holds of confirmed writes (`migrations/audit/0006_baseline_intake.sql`).
//! No openEHR spec governs audit storage: our own design.

use sqlx::PgConnection;
use uuid::Uuid;

use super::writers::Pinned;
use super::{AuditRecord, AuditStore, RecordEnd};
use crate::access_log::store::AccessLogStore;
use crate::db::acquire::acquire;
use crate::system_log::AuditError;

/// Which logs a write wrote records into, or a settle must seal.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Chains {
    /// The access log.
    pub access_log: bool,
    /// The system records.
    pub system: bool,
}

impl Chains {
    /// Both logs.
    pub const BOTH: Self = Self {
        access_log: true,
        system: true,
    };
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

impl AuditStore {
    /// Write `records` into the intake on `conn`, each with its retention end
    /// (`ends`, one per record, or empty); returns each record's id, the
    /// entry's uid for an access the log covers. The caller's transaction,
    /// or a lone system statement's own, commits them.
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
    ) -> Result<(Vec<Uuid>, Chains), AuditError> {
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
        for ((under, start, end), policy) in runs.iter().zip(digests) {
            let run = records.get(*start..*end).unwrap_or_default();
            let run_ends = ends.get(*start..*end).unwrap_or_default();
            let tokens = Self::batch_tokens(under, run).await?;
            ids.extend(Self::insert_rows(conn, run, run_ends, policy, tokens, reserved_by).await?);
        }
        #[cfg(feature = "fhir")]
        {
            ids.extend(
                self.intake_entries(conn, writing, entries, reserved_by)
                    .await?,
            );
        }
        Ok((ids, wrote))
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
    /// last round released; returns how many records it released.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the release fails; the next round tries the
    /// reservations again.
    pub async fn chain_round(&self) -> Result<u64, AuditError> {
        let released = self.releases.take();
        if released.is_empty() {
            return Ok(0);
        }
        let round = async {
            let mut conn = acquire(&self.pool).await?;
            Self::release(&mut conn, &released).await
        }
        .await;
        // NOTE: our own design (#5762), a release that failed is tried again by the next round.
        if round.is_err() {
            self.releases.restore(released);
        }
        round
    }

    /// Whether a confirmed write waits for the next round to release it.
    #[must_use]
    pub fn is_pending(&self) -> bool {
        !self.releases.lock().is_empty()
    }

    /// Seal every record committed before the call and held by no write in
    /// the logs `look` names.
    ///
    /// # Errors
    /// [`AuditError::AccessLog`] when sealing fails.
    pub async fn settle(&self, look: Chains) -> Result<(), AuditError> {
        if look.access_log {
            self.chaining_store()
                .seal_committed(&crate::access_log::seal::ACCESS_LOG)
                .await?;
        }
        if look.system {
            self.chaining_store()
                .seal_committed(&crate::access_log::seal::SYSTEM)
                .await?;
        }
        Ok(())
    }
}

impl AuditStore {
    /// How long the oldest record of either log that no write holds has
    /// waited past the last sealed window; `None` when there is none.
    ///
    /// # Errors
    /// [`AuditError::Store`] when the read fails.
    pub async fn unsealed_age(&self) -> Result<Option<std::time::Duration>, AuditError> {
        let seconds: Option<f64> = sqlx::query_scalar(
            "SELECT extract(epoch FROM now() - least( \
             (SELECT e.stored_at FROM audit.audit_event e \
               WHERE e.seq > (SELECT s.last_id FROM audit.audit_event_seal_state s WHERE s.singleton) \
                 AND NOT EXISTS (SELECT 1 FROM audit.audit_event_hold h WHERE h.seq = e.seq) \
               ORDER BY e.seq LIMIT 1), \
             (SELECT e.time_recorded FROM audit.access_log_entry e \
               WHERE e.id > (SELECT s.last_id FROM audit.access_log_seal_state s WHERE s.singleton) \
                 AND NOT EXISTS (SELECT 1 FROM audit.access_log_hold h WHERE h.entry_id = e.id) \
               ORDER BY e.id LIMIT 1)))::float8",
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
    }

    /// Release the records held under the reservations `ids` on `conn`, a
    /// connection of the store's pool: the reconciler's release of a write
    /// whose process stopped first (#5762); returns how many. Once the
    /// release commits, the records are in every read.
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
                   UNION ALL SELECT reserved_by, taken_at FROM audit.audit_event_hold) AS h \
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
    /// Which of the reservations `ids` still hold records.
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
             UNION ALL SELECT reserved_by FROM audit.audit_event_hold WHERE reserved_by = ANY($1) \
             ) AS h",
        )
        .bind(ids)
        .fetch_all(conn)
        .await?;
        Ok(held)
    }
}
