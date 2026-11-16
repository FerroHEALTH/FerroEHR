// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The apply of stored ITI-93 messages: one at a time, in arrival order, by
//! one replica under a lease, each as one demographic CONTRIBUTION.
//!
//! The applier claims the oldest message still to apply with a lease, locks
//! its row, and in that one transaction commits the contribution, records
//! the merges and bindings, lifts the message's splits and marks it applied,
//! so a crash at any point leaves it to apply again and a message applies
//! once. The contribution is committed as the feed's client: under its policy
//! grants and with its committer, so the commit checks hold it to what that
//! client may write. A message that cannot be applied as its content states
//! is refused, keeps its splits, and is logged; the next one applies. No
//! specification governs the apply: our own design (PMIR 1.6.0
//! §2:3.93.4.1.3).

mod work;

use std::sync::Arc;
use std::time::Duration;

use sqlx::Connection as _;
use uuid::Uuid;

use super::plan::Resolved;
use super::receive::CHANNEL;
use super::{FeedEvent, FeedSender, Refusal};
use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::party_binding::grants::capabilities::{Actor, with_capabilities};
use crate::service::FerroEhrService;
use crate::storage::error::StorageError;
use crate::system_log::event::EventActionCode;
use crate::versioning::audit::change_type;
use crate::versioning::change;
use crate::versioning::committer::{CommitterIdentity, with_committer};
use work::Work;

/// How long a claim holds a message before another replica may take it.
const LEASE_SECONDS: f64 = 60.0;

/// The longest the applier waits between two looks at the queue.
const IDLE: Duration = Duration::from_secs(10);

/// The transaction-level advisory lock an apply holds, and an operator's
/// retry takes, so the two never interleave ("pmia" in ASCII).
pub(super) const APPLY_LOCK: i64 = 0x706d_6961;

/// What one turn of the applier did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Applied {
    /// The message was applied.
    Applied(String),
    /// The message was refused and keeps its splits.
    Refused(String, Refusal),
}

/// Why a message did not apply on this turn.
#[derive(Debug, thiserror::Error)]
pub(super) enum ApplyError {
    /// It cannot be applied as it states.
    #[error(transparent)]
    Refused(Refusal),
    /// The store or a concurrent write stopped it; it applies on a later turn.
    #[error(transparent)]
    Transient(ServiceError),
}

impl From<StorageError> for ApplyError {
    fn from(error: StorageError) -> Self {
        Self::Transient(error.into())
    }
}

impl From<sqlx::Error> for ApplyError {
    fn from(error: sqlx::Error) -> Self {
        Self::Transient(ServiceError::from(StorageError::from(error)))
    }
}

impl From<Resolved> for ApplyError {
    fn from(error: Resolved) -> Self {
        match error {
            Resolved::Storage(error) => error.into(),
            Resolved::Refused(refusal) => Self::Refused(refusal),
        }
    }
}

impl From<ServiceError> for ApplyError {
    fn from(error: ServiceError) -> Self {
        match error {
            ServiceError::Storage(_)
            | ServiceError::Database(_)
            | ServiceError::Acquire(_)
            | ServiceError::VersionConflict(_) => Self::Transient(error),
            other => {
                tracing::warn!(
                    error = %crate::error::ErrorChain::new(&other),
                    "the contribution of an ITI-93 message is refused by the commit checks"
                );
                Self::Refused(Refusal::Commit)
            }
        }
    }
}

/// A `pmir_message` row as a claim reads it.
type ClaimRow = (
    String,
    i64,
    String,
    String,
    sqlx::types::Json<Vec<FeedEvent>>,
);

/// A message claimed for apply.
#[derive(Debug)]
pub(super) struct Claimed {
    /// Its `MessageHeader.id`.
    pub(super) message_id: String,
    sequence: i64,
    sender: FeedSender,
    /// Its changes, in history order.
    pub(super) events: Vec<FeedEvent>,
}

impl FerroEhrService {
    /// Spawns the applier of this replica, `replica`, which applies the
    /// stored messages as they arrive.
    pub fn spawn_pmir_applier(self: &Arc<Self>, replica: Uuid) {
        tokio::spawn(applier(Arc::clone(self), replica));
    }

