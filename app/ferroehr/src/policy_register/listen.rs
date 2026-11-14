// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The invalidation of operational policy held in memory: every replica
//! listens on a PostgreSQL channel, and a policy write notifies it when it
//! commits.
//!
//! PostgreSQL delivers a `NOTIFY` to every listening session when the
//! notifying transaction commits, and to none when it rolls back
//! (<https://www.postgresql.org/docs/18/sql-notify.html>). A notification
//! marks the copy stale, and the next request reads it again; while the
//! listener is not connected, every request reads it, so a lost notification
//! never serves an outdated policy. The admin registers listen on
//! [`CHANNEL`], the policy grants and the `EHR_ACCESS` projection on their
//! own channels; a [`Subscriber`] drops what each notification names before
//! the notification counts as received. No openEHR spec governs operational
//! policy: our own design.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgListener;

/// The channel a register write notifies and every replica listens on.
pub const CHANNEL: &str = "ferroehr_policy_register";

/// The pause before a lost listener connects again.
const RECONNECT: Duration = Duration::from_secs(1);

/// The future a [`Subscriber`] answers with.
pub type Dropping<'a> = Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

/// What a copy keyed by the notification payload drops when its listener
/// hears of a change.
pub trait Subscriber: fmt::Debug + Send + Sync {
    /// Drops what the notification `payload` names.
    fn notified<'a>(&'a self, payload: &'a str) -> Dropping<'a>;

    /// Drops everything: the listener connected or lost its connection, so
    /// notifications may have been missed.
    fn reset(&self) -> Dropping<'_>;
}

/// Whether this replica's copy of one kind of policy may be trusted: the
/// listener on its channel, and the notifications it has received.
#[derive(Debug)]
pub struct Listened {
    /// The pool the listener connects through.
    pool: PgPool,
    /// The channel it listens on.
    channel: &'static str,
    /// Whether the listener is connected, so a request may trust the copy
    /// until a notification marks it stale.
    listening: AtomicBool,
    /// Whether a notification arrived that no read has made up yet.
    stale: AtomicBool,
    /// The notifications received, and the wake of a task waiting on one.
    notified: AtomicU64,
    wake: tokio::sync::Notify,
    /// The copy each notification's payload is dropped from, if any.
    subscriber: Option<Arc<dyn Subscriber>>,
}

impl Listened {
    /// The listener state of `channel` over `pool`, not yet listening, so
    /// every request reads until [`Self::spawn_listener`] connects.
    #[must_use]
    pub fn new(pool: PgPool, channel: &'static str) -> Self {
        Self {
            pool,
            channel,
            listening: AtomicBool::new(false),
            stale: AtomicBool::new(false),
            notified: AtomicU64::new(0),
            wake: tokio::sync::Notify::new(),
            subscriber: None,
        }
    }

    /// The listener state of `channel` over `pool`, as [`Self::new`], whose
    /// every notification `subscriber` drops from its copy before it counts
    /// as received.
    #[must_use]
    pub fn with_subscriber(
        pool: PgPool,
        channel: &'static str,
        subscriber: Arc<dyn Subscriber>,
    ) -> Self {
        Self {
            subscriber: Some(subscriber),
            ..Self::new(pool, channel)
        }
    }

    /// Lets the subscriber drop what `payload` names.
    async fn drop_notified(&self, payload: &str) {
        if let Some(subscriber) = &self.subscriber {
            subscriber.notified(payload).await;
        }
    }

    /// Lets the subscriber drop everything.
    async fn drop_all(&self) {
        if let Some(subscriber) = &self.subscriber {
            subscriber.reset().await;
        }
    }

    /// Listen on the channel on this replica's behalf, in a detached task
    /// that ends with the runtime, connecting again whenever the connection
    /// is lost.
    pub fn spawn_listener(self: &Arc<Self>) {
        tokio::spawn(listen(Arc::clone(self)));
    }

    /// Whether a notification is pending or the listener is down, so the next
    /// request must read; the pending notification is taken.
    pub(crate) fn must_read(&self) -> bool {
        !self.listening.load(Ordering::Acquire) || self.stale.swap(false, Ordering::AcqRel)
    }

    /// Mark the copy stale, so the next request reads it.
    pub(crate) fn mark_stale(&self) {
        self.stale.store(true, Ordering::Release);
    }

    /// Whether the listener is connected; while it is not, every request
    /// reads.
    #[must_use]
    pub fn is_listening(&self) -> bool {
        self.listening.load(Ordering::Acquire)
    }

    /// The notifications this replica has received.
    #[must_use]
    pub fn notifications(&self) -> u64 {
        self.notified.load(Ordering::Acquire)
    }

    /// Wait until this replica has received more than `seen` notifications,
    /// at most `within`; whether it has.
    pub async fn notified_after(&self, seen: u64, within: Duration) -> bool {
        tokio::time::timeout(within, async {
            loop {
                let woken = self.wake.notified();
                if self.notifications() > seen {
                    return;
                }
                woken.await;
            }
        })
        .await
        .is_ok()
    }
}

/// The listener loop: connect, listen, mark the copy stale so the reads
/// missed while disconnected are made up, then mark it stale on every
/// notification; on any failure, fall back to reading on every request and
/// connect again.
#[expect(
    clippy::infinite_loop,
    reason = "a detached task that lives as long as the runtime"
)]
async fn listen(state: Arc<Listened>) {
    let channel = state.channel;
    loop {
        match connect(&state).await {
            Ok(mut listener) => {
                state.drop_all().await;
                state.mark_stale();
                state.listening.store(true, Ordering::Release);
                loop {
                    match listener.try_recv().await {
                        Ok(Some(notification)) => {
                            state.drop_notified(notification.payload()).await;
                            state.mark_stale();
                            state.notified.fetch_add(1, Ordering::AcqRel);
                            state.wake.notify_waiters();
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(channel, %error, "the policy listener failed");
                            break;
                        }
                    }
                }
                state.listening.store(false, Ordering::Release);
                state.drop_all().await;
            }
            Err(error) => {
                tracing::warn!(channel, %error, "the policy listener could not connect");
            }
        }
        tokio::time::sleep(RECONNECT).await;
    }
}

/// A listener on the channel over the pool.
async fn connect(state: &Listened) -> Result<PgListener, sqlx::Error> {
    let mut listener = PgListener::connect_with(&state.pool).await?;
    listener.listen(state.channel).await?;
    Ok(listener)
}
