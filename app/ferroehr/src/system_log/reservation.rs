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
//!
//! A recorded write takes its connections in one fixed order (#5964): the
//! audit connection its records are written on first ([`RecordedConn`]), then
//! the connection of its own domain, so its transaction never waits for the
//! audit pool and no two requests wait for each other across the pools
//! (`crate::db::acquire` refuses the other order in a debug build).

use std::ops::{Deref, DerefMut};

use sqlx::postgres::PgConnection;
use sqlx::{Connection, PgPool, Postgres, Transaction};
use uuid::Uuid;

use super::event::AuditEvent;
use super::recorder::{Reserved, WriteFacts, WriteRecorder};
use crate::db::acquire::{AcquireError, Checkout, acquire};
use crate::error::ServiceError;
use crate::error::refusal::Refusal;
use crate::service::FerroEhrService;

/// The connections of a recorded write: the audit connection its records
/// are written on, taken first while the audit trail is on, and the
/// connection of the write's own domain. It derefs to the domain's
/// connection.
#[derive(Debug)]
pub(crate) struct RecordedConn {
    audit: Option<Checkout>,
    conn: Checkout,
}

impl RecordedConn {
    /// Opens the write's transaction on the domain's connection.
    ///
    /// # Errors
    /// The database error of a failed `BEGIN`.
    pub(crate) async fn begin(&mut self) -> Result<RecordedTx<'_>, sqlx::Error> {
        let Self { audit, conn } = self;
        Ok(RecordedTx {
            tx: conn.begin().await?,
            audit: audit.as_deref_mut(),
        })
    }
}

impl Deref for RecordedConn {
    type Target = PgConnection;

    fn deref(&self) -> &PgConnection {
        &self.conn
    }
}

impl DerefMut for RecordedConn {
    fn deref_mut(&mut self) -> &mut PgConnection {
        &mut self.conn
    }
}

/// The transaction of a recorded write, with the audit connection
/// [`FerroEhrService::commit_recorded`] writes its records on. It derefs to
/// the transaction's connection.
#[derive(Debug)]
pub(crate) struct RecordedTx<'c> {
    tx: Transaction<'c, Postgres>,
    audit: Option<&'c mut PgConnection>,
}

impl RecordedTx<'_> {
    /// Rolls the write back; its records were never written.
    ///
    /// # Errors
    /// The database error of a failed `ROLLBACK`.
    pub(crate) async fn rollback(self) -> Result<(), sqlx::Error> {
        self.tx.rollback().await
    }
}

impl Deref for RecordedTx<'_> {
    type Target = PgConnection;

    fn deref(&self) -> &PgConnection {
        &self.tx
    }
}

impl DerefMut for RecordedTx<'_> {
    fn deref_mut(&mut self) -> &mut PgConnection {
        &mut self.tx
    }
}

/// The records a write holds under its reservation before its statement
/// ([`FerroEhrService::hold_records`]).
#[derive(Debug)]
pub(crate) struct Held {
    /// The reservation the write's statement inserts beside its rows.
    pub(crate) reservation: Uuid,
    /// Whether the request's operation record is among the records.
    built: bool,
    /// The act's own records among them, handed back if the write fails.
    deferred: Vec<AuditEvent>,
}

/// The statement that reserves a write's records in its own transaction;
/// the pool's search path names the domain's schema.
const RESERVE: &str = "INSERT INTO audit_reservation (id) VALUES ($1) ON CONFLICT (id) DO NOTHING";

/// Insert `reservation` into the write's domain on `conn`, for a write that
/// holds records and commits no guarded statement of its own (an identical
/// re-send the opt-in rule answers).
///
/// # Errors
/// The database error of the insert.
pub(crate) async fn reserve(conn: &mut PgConnection, reservation: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query(RESERVE)
        .bind(reservation)
        .execute(conn)
        .await
        .map(drop)
}

