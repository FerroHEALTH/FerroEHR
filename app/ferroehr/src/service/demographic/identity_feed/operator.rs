// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The operator's path for a refused ITI-93 message (#5725): list the
//! refused messages, retry one once its cause is fixed, or settle one.
//!
//! A **retry** puts the message back at its own arrival position, so the one
//! apply path applies it next under its lease, with the configuration and
//! grants then in force. It is refused when a message received after it
//! that names one of its identities has applied since: each announcement
//! states an identity's whole identifier set, so applying the older one
//! would roll the identity back. A **settlement** closes the message without
//! applying it, by an explicit decision with a reason: `discard` keeps its
//! splits, so what the Registry announced as split stays unresolved, and
//! `lift-splits` removes them, so those identifiers and record subjects
//! resolve as they did before the message. No settlement applies a message
//! by an operator's choice of `PERSON`, identifier or survivor: that would
//! assert an identity the Registry did not state. Every list, retry and
//! settlement is an access record naming the operator and the message. PMIR
//! leaves the Consumer's handling to its capabilities (PMIR 1.6.0
//! §2:3.93.4.1.3): our own design.

use serde::Serialize;
use sqlx::Connection as _;
use uuid::Uuid;

use super::apply::APPLY_LOCK;
use super::receive::CHANNEL;
use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::error::refusal::Missing;
use crate::service::FerroEhrService;
use crate::storage::error::StorageError;
use crate::system_log::event::EventActionCode;

/// The longest settlement reason kept, in characters.
pub const REASON_LIMIT: usize = 1000;

/// One refused message as an operator reads it: ids and counts, never an
/// identifier value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefusedMessage {
    /// Its `MessageHeader.id`.
    pub message_id: String,
    /// The refusal's code ([`super::Refusal::code`]).
    pub reason: String,
    /// When it was received.
    pub received_at: jiff::Timestamp,
    /// When its newest attempt refused it.
    pub refused_at: Option<jiff::Timestamp>,
    /// The apply attempts that ended.
    pub attempts: i32,
    /// The Registry's `Patient.id`s it names.
    pub identities: Vec<String>,
    /// The `PERSON`s whose identifiers or record subjects it keeps split.
    pub splits: Vec<SplitHolder>,
}

/// One `PERSON` a refused message keeps split, with how many of its
/// identifiers and record subjects stop resolving.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SplitHolder {
    /// The `PERSON`'s container id.
    pub person: Uuid,
    /// Its patient identifiers that stop resolving.
    pub identifiers: i64,
    /// Its record subjects that stop resolving.
    pub subjects: i64,
}

/// How an operator settles a refused message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Decision {
    /// Close it without applying and keep its splits.
    Discard,
    /// Close it without applying and lift its splits.
    LiftSplits,
}

impl Decision {
    /// The decision's code, as the row and the wire carry it.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::Discard => "discard",
            Self::LiftSplits => "lift-splits",
        }
    }

    /// The decision a code names; `None` for any other text.
    #[must_use]
    pub fn of(code: &str) -> Option<Self> {
        match code {
            "discard" => Some(Self::Discard),
            "lift-splits" => Some(Self::LiftSplits),
            _ => None,
        }
    }
}

/// The refused messages as the list answers them.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct RefusedList {
    /// The refused messages still to settle, oldest first.
    pub messages: Vec<RefusedMessage>,
}

/// What a retry did: the message is back in the queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Retried {
    /// The message retried.
    pub message_id: String,
    /// Its state now, `received`.
    pub state: &'static str,
}

/// What a settlement did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Settled {
    /// The message settled.
    pub message_id: String,
    /// The decision.
    pub decision: Decision,
    /// The identifiers and record subjects whose split it lifted.
    pub lifted: u64,
}

/// A `pmir_message` row as the list reads it.
type RefusedRow = (
    String,
    Option<String>,
    jiff_sqlx::Timestamp,
    Option<jiff_sqlx::Timestamp>,
    i32,
    Vec<String>,
);

