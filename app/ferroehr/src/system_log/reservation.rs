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
//! transaction, and commits. Once the response is built the adapter hands the
//! committed reservations to the chainer ([`FerroEhrService::confirm_reserved`]),
//! whose next round releases their records inside its own transaction, so a
//! write costs two commits. A process that died before that round leaves the
//! reservation, which the reconciler of any replica resolves
//! (`sender::reconciler`): the write committed, so its records are released,
//! exactly as written, once; a reservation that never committed discards
//! them; a spent one is removed. This is the durable reservation of
//! `docs/plans/access-log.md` rule 3; no openEHR spec governs it: our own
//! design.

use sqlx::{Postgres, Transaction};
use uuid::Uuid;

use super::event::AuditEvent;
use super::recorder::{Reserved, WriteFacts, WriteRecorder};
use crate::error::ServiceError;
use crate::error::refusal::Refusal;
use crate::service::FerroEhrService;

/// The statement that reserves a write's records in its own transaction;
/// the pool's search path names the domain's schema.
const RESERVE: &str = "INSERT INTO audit_reservation (id) VALUES ($1) ON CONFLICT (id) DO NOTHING";

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
        if let Err(unwritten) = Box::pin(sender.record(events, Some(reservation))).await {
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
            let _discarded = async {
                let mut conn = crate::db::acquire::acquire(&sender.store().pool).await?;
                super::store::AuditStore::discard(&mut conn, &[reservation]).await
            }
            .await;
            return Err(error.into());
        }
        let reserved = Reserved { id: reservation };
        match &recorder {
            Some(recorder) => recorder.committed(reserved),
            None => self.confirm_reserved(&[reserved]),
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

    /// Hand the reservations `reserved`, whose writes committed, to the
    /// chainer, whose next round releases their records in its own
    /// transaction; the spent reservations are left to the reconciler, which
    /// also releases the records of a process that stops first.
    pub fn confirm_reserved(&self, reserved: &[Reserved]) {
        let Some(sender) = self.audit.as_ref() else {
            return;
        };
        let ids: Vec<Uuid> = reserved.iter().map(|reserved| reserved.id).collect();
        sender.release_committed(&ids);
    }
}
