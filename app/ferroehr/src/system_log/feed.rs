// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-20 ATX: FHIR Feed (`[audit.fhir_feed]`): `ihe-atna`'s
//! [`FeedRepository`] over the endpoint's [`OutboundClient`], which carries
//! the feed's IHE IUA bearer token on each request and renews it once on a
//! `401` (RFC 6750 §3.1), and the [`StoreOutbox`] `ihe-atna`'s forwarder delivers from when
//! the local audit store is on.
//!
//! The store is the feed's outbox: a record not yet delivered has a row in
//! `audit.fhir_feed_outbox`, naming its chain and position under an id of the
//! outbox's own, which the store writes with the record while the feed is
//! configured, read oldest first by that id, and removed once the repository
//! accepted it, when a system-chain record's `delivered_fhir_feed_at` is
//! stamped. The document is rendered from the record's columns when it is
//! read. A record the repository refused, or one that cannot be rendered, is
//! held aside in memory until the next start, because the table is
//! append-only beside its two delivery stamps (no specification governs where
//! a refused record waits: our own design).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use ferrohealth_smart::client::outbound::OutboundClient;
use ferrohealth_smart::config::ServerTls;
use ihe_atna::atna::feed::FeedRepository;
use ihe_atna::atna::forwarder::{Outbox, OutboxError};
use ihe_atna::atna::spool::{Depth, Stored};
use secrecy::SecretSlice;
use tokio::sync::Notify;

use crate::system_log::AuditError;
use crate::system_log::config::FhirFeedConfig;
use crate::system_log::store::AuditStore;

/// The repository `config` names, over `ferrohealth-smart`'s outbound client
/// of `[audit.fhir_feed.endpoint]`.
///
/// # Errors
/// [`AuditError::Transport`] when the endpoint is refused or the URL is not an
/// `http(s)` URL; boot validation refuses an incomplete table first.
pub(super) fn repository(config: &FhirFeedConfig) -> Result<FeedRepository, AuditError> {
    let client = OutboundClient::build(&config.endpoint, "audit.fhir_feed.endpoint", &ServerTls)
        .map_err(|e| AuditError::Transport(Box::new(e)))?;
    // NOTE: an `http` feed opens the `plaintext_audit_feed` deployment gap, which
    // production refuses unless it is accepted by name.
    if client.url().scheme() == "http" {
        FeedRepository::cleartext(client)
    } else {
        FeedRepository::new(client)
    }
    .map_err(|e| AuditError::Transport(Box::new(e)))
}

/// The local audit store as the FHIR Feed's outbox.
#[derive(Debug)]
pub(super) struct StoreOutbox {
    store: AuditStore,
    /// Woken by the drain after it stored records.
    notify: Arc<Notify>,
    /// How long to wait for a wake before reading the store again.
    idle: Duration,
    /// The outbox ids of the records the repository refused, held aside.
    refused: Mutex<BTreeSet<i64>>,
    /// The rows awaiting delivery at the last read.
    waiting: AtomicUsize,
    /// Whether the records written before the outbox existed are entered in it.
    backlog: AtomicBool,
}

impl StoreOutbox {
    /// The outbox over `store`, woken by `notify`, reading the store again
    /// at least every `idle`.
    pub(super) fn new(store: AuditStore, notify: Arc<Notify>, idle: Duration) -> Self {
        Self {
            store,
            notify,
            idle,
            refused: Mutex::new(BTreeSet::new()),
            waiting: AtomicUsize::new(0),
            backlog: AtomicBool::new(false),
        }
    }

    /// The outbox ids held aside.
    fn refused(&self) -> Vec<i64> {
        self.refused
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .iter()
            .copied()
            .collect()
    }
}

/// The forwarder's error for a store failure.
fn outbox_error(error: AuditError) -> OutboxError {
    OutboxError(Box::new(error))
}

#[async_trait::async_trait]
impl Outbox for StoreOutbox {
    async fn oldest(&self) -> Result<Option<Stored>, OutboxError> {
        if !self.backlog.load(Ordering::Relaxed) {
            let moved = self
                .store
                .enqueue_fhir_feed_backlog()
                .await
                .map_err(outbox_error)?;
            if moved > 0 {
                tracing::info!(
                    records = moved,
                    "ATNA audit FHIR feed: records stored before the outbox existed are owed"
                );
            }
            self.backlog.store(true, Ordering::Relaxed);
        }
        let (sequence, document) = loop {
            let (oldest, waiting) = self
                .store
                .oldest_fhir_feed(&self.refused())
                .await
                .map_err(outbox_error)?;
            self.waiting
                .store(usize::try_from(waiting).unwrap_or(0), Ordering::Relaxed);
            let Some((sequence, rendered)) = oldest else {
                return Ok(None);
            };
            match rendered {
                Ok(document) => break (sequence, document),
                Err(e) => {
                    tracing::error!(
                        owed = sequence,
                        error = %crate::error::ErrorChain::new(&e),
                        "ATNA audit FHIR feed: the record cannot be rendered; it is held aside \
                         until the next start"
                    );
                    self.refused
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .insert(sequence);
                }
            }
        };
        let sequence = u64::try_from(sequence).map_err(|e| OutboxError(Box::new(e)))?;
        let message = serde_json::to_vec(&document).map_err(|e| OutboxError(Box::new(e)))?;
        Ok(Some(Stored {
            sequence,
            message: SecretSlice::from(message),
        }))
    }

    async fn stored(&self) {
        tokio::select! {
            () = self.notify.notified() => {}
            () = tokio::time::sleep(self.idle) => {}
        }
    }

    async fn remove(&self, sequence: u64) -> Result<(), OutboxError> {
        let sequence = i64::try_from(sequence).map_err(|e| OutboxError(Box::new(e)))?;
        self.store
            .mark_fhir_feed_delivered(sequence)
            .await
            .map_err(outbox_error)
    }

    async fn quarantine(&self, sequence: u64) -> Result<(), OutboxError> {
        let sequence = i64::try_from(sequence).map_err(|e| OutboxError(Box::new(e)))?;
        tracing::error!(
            owed = sequence,
            "ATNA audit FHIR feed: the repository refused the record; it is held aside until \
             the next start"
        );
        self.refused
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(sequence);
        Ok(())
    }

    fn depth(&self) -> Depth {
        let quarantined = self
            .refused
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .len();
        Depth {
            messages: self.waiting.load(Ordering::Relaxed) + quarantined,
            bytes: 0,
            quarantined,
        }
    }

    fn is_durable(&self) -> bool {
        true
    }
}
