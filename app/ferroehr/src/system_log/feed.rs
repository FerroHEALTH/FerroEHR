// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-20 ATX: FHIR Feed (`[audit.fhir_feed]`): `ihe-atna`'s
//! [`FeedRepository`] with its [`Bearer`] source, which asks the feed's IHE
//! IUA bearer token for each request and renews it once on a `401` (RFC 6750
//! §3.1), and the [`StoreOutbox`] `ihe-atna`'s forwarder delivers from when
//! the local audit store is on.
//!
//! The store is the feed's outbox: a record not yet delivered has a row in
//! `audit.fhir_feed_outbox`, which the store writes with the record while the
//! feed is configured, read oldest first by its chain position, and removed
//! once the repository accepted it, when the record's `delivered_fhir_feed_at`
//! is stamped. The document is rendered from the record's columns when it is
//! read. A record the repository refused, or one that cannot be rendered, is
//! held aside in memory until the next start, because the table is
//! append-only beside its two delivery stamps (no specification governs where
//! a refused record waits: our own design).

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use ihe_atna::atna::bearer::Bearer;
use ihe_atna::atna::feed::FeedRepository;
use ihe_atna::atna::forwarder::{Outbox, OutboxError};
use ihe_atna::atna::spool::{Depth, Stored};
use secrecy::SecretSlice;
use tokio::sync::Notify;

use crate::system_log::AuditError;
use crate::system_log::config::FhirFeedConfig;
use crate::system_log::store::AuditStore;

/// How long one `POST` may take; no specification governs it: our own design.
const FEED_TIMEOUT: Duration = Duration::from_secs(30);

/// The repository `config` names, authorized by its grant when one is set.
///
/// # Errors
/// [`AuditError::Transport`] when the URL is not an `http(s)` URL, the
/// configured grant is incomplete or its HTTP engine cannot be built; boot
/// validation refuses an incomplete grant first.
pub(super) fn repository(config: &FhirFeedConfig) -> Result<FeedRepository, AuditError> {
    let token = config
        .oauth
        .as_ref()
        .map(|oauth| {
            let grant = oauth
                .resolve()
                .map_err(|e| AuditError::Transport(Box::new(e)))?;
            ferrohealth_smart::client::cache::TokenSource::new(grant)
                .map_err(|e| AuditError::Transport(Box::new(e)))
        })
        .transpose()?;
    let base =
        url::Url::parse(config.url.expose()).map_err(|e| AuditError::Transport(Box::new(e)))?;
    let http = reqwest::Client::builder()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|e| AuditError::Transport(Box::new(e)))?;
    // NOTE: an `http` feed opens the `plaintext_audit_feed` deployment gap, which
    // production refuses unless it is accepted by name.
    let repository = if base.scheme() == "http" {
        FeedRepository::cleartext(base, http, FEED_TIMEOUT)
    } else {
        FeedRepository::new(base, http, FEED_TIMEOUT)
    }
    .map_err(|e| AuditError::Transport(Box::new(e)))?;
    Ok(match token {
        Some(token) => repository.with_credentials(Arc::new(Bearer::from(token))),
        None => repository,
    })
}

/// The local audit store as the FHIR Feed's outbox.
#[derive(Debug)]
pub(super) struct StoreOutbox {
    store: AuditStore,
    /// Woken by the drain after it stored records.
    notify: Arc<Notify>,
    /// How long to wait for a wake before reading the store again.
    idle: Duration,
    /// The chain positions the repository refused, held aside.
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

    /// The chain positions held aside.
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
                        chain_seq = sequence,
                        error = %crate::service::error::ErrorChain::new(&e),
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
            chain_seq = sequence,
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
