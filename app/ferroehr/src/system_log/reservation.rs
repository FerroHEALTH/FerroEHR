// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The one commit choke point of a request-scoped write, and the durable
//! reservation of its audit records (#5762).
//!
//! The audit domain lives apart from the clinical and party domains
//! (`DeploymentGap::SharedCluster`), so a write's records cannot join the
//! write's own transaction. [`FerroEhrService::commit_recorded`] writes them
//! first, into the audit intake held under a reservation id, then inserts
//! the id into the write's domain (`audit_reservation`) in the write's own
//! transaction, and commits. The adapter releases the held records once the
//! response is built ([`FerroEhrService::confirm_reserved`]); a process that
//! died between the commit and the release leaves the reservation, which the
//! reconciler of any replica resolves (`sender::reconciler`): the write
//! committed, so its records are released, exactly as written, once; a
//! reservation that never committed discards them. This is the durable
//! reservation of `docs/plans/access-log.md` rule 3; no openEHR spec governs
//! it: our own design.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::event::AuditEvent;
use super::recorder::{Reserved, WriteFacts, WriteRecorder};
use crate::db::domain::Domain;
use crate::error::ServiceError;
use crate::error::refusal::Refusal;
use crate::service::FerroEhrService;

/// The statement that reserves a write's records in its own transaction;
/// the pool's search path names the domain's schema.
const RESERVE: &str = "INSERT INTO audit_reservation (id) VALUES ($1) ON CONFLICT (id) DO NOTHING";

/// The statement that removes a confirmed reservation.
const CONFIRMED: &str = "DELETE FROM audit_reservation WHERE id = ANY($1)";

impl FerroEhrService {
    /// Commit `tx`, the transaction of a request-scoped write in `domain`,
    /// with the write's audit records reserved: the operation record the
    /// request's recorder builds from `facts`, and `records`, the act's own
    /// records, each stamped with the caller.
    ///
    /// The records are written into the audit intake held under the
    /// request's reservation before the commit, and the reservation joins
    /// the commit, so a committed write always has its records. Outside a
    /// request's recorder the reservation is confirmed at once.
    ///
    /// # Errors
    /// [`ServiceError::Unrecorded`] when the records cannot be written, and
    /// the database error of a failed reservation or commit; nothing is
    /// committed then.
    pub(crate) async fn commit_recorded(
        &self,
        mut tx: Transaction<'_, Postgres>,
        domain: Domain,
        facts: &WriteFacts,
        records: Vec<AuditEvent>,
    ) -> Result<(), ServiceError> {
        let Some(sender) = self.audit.as_ref().filter(|sender| sender.enabled()) else {
            tx.commit().await?;
            return Ok(());
        };
        let recorder = super::recorder::current();
        let mut events = recorder
            .as_ref()
            .map(|recorder| recorder.records_for(facts))
            .unwrap_or_default();
        let built = !events.is_empty();
        let deferred = recorder
            .as_ref()
            .map(WriteRecorder::take_deferred)
            .unwrap_or_default();
        for mut record in records.into_iter().chain(deferred) {
            super::stamp_caller(&mut record);
            events.push(record);
        }
        if events.is_empty() {
            tx.commit().await?;
            return Ok(());
        }
        let unbuild = || {
            if let (Some(recorder), true) = (&recorder, built) {
                recorder.unbuild();
            }
        };
        let reservation = Uuid::now_v7();
        if let Err(unwritten) = Box::pin(sender.record_held(events, reservation)).await {
            unbuild();
            return Err(ServiceError::Unrecorded(
                Refusal::new((), "audit trail unavailable").with_source(unwritten),
            ));
        }
        let committed = async {
            sqlx::query(RESERVE)
                .bind(reservation)
                .execute(&mut *tx)
                .await?;
            tx.commit().await
        }
        .await;
        if let Err(error) = committed {
            unbuild();
            // NOTE: our own design (#5762), a held record of a write that never committed
            // records no access; the reconciler discards it when this fails.
            let _discarded = sender.store().discard(&[reservation]).await;
            return Err(error.into());
        }
        let reserved = Reserved {
            id: reservation,
            domain,
        };
        match &recorder {
            Some(recorder) => recorder.committed(reserved),
            None => self.confirm_reserved(&[reserved]).await,
        }
        Ok(())
    }

    /// Whether a write of the running request has its audit records reserved
    /// at its commit: the audit trail is on and a protocol adapter primed the
    /// request's recorder.
    pub(crate) fn records_writes(&self) -> bool {
        self.audit
            .as_ref()
            .is_some_and(super::sender::AuditSender::enabled)
            && super::recorder::current().is_some()
    }

    /// Release the records held under `reserved`, whose writes committed,
    /// and remove the reservations; a failure is logged, and the reconciler
    /// releases them later.
    pub async fn confirm_reserved(&self, reserved: &[Reserved]) {
        let Some(sender) = self.audit.as_ref() else {
            return;
        };
        if reserved.is_empty() {
            return;
        }
        let ids: Vec<Uuid> = reserved.iter().map(|reserved| reserved.id).collect();
        if let Err(error) = sender.release(&ids).await {
            tracing::warn!(
                error = %crate::error::ErrorChain::new(&error),
                "the held audit records of a committed write were not released; the reconciler \
                 releases them"
            );
            return;
        }
        for domain in [Domain::Clinical, Domain::Party] {
            let ids: Vec<Uuid> = reserved
                .iter()
                .filter(|reserved| reserved.domain == domain)
                .map(|reserved| reserved.id)
                .collect();
            if ids.is_empty() {
                continue;
            }
            let pool = match domain {
                Domain::Party => &self.demographic_pool,
                Domain::Clinical | Domain::Audit => &self.pool,
            };
            if let Err(error) = sqlx::query(CONFIRMED).bind(&ids).execute(pool).await {
                tracing::warn!(
                    error = %crate::error::ErrorChain::new(&error),
                    "a confirmed audit reservation was not removed; the reconciler removes it"
                );
            }
        }
    }
}