impl FerroEhrService {
    /// The refused messages still to settle, oldest first, each recorded as
    /// read by the request's operator.
    ///
    /// # Errors
    /// [`ServiceError`] when the store does not answer or the audit trail
    /// refuses a record.
    pub async fn pmir_refused(&self) -> Result<RefusedList, ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        let rows: Vec<RefusedRow> = sqlx::query_as(
            "SELECT message_id, reason, received_at, refused_at, attempts, identities \
             FROM pmir_message WHERE state = 'refused' ORDER BY sequence",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(StorageError::from)?;
        let held: Vec<(String, Uuid, i64, i64)> = sqlx::query_as(
            "SELECT s.message_id, s.holder_id, \
                    count(*) FILTER (WHERE s.kind = 'identifier'), \
                    count(*) FILTER (WHERE s.kind = 'subject') \
             FROM pmir_suppression s JOIN pmir_message m ON m.message_id = s.message_id \
             WHERE m.state = 'refused' GROUP BY s.message_id, s.holder_id \
             ORDER BY s.message_id, s.holder_id",
        )
        .fetch_all(&mut *conn)
        .await
        .map_err(StorageError::from)?;
        let mut listed = Vec::with_capacity(rows.len());
        for (message_id, reason, received_at, refused_at, attempts, identities) in rows {
            let splits = held
                .iter()
                .filter(|(id, ..)| *id == message_id)
                .map(|(_, person, identifiers, subjects)| SplitHolder {
                    person: *person,
                    identifiers: *identifiers,
                    subjects: *subjects,
                })
                .collect();
            self.record_party_access(
                EventActionCode::Read,
                format!("{OBJECT}:{message_id}"),
                Some(LIST),
            )
            .await?;
            listed.push(RefusedMessage {
                message_id,
                reason: reason.unwrap_or_default(),
                received_at: received_at.to_jiff(),
                refused_at: refused_at.map(jiff_sqlx::Timestamp::to_jiff),
                attempts,
                identities,
                splits,
            });
        }
        Ok(RefusedList { messages: listed })
    }

    /// The number of refused messages still to settle.
    ///
    /// # Errors
    /// [`ServiceError`] when the store does not answer.
    pub async fn pmir_refused_count(&self) -> Result<u64, ServiceError> {
        let mut conn = acquire(&self.demographic_pool).await?;
        let count: i64 =
            sqlx::query_scalar("SELECT count(*) FROM pmir_message WHERE state = 'refused'")
                .fetch_one(&mut *conn)
                .await
                .map_err(StorageError::from)?;
        let count = u64::try_from(count)
            .map_err(|error| ServiceError::internal("counting the refused messages", error))?;
        crate::telemetry::metrics::metrics()
            .pmir_messages_refused
            .record(count, &[]);
        Ok(count)
    }

