// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The audit subsystem's own runtime: one thread that runs the chainer, the
//! forwarders and the store's upkeep, apart from the worker threads that
//! serve requests (#5770).
//!
//! A batch of the chainer or a forwarder's rendering never takes a request's
//! worker, so the request tail does not follow the rhythm of the batches. The
//! runtime has its own pool over the audit store, opened in its own context,
//! so its connections are driven by its own reactor. No specification
//! governs where the audit trail's background work runs: our own design.

use std::future::Future;
use std::thread::JoinHandle;

use sqlx::PgPool;
use tokio::runtime::Handle;
use tokio::sync::oneshot;

use crate::system_log::AuditError;

/// The running audit runtime.
#[derive(Debug)]
pub(super) struct Background {
    handle: Handle,
    stop: Option<oneshot::Sender<()>>,
    thread: Option<JoinHandle<()>>,
}

impl Background {
    /// Start the runtime on a thread of its own.
    ///
    /// # Errors
    /// [`AuditError::Runtime`] when the thread or its runtime cannot start.
    pub(super) fn start() -> Result<Self, AuditError> {
        let (handed, handle) = std::sync::mpsc::channel();
        let (stop, stopped) = oneshot::channel::<()>();
        let thread = std::thread::Builder::new()
            .name("ferroehr-audit".to_owned())
            .spawn(move || {
                let runtime = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime,
                    Err(error) => {
                        drop(handed.send(Err(error)));
                        return;
                    }
                };
                if handed.send(Ok(runtime.handle().clone())).is_err() {
                    return;
                }
                // A handle dropped without a stop leaves the runtime running,
                // as tasks spawned on the request runtime would keep running.
                runtime.block_on(async {
                    if stopped.await.is_err() {
                        std::future::pending::<()>().await;
                    }
                });
            })
            .map_err(AuditError::Runtime)?;
        let handle = handle
            .recv()
            .map_err(|_gone| {
                AuditError::Runtime(std::io::Error::other("the audit runtime thread ended"))
            })?
            .map_err(AuditError::Runtime)?;
        Ok(Self {
            handle,
            stop: Some(stop),
            thread: Some(thread),
        })
    }

    /// Spawn `task` on the audit runtime.
    pub(super) fn spawn<F>(&self, task: F) -> tokio::task::JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        self.handle.spawn(task)
    }

    /// A pool over the audit store of `pool`, opened in the audit runtime's
    /// context.
    pub(super) fn pool_over(&self, pool: &PgPool) -> PgPool {
        let _entered = self.handle.enter();
        crate::db::domain_pool_from(pool, crate::db::domain::Domain::Audit)
    }

    /// End the runtime once the tasks on it have been stopped.
    pub(super) fn stop(mut self) {
        if let Some(stop) = self.stop.take() {
            let _ended = stop.send(());
        }
        // The thread ends with its runtime; it is not joined, so a shutdown
        // never blocks a worker of the request runtime.
        drop(self.thread.take());
    }
}

#[cfg(test)]
mod tests {
    use super::Background;

    /// The chainer, the forwarders and the upkeep run on the audit runtime's
    /// own thread, never on a worker of the runtime that serves requests,
    /// whatever a round costs (#5770).
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn spawned_work_runs_on_the_audit_thread() {
        let background = Background::start().expect("the audit runtime starts");
        let request_worker = std::thread::current().id();
        let ran_on = background
            .spawn(async { std::thread::current() })
            .await
            .expect("the task completes");
        assert_eq!(ran_on.name(), Some("ferroehr-audit"));
        assert_ne!(ran_on.id(), request_worker);
        background.stop();
    }
}
