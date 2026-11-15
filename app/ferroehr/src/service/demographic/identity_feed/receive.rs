// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The receipt of an ITI-93 Mobile Patient Identity Feed message
//! (PMIR 1.6.0 §2:3.93.4.1): read, stored and acknowledged.
//!
//! The message is read by `ihe_iti::pmir::feed`, which keeps only the
//! resource ids and the business identifiers. In one transaction it is stored
//! by its `MessageHeader.id`, and each identifier and record subject it
//! splits off a `PERSON` is entered in `pmir_suppression` and the `PERSON`
//! stamped, so every replica stops resolving it before the response leaves.
//! A redelivered message is acknowledged again and stored once. The response
//! is sent once the message is stored, not once it is applied: PMIR has the
//! Consumer answer when the message is "fully processed" (§2:3.93.4.2.1), and
//! storing it durably with its effect on resolution in force is where
//! FerroEHR's processing of it is complete; applying it as content follows
//! off the request path. That reading is our own design.

use ihe_iti::pmir::error::FeedError;
use ihe_iti::pmir::feed::{Event, Feed, LinkType, PatientIdentity, ResponseId, Unwritable};
use secrecy::ExposeSecret as _;
use sqlx::Connection as _;
use sqlx::PgConnection;
use uuid::Uuid;

use super::plan::{self, Identity};
use super::{ChangeKind, FeedEvent, FeedSender, Identifier};
use crate::db::acquire::acquire;
use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::storage::error::StorageError;

/// The notification channel a stored message is announced on, which wakes
/// the appliers.
pub(super) const CHANNEL: &str = "ferroehr_pmir_feed";

/// A message received and stored.
#[derive(Debug)]
pub struct Received {
    /// The Mobile Patient Identity Feed Response (§2:3.93.4.2.2), FHIR JSON.
    pub acknowledgement: Vec<u8>,
    /// Whether this receipt stored the message; `false` for a redelivery.
    pub stored: bool,
}

/// Why a message was not stored.
#[derive(Debug, thiserror::Error)]
pub enum ReceiveError {
    /// The message departs from the PMIR profiles; the body is the
    /// `OperationOutcome` that refuses it.
    #[error("the message is no PMIR feed message")]
    Malformed {
        /// The reader's refusal.
        #[source]
        error: FeedError,
        /// The refusing `OperationOutcome`, FHIR JSON.
        outcome: Vec<u8>,
    },
    /// The response could not be written.
    #[error(transparent)]
    Unwritable(#[from] Unwritable),
    /// The store did not take the message.
    #[error(transparent)]
    Service(#[from] ServiceError),
}

impl From<StorageError> for ReceiveError {
    fn from(error: StorageError) -> Self {
        Self::Service(error.into())
    }
}

impl From<sqlx::Error> for ReceiveError {
    fn from(error: sqlx::Error) -> Self {
        Self::Service(ServiceError::from(StorageError::from(error)))
    }
}

/// The id a `Patient` reference names: the segment after `Patient/`.
fn referenced_id(reference: &str) -> String {
    reference
        .rsplit_once("Patient/")
        .map_or(reference, |(_, id)| id)
        .trim_end_matches('/')
        .split('/')
        .next()
        .unwrap_or_default()
        .to_owned()
}

/// The stored form of `identity` reported as `kind`: its id, its
/// identifiers, its `active` and the survivor its `replaced-by` link names.
fn event(kind: ChangeKind, identity: &PatientIdentity) -> FeedEvent {
    FeedEvent {
        kind,
        id: identity
            .id()
            .map(|id| id.expose_secret().to_owned())
            .unwrap_or_default(),
        identifiers: identity
            .identifiers()
            .iter()
            .filter_map(|identifier| {
                // NOTE: an identifier without a system names no domain, so no patient
                // PERSON carries it under an issuer and nothing resolves by it.
                Some(Identifier {
                    system: identifier.system()?.to_owned(),
                    value: identifier.value().expose_secret().to_owned(),
                })
            })
            .collect(),
        active: identity.active(),
        replaced_by: identity
            .links()
            .iter()
            .find(|link| link.kind() == LinkType::ReplacedBy)
            .map(|link| referenced_id(link.other().expose_secret())),
    }
}

/// The stored form of the changes of `feed`, in history order. The reader
/// holds every created, updated and merged `Patient` to stating `active`,
/// so the state of each is read from the message alone (§2:3.93.4.1.2.4).
fn events(feed: &Feed) -> Vec<FeedEvent> {
    feed.events()
        .iter()
        .filter_map(|change| match change {
            Event::Created(identity) => Some(event(ChangeKind::Create, identity)),
            Event::Updated(identity) => Some(event(ChangeKind::Update, identity)),
            Event::Deleted(identity) => Some(event(ChangeKind::Delete, identity)),
            Event::Merged {
                subsumed,
                surviving,
            } => Some(FeedEvent {
                replaced_by: Some(referenced_id(surviving.expose_secret())),
                ..event(ChangeKind::Merge, subsumed)
            }),
            _ => None,
        })
        .collect()
}

/// One identifier or record subject a received message stops resolving.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Split {
    holder: Uuid,
    kind: &'static str,
    namespace: String,
    value: String,
}

impl FerroEhrService {
    /// Reads, stores and acknowledges the ITI-93 message `body` of media type
    /// `media` that `sender` posted to the feed route `source`.
    ///
    /// # Errors
    /// [`ReceiveError::Malformed`] for a message the PMIR profiles refuse,
    /// [`ReceiveError::Service`] when the store does not take it, and
    /// [`ReceiveError::Unwritable`] when the response cannot be written.
    pub async fn pmir_receive(
        &self,
        sender: &FeedSender,
        media: Option<&str>,
        body: &[u8],
        source: &str,
    ) -> Result<Received, ReceiveError> {
        let feed = match Feed::read(media, body) {
            Ok(feed) => feed,
            Err(error) => {
                let outcome = ihe_iti::pmir::feed::refusal(&error)?;
                return Err(ReceiveError::Malformed { error, outcome });
            }
        };
        let stored = self
            .pmir_store(sender, feed.message_id(), &events(&feed))
            .await?;
        let response = ResponseId::new(&Uuid::now_v7().to_string())
            .map_err(|e| ServiceError::internal("minting the response message id", e))?;
        Ok(Received {
            acknowledgement: feed.acknowledgement(&response, source)?,
            stored,
        })
    }

