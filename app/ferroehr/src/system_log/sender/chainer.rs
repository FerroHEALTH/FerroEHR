// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The chainer task: rounds of [`AuditStore::chain_round`], woken by the
//! requests that wrote intake rows and by a slow poll for the rows other
//! replicas left (#5745).
//!
//! It runs on the audit runtime, never on a worker that serves requests
//! (#5770). A wake of an idle chainer starts a round [`GATHER`] later, and
//! while requests keep writing, rounds start one [`ROUND`] apart, so one round
//! takes what every request of that window wrote, up to [`BATCH`] per chain:
//! the batch grows with the rate, the head and its signed checkpoint are taken
//! once per round, and an idle server runs no round but the poll's. A round
//! that found the head held by another replica's chainer, or filled its batch,
//! runs again at once. The bounds are our own design, set from the capacity runs of
//! #5745 and #5770 and the performance gate; no specification governs them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use opentelemetry::KeyValue;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::background::Background;
use crate::access_log::chainer::Round;
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

/// How long a round that found the head held waits before the next.
const BUSY: Duration = Duration::from_millis(2);

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
    while !stopping.load(Ordering::Acquire) {
        let began = tokio::time::Instant::now();
        let round = store.chain_round(BATCH, look).await;
        look = Chains::default();
        match round {
            Ok(round) => {
                let chained = round.chained();
                if let Round::Chained { entries, held } = round.access_log
                    && entries > 0
                {
                    metrics
                        .log_chain_hold
                        .record(held.as_secs_f64(), &[KeyValue::new("outcome", "ok")]);
                    metrics.log_chained.add(
                        u64::try_from(entries).unwrap_or(u64::MAX),
                        &[KeyValue::new("chain", "access_log")],
                    );
                }
                if round.system > 0 {
                    metrics
                        .log_chained
                        .add(round.system, &[KeyValue::new("chain", "system")]);
                }
                if chained > 0 {
                    crate::telemetry::metrics::metrics().atna_audit_sent.add(
                        u64::try_from(chained).unwrap_or(u64::MAX),
                        &[KeyValue::new("sink", "store")],
                    );
                    for notify in &delivered {
                        notify.notify_one();
                    }
                }
                if round.access_log == Round::Busy {
                    tokio::time::sleep(BUSY).await;
                    continue;
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
                metrics
                    .log_chain_hold
                    .record(0.0, &[KeyValue::new("outcome", "failed")]);
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
            () = tokio::time::sleep(POLL) => look = Chains::BOTH,
        }
    }
    drain(&store).await;
}

/// Chain what the intake holds before the chainer stops, so a graceful stop
/// leaves no row for the next replica's poll; the stop's timeout bounds it.
async fn drain(store: &AuditStore) {
    loop {
        match store.chain_round(BATCH, Chains::BOTH).await {
            Ok(round) if round.access_log == Round::Busy => tokio::time::sleep(BUSY).await,
            Ok(round) if round.chained() > 0 => {}
            Ok(_) => return,
            Err(error) => {
                tracing::warn!(
                    error = %crate::error::ErrorChain::new(&error),
                    "the audit chainer stopped with rows in the intake; the next chainer takes them"
                );
                return;
            }
        }
    }
}
