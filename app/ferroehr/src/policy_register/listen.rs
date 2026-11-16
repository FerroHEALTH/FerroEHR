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
//! the notification counts as received. [`Listeners`] holds every channel of
//! one database on one `LISTEN` session, apart from the request pools, so the
//! pools may run through a transaction-mode pooler that would end a `LISTEN`
//! ([`crate::db::domain::listen_options`]) and an instance opens one listener
//! connection per database, which [`crate::db::DbConfig::listen_sessions`]
//! counts into its connection budget (#5845). No openEHR spec governs
//! operational policy: our own design.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgListener, PgPoolOptions};

use crate::db::domain::Domain;
use crate::db::{DbConfig, DbError};

/// The channel a register write notifies and every replica listens on.
pub const CHANNEL: &str = "ferroehr_policy_register";

/// The `application_name` of every listener session, which tells an operator
/// a `LISTEN` session from a pooled connection in `pg_stat_activity`.
pub const SESSION_NAME: &str = "ferroehr policy listener";

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
    /// The channel it listens on.
    channel: &'static str,
    /// The domain whose database the writes that notify the channel run in.
    domain: Domain,
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
    /// What the operator metrics read of this listener.
    counts: ListenerCounts,
}

/// The counts the operator metrics read of one listener
/// ([`crate::telemetry::access_path`]). No openEHR spec governs them: our own
/// design (#5591).
#[derive(Debug, Default)]
struct ListenerCounts {
    /// The connections lost after the listener had connected.
    losses: AtomicU64,
    /// The reads made because a notification marked the copy stale.
    reads_notified: AtomicU64,
    /// The reads made because the listener was down.
    reads_unlistened: AtomicU64,
    /// The Unix time in milliseconds of the newest notification; `0` before
    /// the first.
    last_notified_ms: AtomicI64,
}

impl Listened {
    /// The listener state of `channel`, notified in `domain`'s database, not
    /// yet listening, so every request reads until [`Listeners::spawn`]
    /// connects it.
    #[must_use]
    pub fn new(channel: &'static str, domain: Domain) -> Self {
        Self {
            channel,
            domain,
            listening: AtomicBool::new(false),
            stale: AtomicBool::new(false),
            notified: AtomicU64::new(0),
            wake: tokio::sync::Notify::new(),
            subscriber: None,
            counts: ListenerCounts::default(),
        }
    }

    /// The listener state of `channel`, as [`Self::new`], whose every
    /// notification `subscriber` drops from its copy before it counts as
    /// received.
    #[must_use]
    pub fn with_subscriber(
        channel: &'static str,
        domain: Domain,
        subscriber: Arc<dyn Subscriber>,
    ) -> Self {
        Self {
            subscriber: Some(subscriber),
            ..Self::new(channel, domain)
        }
    }