    /// Claims the oldest stored message still to apply for `replica` and
    /// applies it; `None` when none is due or another replica holds it.
    ///
    /// # Errors
    /// [`ServiceError`] when the store does not answer or a concurrent write
    /// stops the apply; the message applies on a later turn.
    pub async fn pmir_apply_next(&self, replica: Uuid) -> Result<Option<Applied>, ServiceError> {
        let Some(message) = self.claim(replica).await? else {
            return Ok(None);
        };
        if let Err(error) = self.grants.refresh_if_stale().await {
            self.release(&message.message_id).await?;
            return Err(ServiceError::exception(format!(
                "the policy grants could not be read: {error}"
            )));
        }
        let sender = &message.sender;
        let capabilities = Arc::new(self.grants.capabilities(
            Actor::Client {
                issuer: &sender.issuer,
                client_id: &sender.client_id,
            },
            jiff::Timestamp::now(),
        ));
        let committer = CommitterIdentity {
            subject: sender.client_id.clone(),
            id_type: "oauth2",
            issuer: Some(sender.issuer.clone()),
            client: Some(sender.client_id.clone()),
            person: false,
            party: None,
            representative: false,
            bound: None,
            system: true,
            agent: capabilities.client().map(|client| client.party),
            external: None,
        };
        let applied = Box::pin(with_committer(
            Some(committer),
            with_capabilities(capabilities, Box::pin(self.apply_message(&message))),
        ))
        .await;
        match applied {
            Ok(()) => Ok(Some(Applied::Applied(message.message_id))),
            Err(ApplyError::Refused(refusal)) => {
                self.refuse(&message.message_id, refusal).await?;
                Ok(Some(Applied::Refused(message.message_id, refusal)))
            }
            Err(ApplyError::Transient(error)) => {
                self.release(&message.message_id).await?;
                Err(error)
            }
        }
    }

