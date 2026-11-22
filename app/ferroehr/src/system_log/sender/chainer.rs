// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The chainer task: the release of confirmed writes' holds, woken by the
//! requests that confirmed them, and both logs' frontier, seals and owed
//! deliveries (#5745, #6242).
//!
//! It runs on the audit runtime, never on a worker that serves requests
//! (#5770). A wake of an idle chainer starts a release round [`GATHER`]
//! later, so one round releases what every request of that window confirmed.
//! Every [`FRONTIER_EVERY`] the task moves the frontier of both logs
//! ([`crate::access_log::frontier`]), owes the records below it to the sinks
//! that deliver from the store, and every [`SEAL_EVERY`] seals them in
//! windows ([`crate::access_log::seal`]); until a round seals one, as at start
//! or at rest, every round tries. The bounds are our own design; no
//! specification governs them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::background::Background;
use crate::access_log::frontier::Frontier;
use crate::system_log::store::AuditStore;
use crate::system_log::store::intake::Chains;

/// How long after a wake an idle chainer starts its round, gathering the
/// confirmations the requests of that window queue.
const GATHER: Duration = Duration::from_millis(200);

/// How often the frontier of both logs moves and the records below it are
/// owed to the sinks.
const FRONTIER_EVERY: Duration = Duration::from_secs(1);

/// How often the records below the frontier are sealed in windows. No text
/// sets how often a log must be sealed: EHDS Art. 9 and Annex II 3.2 ask for a
/// complete log, CRA Annex I Part I (2)(f) for integrity and reporting of
/// corruption on the risk assessment's terms; our own design.
const SEAL_EVERY: Duration = Duration::from_secs(60);

/// How long a failed round waits before the next.
const FAILED: Duration = Duration::from_millis(500);

/// The running chainer and how to stop it.
#[derive(Debug)]
pub(super) struct Chainer {
    stopping: Arc<AtomicBool>,
    wake: Arc<Notify>,
    join: JoinHandle<()>,
}

impl Chainer {
    /// Spawn the chainer over `store`, woken by `wake`, waking each of
    /// `delivered` once a round chained records.
    pub(super) fn spawn(
        background: &Background,
        store: AuditStore,
        wake: Arc<Notify>,
        delivered: Vec<Arc<Notify>>,
    ) -> Self {
        let stopping = Arc::new(AtomicBool::new(false));
        let join = background.spawn(run(
            store,
            Arc::clone(&wake),
            delivered,
            Arc::clone(&stopping),
        ));
        Self {
            stopping,
            wake,
            join,
        }
    }

    /// Stop after the round in progress, waiting at most `timeout`.
    pub(super) async fn stop(self, timeout: Duration) {
        self.stopping.store(true, Ordering::Release);
        self.wake.notify_one();
        match tokio::time::timeout(timeout, self.join).await {
            Ok(Ok(())) => tracing::debug!("the audit chainer stopped"),
            Ok(Err(e)) => tracing::warn!("the audit chainer task panicked: {e}"),
            Err(_) => tracing::warn!("the audit chainer did not stop within {timeout:?}"),
        }
    }
}

/// The chainer's loop.
async fn run(
    store: AuditStore,
    wake: Arc<Notify>,
    delivered: Vec<Arc<Notify>>,
    stopping: Arc<AtomicBool>,
) {
    let mut frontier = Frontier::default();
    let mut moved_at = tokio::time::Instant::now()
        .checked_sub(FRONTIER_EVERY)
        .unwrap_or_else(tokio::time::Instant::now);
    // NOTE: our own design (#6242), a starting chainer seals at once what a stopped one left.
    let mut sealed_at = tokio::time::Instant::now()
        .checked_sub(SEAL_EVERY)
        .unwrap_or_else(tokio::time::Instant::now);
    while !stopping.load(Ordering::Acquire) {
        let began = tokio::time::Instant::now();
        if store.is_pending()
            && let Err(error) = store.chain_round().await
        {
            tracing::error!(
                error = %crate::error::ErrorChain::new(&error),
                "an audit release round failed; the records stay held until the next"
            );
            tokio::time::sleep(FAILED).await;
            continue;
        }
        if began.duration_since(moved_at) >= FRONTIER_EVERY {
            moved_at = began;
            let seal = began.duration_since(sealed_at) >= SEAL_EVERY;
            match store.frontier_round(&mut frontier, seal).await {
                Ok((owed, sealed)) => {
                    if sealed {
                        sealed_at = began;
                    }
                    if owed > 0 {
                        for notify in &delivered {
                            notify.notify_one();
                        }
                    }
                }
                Err(error) => tracing::error!(
                    error = %crate::error::ErrorChain::new(&error),
                    "the audit frontier did not move; the next round moves it"
                ),
            }
        }
        let next = moved_at + FRONTIER_EVERY;
        tokio::select! {
            () = wake.notified() => tokio::time::sleep(GATHER).await,
            () = tokio::time::sleep_until(next) => {}
        }
    }
    drain(&store).await;
}

/// Release what confirmed writes queued and seal both logs before the
/// chainer stops, so a graceful stop leaves nothing for the next replica's
/// sealer; the stop's timeout bounds it.
async fn drain(store: &AuditStore) {
    if let Err(error) = store.chain_round().await {
        tracing::warn!(
            error = %crate::error::ErrorChain::new(&error),
            "the audit chainer stopped with confirmed writes held; the reconciler releases them"
        );
    }
    if let Err(error) = store.settle(Chains::BOTH).await {
        tracing::warn!(
            error = %crate::error::ErrorChain::new(&error),
            "the audit chainer stopped before it sealed every record; the next sealer seals them"
        );
    }
}