    /// The channel this copy listens on.
    #[must_use]
    pub const fn channel(&self) -> &'static str {
        self.channel
    }

    /// The domain whose database notifies the channel.
    #[must_use]
    pub const fn domain(&self) -> Domain {
        self.domain
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

    /// The session listens again: drop everything and mark the copy stale,
    /// so the notifications missed while it was down are made up, then
    /// trust the copy until the next notification.
    async fn connected(&self) {
        self.drop_all().await;
        self.mark_stale();
        self.listening.store(true, Ordering::Release);
    }

    /// A notification of `payload` arrived on the channel.
    async fn heard(&self, payload: &str) {
        self.drop_notified(payload).await;
        self.mark_stale();
        self.counts
            .last_notified_ms
            .store(jiff::Timestamp::now().as_millisecond(), Ordering::Relaxed);
        self.notified.fetch_add(1, Ordering::AcqRel);
        self.wake.notify_waiters();
    }

    /// The session was lost: every request reads until it listens again.
    async fn lost(&self) {
        self.listening.store(false, Ordering::Release);
        self.counts.losses.fetch_add(1, Ordering::Relaxed);
        self.drop_all().await;
    }

    /// Whether a notification is pending or the listener is down, so the next
    /// request must read; the pending notification is taken.
    pub(crate) fn must_read(&self) -> bool {
        if self.unlistened() {
            return true;
        }
        let stale = self.stale.swap(false, Ordering::AcqRel);
        if stale {
            self.counts.reads_notified.fetch_add(1, Ordering::Relaxed);
        }
        stale
    }

    /// Whether the listener is down, so this request reads the store; such a
    /// read is counted.
    pub(crate) fn unlistened(&self) -> bool {
        let down = !self.listening.load(Ordering::Acquire);
        if down {
            self.counts.reads_unlistened.fetch_add(1, Ordering::Relaxed);
        }
        down
    }

    /// The connections lost after the listener had connected.
    #[must_use]
    pub fn losses(&self) -> u64 {
        self.counts.losses.load(Ordering::Relaxed)
    }

    /// The reads made because a notification marked the copy stale, and
    /// those made because the listener was down.
    #[must_use]
    pub fn reads(&self) -> (u64, u64) {
        (
            self.counts.reads_notified.load(Ordering::Relaxed),
            self.counts.reads_unlistened.load(Ordering::Relaxed),
        )
    }

    /// The moment of the newest notification; `None` before the first.
    #[must_use]
    pub fn last_notified(&self) -> Option<jiff::Timestamp> {
        match self.counts.last_notified_ms.load(Ordering::Relaxed) {
            0 => None,
            // NOTE: a stored instant is one this process read from the clock, so it is
            // always in range; one that were not reads as no notification.
            ms => jiff::Timestamp::from_millisecond(ms).ok(),
        }
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

/// Every listened channel of this replica, grouped by the database whose
/// `LISTEN` session carries it: one session per distinct
/// [`DbConfig::listen_dsn`], however many channels it carries.
///
/// A notification is delivered by the database it was raised in
/// (<https://www.postgresql.org/docs/18/sql-notify.html>), so one session per
/// database hears every channel of that database. No openEHR spec governs the
/// process model: our own design (#5845).
pub struct Listeners {
    /// One entry per session, in the order its first channel was given.
    sessions: Vec<Session>,
}

/// One `LISTEN` session and the copies whose channels it carries.
struct Session {
    /// The connect options of the session; they carry the credential, so
    /// they are never rendered.
    options: PgConnectOptions,
    /// The copies it carries, each on its own channel.
    listened: Vec<Arc<Listened>>,
}

impl fmt::Debug for Listeners {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let channels: Vec<Vec<&str>> = self
            .sessions
            .iter()
            .map(|session| session.listened.iter().map(|l| l.channel).collect())
            .collect();
        f.debug_struct("Listeners")
            .field("sessions", &channels)
            .finish()
    }
}

impl Listeners {
    /// The sessions that carry `listened` under `settings`: the copies whose
    /// domains listen on one DSN share one session.
    ///
    /// # Errors
    /// [`DbError::Sqlx`] when a listen DSN does not parse as a `PostgreSQL`
    /// URL.
    pub fn new(settings: &DbConfig, listened: &[Arc<Listened>]) -> Result<Self, DbError> {
        let mut keyed: Vec<(&str, Session)> = Vec::new();
        for copy in listened {
            let dsn = settings.listen_dsn(copy.domain);
            if let Some((_, session)) = keyed.iter_mut().find(|(key, _)| *key == dsn) {
                session.listened.push(Arc::clone(copy));
            } else {
                let options = crate::db::domain::listen_options(settings, copy.domain)?;
                keyed.push((
                    dsn,
                    Session {
                        options,
                        listened: vec![Arc::clone(copy)],
                    },
                ));
            }
        }
        Ok(Self {
            sessions: keyed.into_iter().map(|(_, session)| session).collect(),
        })
    }

    /// One session on `options` carrying every channel of `listened`, for a
    /// caller that holds connect options and no configuration.
    #[must_use]
    pub fn on(options: PgConnectOptions, listened: &[Arc<Listened>]) -> Self {
        Self {
            sessions: vec![Session {
                options,
                listened: listened.to_vec(),
            }],
        }
    }

    /// The `LISTEN` sessions these copies take, one connection each.
    #[must_use]
    pub fn sessions(&self) -> usize {
        self.sessions.len()
    }

    /// Listen on every channel on this replica's behalf, one detached task
    /// per session that ends with the runtime and connects again whenever its
    /// connection is lost.
    pub fn spawn(self) {
        for session in self.sessions {
            tokio::spawn(listen(session));
        }
    }
}

/// The loop of one session: connect, listen on every channel it carries,
/// mark each copy stale so the reads missed while disconnected are made up,
/// then mark the copy a notification names stale; on any failure, fall back
/// to reading on every request and connect again.
#[expect(
    clippy::infinite_loop,
    reason = "a detached task that lives as long as the runtime"
)]
async fn listen(session: Session) {
    let mut channels: Vec<&'static str> = session.listened.iter().map(|l| l.channel).collect();
    channels.sort_unstable();
    channels.dedup();
    loop {
        match connect(&channels, &session.options).await {
            Ok(mut listener) => {
                for copy in &session.listened {
                    copy.connected().await;
                }
                loop {
                    match listener.try_recv().await {
                        Ok(Some(notification)) => {
                            let named = session
                                .listened
                                .iter()
                                .filter(|copy| copy.channel == notification.channel());
                            for copy in named {
                                copy.heard(notification.payload()).await;
                            }
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(?channels, %error, "the policy listener failed");
                            break;
                        }
                    }
                }
                for copy in &session.listened {
                    copy.lost().await;
                }
            }
            Err(error) => {
                tracing::warn!(?channels, %error, "the policy listener could not connect");
            }
        }
        tokio::time::sleep(RECONNECT).await;
    }
}

/// A listener on every one of `channels`, on one connection made with
/// `options` and named [`SESSION_NAME`] that no pool shares, the way
/// `PgListener::connect` makes it from a URL.
async fn connect(
    channels: &[&'static str],
    options: &PgConnectOptions,
) -> Result<PgListener, sqlx::Error> {
    let session = PgPoolOptions::new()
        .max_connections(1)
        .min_connections(0)
        .max_lifetime(None)
        .idle_timeout(None)
        .connect_with(options.clone().application_name(SESSION_NAME))
        .await?;
    let mut listener = PgListener::connect_with(&session).await?;
    listener.ignore_pool_close_event(true);
    listener.listen_all(channels.iter().copied()).await?;
    Ok(listener)
}
