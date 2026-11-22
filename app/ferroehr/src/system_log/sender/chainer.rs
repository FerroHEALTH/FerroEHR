// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The chainer task: rounds of [`AuditStore::chain_round`], woken by the
//! requests that wrote system intake rows or confirmed held writes, and by a
//! slow poll for the rows other replicas left (#5745), and the access log's
//! frontier, seals and owed deliveries (#6242).
//!
//! It runs on the audit runtime, never on a worker that serves requests
//! (#5770). A wake of an idle chainer starts a round [`GATHER`] later, and
//! while requests keep writing, rounds start one [`ROUND`] apart, so one round
//! takes what every request of that window wrote, up to [`BATCH`] system rows.
//! A round that filled its batch runs again at once. Every
//! [`FRONTIER_EVERY`] the task moves the access log's frontier
//! ([`crate::access_log::frontier`]), owes the entries below it to the sinks
//! that deliver from the store, and every [`SEAL_EVERY`] seals them in one
//! window ([`crate::access_log::seal`]); until a round seals one, as at start
//! or at rest, every round tries. The bounds are our own design; no
//! specification governs them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use opentelemetry::KeyValue;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::background::Background;
use crate::access_log::frontier::Frontier;
use crate::system_log::store::AuditStore;
use crate::system_log::store::intake::Chains;

/// The most intake rows of each chain one round takes.
const BATCH: u32 = 512;

/// How long after a wake an idle chainer starts its round, gathering the rows
/// the requests of that window write.
const GATHER: Duration = Duration::from_millis(200);

/// While requests keep writing, the time from the start of one round to the
/// start of the next, unless a round filled its batch: at 3000 entries a
/// second a round takes about 150, where back-to-back rounds took about 10,
/// each with its own signed checkpoint and commit, and rounds 200 ms apart
/// put bursts of 600 on the database (#5770).
const ROUND: Duration = Duration::from_millis(50);

/// How long the chainer waits for a wake before it looks at both chains,
/// for the rows a stopped replica left.
const POLL: Duration = Duration::from_secs(2);

/// How often the access log's frontier moves and the entries below it are
/// owed to the sinks.
const FRONTIER_EVERY: Duration = Duration::from_secs(1);

/// How often the entries below the frontier are sealed in a window. No text
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
    let metrics = crate::telemetry::access_path::metrics();
    let mut look = Chains::BOTH;
    let mut frontier = Frontier::default();
    let mut moved_at = tokio::time::Instant::now();
    // NOTE: our own design (#6242), a starting chainer seals at once what a stopped one left.
    let mut sealed_at = tokio::time::Instant::now()
        .checked_sub(SEAL_EVERY)
        .unwrap_or_else(tokio::time::Instant::now);
    let mut polled_at = tokio::time::Instant::now();
    while !stopping.load(Ordering::Acquire) {
        let began = tokio::time::Instant::now();
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
                    "the access log's frontier did not move; the next round moves it"
                ),
            }
        }
        let round = store.chain_round(BATCH, look).await;
        look = Chains::default();
        match round {
            Ok(round) => {
                if round.system > 0 {
                    metrics
                        .log_chained
                        .add(round.system, &[KeyValue::new("chain", "system")]);
                    crate::telemetry::metrics::metrics()
                        .atna_audit_sent
                        .add(round.system, &[KeyValue::new("sink", "store")]);
                    for notify in &delivered {
                        notify.notify_one();
                    }
                }
                if round.filled(BATCH) {
                    continue;
                }
                if store.is_pending() {
                    tokio::time::sleep_until(began + ROUND).await;
                    continue;
                }
            }
            Err(error) => {
                tracing::error!(
                    error = %crate::error::ErrorChain::new(&error),
                    "an audit chainer round failed; the intake keeps its records"
                );
                tokio::time::sleep(FAILED).await;
                continue;
            }
        }
        tokio::select! {
            () = wake.notified() => tokio::time::sleep(GATHER).await,
            () = tokio::time::sleep(FRONTIER_EVERY) => {
                if polled_at.elapsed() >= POLL {
                    polled_at = tokio::time::Instant::now();
                    look = Chains::BOTH;
                }
            }
        }
    }
    drain(&store).await;
}

/// Chain what the system intake holds and seal what the access log holds
/// before the chainer stops, so a graceful stop leaves nothing for the next
/// replica's poll; the stop's timeout bounds it.
async fn drain(store: &AuditStore) {
    loop {
        match store.chain_round(BATCH, Chains::BOTH).await {
            Ok(round) if round.chained() > 0 => {}
            Ok(_) => break,
            Err(error) => {
                tracing::warn!(
                    error = %crate::error::ErrorChain::new(&error),
                    "the audit chainer stopped with rows in the intake; the next chainer takes them"
                );
                return;
            }
        }
    }
    if let Err(error) = store
        .settle(Chains {
            access_log: true,
            system: false,
        })
        .await
    {
        tracing::warn!(
            error = %crate::error::ErrorChain::new(&error),
            "the audit chainer stopped before it sealed every entry; the next sealer seals them"
        );
    }
}