    /// Takes the lease on the oldest message still to apply.
    async fn claim(&self, replica: Uuid) -> Result<Option<Claimed>, ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        let row: Option<ClaimRow> = sqlx::query_as(
            "UPDATE pmir_message m SET leased_by = $1, \
                     leased_until = now() + make_interval(secs => $2) \
                 WHERE m.message_id = (SELECT message_id FROM pmir_message \
                     WHERE state = 'received' ORDER BY sequence LIMIT 1) \
                   AND (m.leased_until IS NULL OR m.leased_until < now() OR m.leased_by = $1) \
                 RETURNING m.message_id, m.sequence, m.issuer, m.client_id, m.events",
        )
        .bind(replica)
        .bind(LEASE_SECONDS)
        .fetch_optional(&mut *conn)
        .await
        .map_err(StorageError::from)?;
        Ok(row.map(
            |(message_id, sequence, issuer, client_id, events)| Claimed {
                message_id,
                sequence,
                sender: FeedSender { issuer, client_id },
                events: events.0,
            },
        ))
    }

    /// Gives up the lease on `message_id`, so any replica takes it again.
    async fn release(&self, message_id: &str) -> Result<(), ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        sqlx::query(
            "UPDATE pmir_message SET leased_by = NULL, leased_until = NULL WHERE message_id = $1",
        )
        .bind(message_id)
        .execute(&mut *conn)
        .await
        .map_err(StorageError::from)?;
        Ok(())
    }

    /// Marks `message_id` refused for `refusal`; its splits stay.
    async fn refuse(&self, message_id: &str, refusal: Refusal) -> Result<(), ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        sqlx::query(
            "UPDATE pmir_message SET state = 'refused', reason = $2, refused_at = now(), \
             attempts = attempts + 1, leased_by = NULL, leased_until = NULL \
             WHERE message_id = $1 AND state = 'received'",
        )
        .bind(message_id)
        .bind(refusal.code())
        .execute(&mut *conn)
        .await
        .map_err(StorageError::from)?;
        tracing::error!(
            message_id,
            reason = refusal.code(),
            "an ITI-93 message is refused: what it splits stays unresolved until it is settled"
        );
        self.pmir_refused_count().await?;
        Ok(())
    }

    /// Applies `message` in one transaction.
    async fn apply_message(&self, message: &Claimed) -> Result<(), ApplyError> {
        let mut conn = acquire(&self.demographic_pool)
            .await
            .map_err(ServiceError::from)?;
        let mut tx = conn.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(APPLY_LOCK)
            .execute(&mut *tx)
            .await?;
        let state: Option<String> =
            sqlx::query_scalar("SELECT state FROM pmir_message WHERE message_id = $1 FOR UPDATE")
                .bind(&message.message_id)
                .fetch_optional(&mut *tx)
                .await?;
        if state.as_deref() != Some("received") {
            return Ok(());
        }
        let earlier: bool = sqlx::query_scalar(
            "SELECT EXISTS (SELECT 1 FROM pmir_message WHERE state = 'received' AND sequence < $1)",
        )
        .bind(message.sequence)
        .fetch_one(&mut *tx)
        .await?;
        if earlier {
            return Err(ApplyError::Transient(ServiceError::exception(
                "an earlier ITI-93 message is still to apply".to_owned(),
            )));
        }
        let path = self
            .binding
            .patient_identifier_path()
            .map(|(_, path)| path.to_owned())
            .unwrap_or_default();
        let mut work = Work::new(self, message, path);
        for event in &message.events {
            work.event(&mut tx, event).await?;
        }
        let changes = work.changes().await?;
        if !changes.is_empty() {
            let description = format!(
                "PMIR ITI-93 message {} from the Patient Identity Registry",
                message.message_id
            );
            let audit = self.audit(change_type::MODIFICATION, &description);
            let changes = changes.into_iter().map(|c| (audit.clone(), c)).collect();
            change::commit_contribution(
                &mut tx,
                None,
                None,
                &audit,
                changes,
                Vec::new(),
                &self.signing_ctx(),
            )
            .await?;
        }
        let holders: Vec<Uuid> = sqlx::query_scalar(
            "DELETE FROM pmir_suppression WHERE message_id = $1 RETURNING holder_id",
        )
        .bind(&message.message_id)
        .fetch_all(&mut *tx)
        .await?;
        crate::party_binding::grants::caller::store::finish(&mut tx, holders).await?;
        sqlx::query(
            "UPDATE pmir_message SET state = 'applied', events = NULL, settled_at = now(), \
             attempts = attempts + 1, leased_by = NULL, leased_until = NULL \
             WHERE message_id = $1",
        )
        .bind(&message.message_id)
        .execute(&mut *tx)
        .await?;
        self.record_party_access(
            EventActionCode::Update,
            format!("{}:{}", super::operator::OBJECT, message.message_id),
            None,
        )?;
        tx.commit().await?;
        tracing::info!(
            changes = message.events.len(),
            "an ITI-93 message is applied"
        );
        Ok(())
    }
}

/// The applier of one replica: woken by a stored message's notification, and
/// at least every [`IDLE`].
#[expect(
    clippy::infinite_loop,
    reason = "a detached background task with no shutdown channel; it ends when the runtime \
              drops it, and `-> !` cannot be spawned on stable"
)]
async fn applier(service: Arc<FerroEhrService>, replica: Uuid) {
    let mut listener = sqlx::postgres::PgListener::connect_with(&service.demographic_pool)
        .await
        .ok();
    if let Some(listening) = listener.as_mut()
        && let Err(error) = listening.listen(CHANNEL).await
    {
        tracing::warn!(%error, "the ITI-93 applier listens to nothing; it looks every 10 seconds");
        listener = None;
    }
    loop {
        loop {
            match Box::pin(service.pmir_apply_next(replica)).await {
                Ok(Some(_)) => {}
                Ok(None) => break,
                Err(error) => {
                    tracing::warn!(
                        error = %crate::error::ErrorChain::new(&error),
                        "an ITI-93 message did not apply; it is tried again"
                    );
                    break;
                }
            }
        }
        if let Err(error) = service.pmir_refused_count().await {
            tracing::warn!(
                error = %crate::error::ErrorChain::new(&error),
                "the refused ITI-93 messages could not be counted"
            );
        }
        match listener.as_mut() {
            Some(listening) => {
                // NOTE: a notification or the idle bound, whichever comes first; a lost
                // listener connection reconnects inside sqlx on the next recv.
                let _woken = tokio::time::timeout(IDLE, listening.recv()).await;
            }
            None => tokio::time::sleep(IDLE).await,
        }
    }
}