impl FerroEhrService {
    /// The connections of a write on `pool` whose records
    /// [`Self::commit_recorded`] reserves: an audit connection first while
    /// the audit trail is on, then one of `pool`.
    ///
    /// # Errors
    /// [`AcquireError`] when either pool fails.
    pub(crate) async fn recorded_conn(&self, pool: &PgPool) -> Result<RecordedConn, AcquireError> {
        let audit = match self.audit.as_ref().filter(|sender| sender.enabled()) {
            Some(sender) => Some(acquire(&sender.store().pool).await?),
            None => None,
        };
        Ok(RecordedConn {
            audit,
            conn: acquire(pool).await?,
        })
    }

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
        tx: RecordedTx<'_>,
        facts: &WriteFacts,
        records: Vec<AuditEvent>,
    ) -> Result<(), ServiceError> {
        let RecordedTx { mut tx, audit } = tx;
        let (Some(sender), Some(audit)) =
            (self.audit.as_ref().filter(|sender| sender.enabled()), audit)
        else {
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
        if let Err(error) = sender.learn_subjects(&mut tx, &events).await {
            unbuild();
            return Err(error.into());
        }
        let reservation = Uuid::now_v7();
        if let Err(unwritten) = Box::pin(sender.record_on(audit, events, reservation)).await {
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
            let _discarded = super::store::AuditStore::discard(audit, &[reservation]).await;
            return Err(error.into());
        }
        let reserved = Reserved { id: reservation };
        match &recorder {
            Some(recorder) => recorder.committed(reserved),
            None => self.confirm_reserved(&[reserved]),
        }
        Ok(())
    }

    /// Write the running request's operation record built from `facts`, with
    /// the act's deferred records, into the audit intake on `audit` under a
    /// fresh reservation, before the write's statement runs (#6256).
    ///
    /// The intake is durable when this returns, so the reservation the
    /// write's own statement inserts beside its rows can only commit after
    /// its records are (#5747, #5986). `clinical` is the write's connection,
    /// on which the record subjects not yet known are read; it holds no
    /// transaction. `None` when nothing is recorded: the audit trail is off,
    /// no adapter primed the request, or no record is due.
    ///
    /// # Errors
    /// [`ServiceError::Unrecorded`] when the records cannot be written, and
    /// the database error of the subjects' read; nothing is held then.
    pub(crate) async fn hold_records(
        &self,
        conn: &mut RecordedConn,
        facts: &WriteFacts,
    ) -> Result<Option<Held>, ServiceError> {
        let RecordedConn {
            audit,
            conn: clinical,
        } = conn;
        let (Some(sender), Some(audit)) = (
            self.audit.as_ref().filter(|sender| sender.enabled()),
            audit.as_deref_mut(),
        ) else {
            return Ok(None);
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
        events.extend(deferred.iter().cloned().map(|mut record| {
            super::stamp_caller(&mut record);
            record
        }));
        if events.is_empty() {
            return Ok(None);
        }
        let held = Held {
            reservation: Uuid::now_v7(),
            built,
            deferred,
        };
        if let Err(error) = sender.learn_subjects(&mut *clinical, &events).await {
            Self::unhold(&held);
            return Err(error.into());
        }
        if let Err(unwritten) = Box::pin(sender.record_on(audit, events, held.reservation)).await {
            Self::unhold(&held);
            return Err(ServiceError::Unrecorded(
                Refusal::new((), "audit trail unavailable").with_source(unwritten),
            ));
        }
        Ok(Some(held))
    }

    /// Discard at once the records `held` holds, whose write did not commit:
    /// they record no access, and the request's next write or its adapter
    /// records them instead. A discard that fails leaves them to the
    /// reconciler, which discards a reservation that never committed.
    pub(crate) async fn discard_held(&self, conn: &mut RecordedConn, held: &Held) {
        Self::unhold(held);
        let Some(audit) = conn.audit.as_deref_mut() else {
            return;
        };
        // NOTE: our own design (#5762), a held record of a write that never committed
        // records no access; the reconciler discards it when this fails.
        let _discarded = super::store::AuditStore::discard(audit, &[held.reservation]).await;
    }

    /// Hand `held`, whose write committed with its reservation, to the
    /// request's recorder, or confirm it at once outside a request.
    pub(crate) fn written_held(&self, held: &Held) {
        let reserved = Reserved {
            id: held.reservation,
        };
        match super::recorder::current() {
            Some(recorder) => recorder.committed(reserved),
            None => self.confirm_reserved(&[reserved]),
        }
    }

    /// Hand the request's operation record and deferred records back to its
    /// recorder after their write did not commit.
    fn unhold(held: &Held) {
        if let Some(recorder) = super::recorder::current() {
            if held.built {
                recorder.unbuild();
            }
            if !held.deferred.is_empty() {
                recorder.defer(held.deferred.clone());
            }
        }
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
