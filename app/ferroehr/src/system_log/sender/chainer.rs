// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The chainer task: rounds of [`AuditStore::chain_round`] as long as the
//! intake holds rows, woken by each request that wrote one and by a poll for
//! the rows other replicas wrote (#5745).
//!
//! It runs on the audit runtime, never on a worker that serves requests
//! (#5770). A round batches whatever the intake holds when it starts, up to
//! [`BATCH`], so the batch grows with the rate, the head is taken once per
//! batch, and a round is bounded in time. A round that finds the head held by another replica's chainer
//! waits [`BUSY`] and tries again. The bounds are our own design, set from the
//! capacity runs of #5745; no specification governs them.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use opentelemetry::KeyValue;
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::background::Background;
use crate::access_log::chainer::Round;
use crate::system_log::store::AuditStore;

/// The most intake rows of each chain one round takes.
const BATCH: u32 = 512;

/// How long the chainer waits for a wake before it looks at the intake
/// again, for the rows other replicas wrote.
const IDLE: Duration = Duration::from_millis(50);

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
    while !stopping.load(Ordering::Acquire) {
        match store.chain_round(BATCH).await {
            Ok(round) => {
                let chained = round.chained();
                if let Round::Chained { entries, held } = round.access_log {
                    metrics
                        .log_chain_hold
                        .record(held.as_secs_f64(), &[KeyValue::new("outcome", "ok")]);
                    metrics.log_chained.add(
                        u64::try_from(entries).unwrap_or(u64::MAX),
                        &[KeyValue::new("chain", "access_log")],
                    );
                }
                metrics
                    .log_chained
                    .add(round.system, &[KeyValue::new("chain", "system")]);
                if chained > 0 {
                    crate::telemetry::metrics::metrics().atna_audit_sent.add(
                        u64::try_from(chained).unwrap_or(u64::MAX),
                        &[KeyValue::new("sink", "store")],
                    );
                    for notify in &delivered {
                        notify.notify_one();
                    }
                    continue;
                }
                if round.access_log == Round::Busy {
                    tokio::time::sleep(BUSY).await;
                    continue;
                }
                tokio::select! {
                    () = wake.notified() => {}
                    () = tokio::time::sleep(IDLE) => {}
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
            }
        }
    }
}