    /// Stores the message `message_id` that `sender` sent, read into
    /// `events`, and stops what it splits resolving, in one transaction;
    /// `false` when the message was stored before, which changes nothing.
    async fn pmir_store(
        &self,
        sender: &FeedSender,
        message_id: &str,
        events: &[FeedEvent],
    ) -> Result<bool, ReceiveError> {
        let mut conn = acquire(&self.demographic_pool)
            .await
            .map_err(ServiceError::from)?;
        let mut tx = conn.begin().await?;
        let stored: Option<i64> = sqlx::query_scalar(
            "INSERT INTO pmir_message (message_id, issuer, client_id, events) \
             VALUES ($1, $2, $3, $4) ON CONFLICT (message_id) DO NOTHING RETURNING sequence",
        )
        .bind(message_id)
        .bind(&sender.issuer)
        .bind(&sender.client_id)
        .bind(sqlx::types::Json(events))
        .fetch_optional(&mut *tx)
        .await?;
        if stored.is_some() {
            let splits = self.splits(&mut tx, events).await?;
            for split in &splits {
                sqlx::query(
                    "INSERT INTO pmir_suppression (message_id, holder_id, kind, namespace, value) \
                     VALUES ($1, $2, $3, $4, $5) ON CONFLICT DO NOTHING",
                )
                .bind(message_id)
                .bind(split.holder)
                .bind(split.kind)
                .bind(&split.namespace)
                .bind(&split.value)
                .execute(&mut *tx)
                .await?;
            }
            let holders = splits.iter().map(|split| split.holder).collect();
            crate::party_binding::grants::caller::store::finish(&mut tx, holders).await?;
            sqlx::query("SELECT pg_notify($1, $2)")
                .bind(CHANNEL)
                .bind(message_id)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        tracing::info!(
            stored = stored.is_some(),
            changes = events.len(),
            "an ITI-93 message is received"
        );
        Ok(stored.is_some())
    }

    /// The identifiers and record subjects `events` split off a `PERSON`:
    /// those an update or a delete no longer announces, and the record
    /// subjects a standing merge moved when a message about the merged
    /// identity states anything but that merge. Read on `tx` before the
    /// message is applied, so the split holds from receipt.
    async fn splits(
        &self,
        tx: &mut PgConnection,
        events: &[FeedEvent],
    ) -> Result<Vec<Split>, ReceiveError> {
        let engine = self.identifiers.as_deref();
        let mut splits = Vec::new();
        for event in events {
            let known = plan::identity(tx, &event.id).await?;
            let merged_into = known
                .as_ref()
                .and_then(|identity| identity.merged_into.as_ref());
            let restates_merge =
                event.kind == ChangeKind::Merge && event.replaced_by.as_ref() == merged_into;
            if merged_into.is_some()
                && !restates_merge
                && let Some(merge) = plan::standing_merge(tx, &event.id).await?
                && let Some(holder) = merge.survivor_person
            {
                splits.extend(merge.subjects.into_iter().map(|(namespace, value)| Split {
                    holder,
                    kind: "subject",
                    namespace,
                    value,
                }));
            }
            if matches!(event.kind, ChangeKind::Merge) || merged_into.is_some() {
                continue;
            }
            let Some(person) = Self::bound(tx, engine, known.as_ref(), &event.identifiers).await?
            else {
                continue;
            };
            let announced: &[Identifier] = match event.kind {
                ChangeKind::Delete => &[],
                _ => &event.identifiers,
            };
            for row in plan::unannounced(tx, engine, person, announced).await? {
                splits.push(Split {
                    holder: person,
                    kind: "identifier",
                    namespace: row.issuer,
                    value: row.value,
                });
            }
        }
        splits.sort();
        splits.dedup();
        Ok(splits)
    }

    /// The `PERSON` an identity is bound to for the receipt's split: its
    /// binding, or the one `PERSON` that carries one of its identifiers. An
    /// ambiguous identity splits nothing here; its message is refused when
    /// applied and resolves nothing more by the split it never had.
    async fn bound(
        tx: &mut PgConnection,
        engine: Option<&crate::identifier_protection::engine::IdentifierProtection>,
        known: Option<&Identity>,
        identifiers: &[Identifier],
    ) -> Result<Option<Uuid>, ReceiveError> {
        match plan::resolve(tx, engine, known, identifiers, None).await {
            Ok(person) => Ok(person),
            Err(plan::Resolved::Storage(error)) => Err(error.into()),
            Err(plan::Resolved::Refused(_)) => Ok(None),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::referenced_id;

    #[test]
    fn a_reference_names_the_patient_id() {
        assert_eq!(referenced_id("Patient/abc"), "abc");
        assert_eq!(referenced_id("https://pix.example/fhir/Patient/abc"), "abc");
        assert_eq!(
            referenced_id("https://pix.example/fhir/Patient/abc/_history/2"),
            "abc"
        );
        assert_eq!(referenced_id("abc"), "abc");
    }
}