    /// Puts the refused message `message_id` back at its arrival position,
    /// so the applier applies it next.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] for no such message,
    /// [`ServiceError::Conflict`] for a message that is not refused or that a
    /// later applied message about one of its identities supersedes,
    /// [`ServiceError::BadRequest`] outside a request naming its operator,
    /// and [`ServiceError`] when the store or the audit trail refuses.
    pub async fn pmir_retry(&self, message_id: &str) -> Result<Retried, ServiceError> {
        let operator = operator()?;
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await.map_err(StorageError::from)?;
        // NOTE: our own design, the applier holds this lock while it applies, so no later message
        // applies between the supersession check and the message's return to the queue.
        sqlx::query("SELECT pg_advisory_xact_lock($1)")
            .bind(APPLY_LOCK)
            .execute(&mut *tx)
            .await
            .map_err(StorageError::from)?;
        let row: Option<(String, i64, Vec<String>)> = sqlx::query_as(
            "SELECT state, sequence, identities FROM pmir_message \
             WHERE message_id = $1 FOR UPDATE",
        )
        .bind(message_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from)?;
        let (state, sequence, identities) = row.ok_or_else(|| missing(message_id))?;
        if state != "refused" {
            return Err(ServiceError::conflict(format!(
                "the ITI-93 message {message_id} is {state}, not refused: only a refused message \
                 is retried"
            )));
        }
        let superseding: Option<String> = sqlx::query_scalar(
            "SELECT message_id FROM pmir_message WHERE state = 'applied' AND sequence > $1 \
               AND identities && $2 ORDER BY sequence LIMIT 1",
        )
        .bind(sequence)
        .bind(&identities)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from)?;
        if let Some(later) = superseding {
            return Err(ServiceError::conflict(format!(
                "the ITI-93 message {later}, received after {message_id}, has applied a newer \
                 state of one of its identities: applying {message_id} now would roll that \
                 identity back, so settle it"
            )));
        }
        sqlx::query(
            "UPDATE pmir_message SET state = 'received', reason = NULL, \
             leased_by = NULL, leased_until = NULL WHERE message_id = $1",
        )
        .bind(message_id)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from)?;
        sqlx::query("SELECT pg_notify($1, $2)")
            .bind(CHANNEL)
            .bind(message_id)
            .execute(&mut *tx)
            .await
            .map_err(StorageError::from)?;
        self.record_party_access(
            EventActionCode::Update,
            format!("{OBJECT}:{message_id}"),
            Some(RETRY),
        )
        .await?;
        tx.commit().await.map_err(StorageError::from)?;
        tracing::info!(message_id, operator, "a refused ITI-93 message is retried");
        Ok(Retried {
            message_id: message_id.to_owned(),
            state: "received",
        })
    }

    /// Settles the refused message `message_id` by `decision` for `reason`,
    /// without applying it; a message discarded earlier may still have its
    /// splits lifted.
    ///
    /// # Errors
    /// [`ServiceError::NotFound`] for no such message,
    /// [`ServiceError::Conflict`] for a message the decision cannot settle,
    /// [`ServiceError::BadRequest`] for an empty or overlong reason or
    /// outside a request naming its operator, and [`ServiceError`] when the
    /// store or the audit trail refuses.
    pub async fn pmir_settle(
        &self,
        message_id: &str,
        decision: Decision,
        reason: &str,
    ) -> Result<Settled, ServiceError> {
        let operator = operator()?;
        let reason = reason.trim();
        if reason.is_empty() || reason.chars().count() > REASON_LIMIT {
            return Err(ServiceError::precondition(format!(
                "a settlement needs a reason of 1 to {REASON_LIMIT} characters"
            )));
        }
        let mut conn = acquire(&self.demographic_pool).await?;
        let mut tx = conn.begin().await.map_err(StorageError::from)?;
        let row: Option<(String, Option<String>)> = sqlx::query_as(
            "SELECT state, decision FROM pmir_message WHERE message_id = $1 FOR UPDATE",
        )
        .bind(message_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(StorageError::from)?;
        let (state, settled) = row.ok_or_else(|| missing(message_id))?;
        let lifts_discarded = state == "settled"
            && settled.as_deref() == Some(Decision::Discard.code())
            && decision == Decision::LiftSplits;
        if state != "refused" && !lifts_discarded {
            return Err(ServiceError::conflict(format!(
                "the ITI-93 message {message_id} is {state}: only a refused message is settled, \
                 and a discarded one only has its splits lifted"
            )));
        }
        let lifted = match decision {
            Decision::Discard => 0,
            Decision::LiftSplits => {
                let holders: Vec<Uuid> = sqlx::query_scalar(
                    "DELETE FROM pmir_suppression WHERE message_id = $1 RETURNING holder_id",
                )
                .bind(message_id)
                .fetch_all(&mut *tx)
                .await
                .map_err(StorageError::from)?;
                let lifted = u64::try_from(holders.len())
                    .map_err(|error| ServiceError::internal("counting the lifted splits", error))?;
                crate::party_binding::grants::caller::store::finish(&mut tx, holders).await?;
                lifted
            }
        };
        sqlx::query(
            "UPDATE pmir_message SET state = 'settled', events = NULL, decision = $2, \
             decided_by = $3, decision_reason = $4, settled_at = now(), \
             leased_by = NULL, leased_until = NULL WHERE message_id = $1",
        )
        .bind(message_id)
        .bind(decision.code())
        .bind(&operator)
        .bind(reason)
        .execute(&mut *tx)
        .await
        .map_err(StorageError::from)?;
        self.record_party_access(
            EventActionCode::Update,
            format!("{OBJECT}:{message_id}"),
            Some(SETTLE),
        )
        .await?;
        tx.commit().await.map_err(StorageError::from)?;
        tracing::info!(
            message_id,
            operator,
            decision = decision.code(),
            lifted,
            "a refused ITI-93 message is settled"
        );
        Ok(Settled {
            message_id: message_id.to_owned(),
            decision,
            lifted,
        })
    }
}

/// The prefix of the object id an operator's act on a message is recorded
/// under.
pub const OBJECT: &str = "pmir-message";
/// The operation of a list of the refused messages.
const LIST: &str = "admin_pmir_messages_list";
/// The operation of a retry.
const RETRY: &str = "admin_pmir_message_retry";
/// The operation of a settlement.
const SETTLE: &str = "admin_pmir_message_settle";

/// The operator of the current request: its committer's subject.
fn operator() -> Result<String, ServiceError> {
    crate::versioning::committer::current_committer()
        .map(|committer| committer.subject)
        .filter(|subject| !subject.trim().is_empty())
        .ok_or_else(|| {
            ServiceError::precondition(
                "a retry or settlement of an ITI-93 message names its operator: authenticate the \
                 request, or give the CLI --operator",
            )
        })
}

/// The refusal of a message id the store holds no message of.
fn missing(message_id: &str) -> ServiceError {
    ServiceError::not_found(
        Missing::VersionedObject,
        format!("no ITI-93 message {message_id} is stored"),
    )
}

#[cfg(test)]
mod tests {
    use super::Decision;

    #[test]
    fn a_decision_reads_back_from_its_code() {
        for decision in [Decision::Discard, Decision::LiftSplits] {
            assert_eq!(Decision::of(decision.code()), Some(decision));
        }
        assert_eq!(Decision::of("apply"), None);
    }
}
