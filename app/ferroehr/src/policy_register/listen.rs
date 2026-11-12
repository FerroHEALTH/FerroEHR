// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The invalidation of the registers in force: every replica listens on one
//! PostgreSQL channel, and a register write notifies it when it commits.
//!
//! PostgreSQL delivers a `NOTIFY` to every listening session when the
//! notifying transaction commits, and to none when it rolls back
//! (<https://www.postgresql.org/docs/18/sql-notify.html>). A notification
//! marks the registers stale, and the next request reads them again; while
//! the listener is not connected, every request reads them, so a lost
//! notification never serves an outdated policy. No openEHR spec governs
//! operational policy: our own design.

use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;

use sqlx::postgres::PgListener;

use crate::policy_register::LiveRegisters;

/// The channel a register write notifies and every replica listens on.
pub const CHANNEL: &str = "ferroehr_policy_register";

/// The pause before a lost listener connects again.
const RECONNECT: Duration = Duration::from_secs(1);

impl LiveRegisters {
    /// Listen for register writes on every replica's behalf, in a detached
    /// task that ends with the runtime, connecting again whenever the
    /// connection is lost.
    pub fn spawn_listener(self: &Arc<Self>) {
        tokio::spawn(listen(Arc::clone(self)));
    }

    /// Whether a notification is pending or the listener is down, so the next
    /// request must read the registers.
    pub(crate) fn must_read(&self) -> bool {
        !self.listening.load(Ordering::Acquire) || self.stale.swap(false, Ordering::AcqRel)
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

/// The listener loop: connect, listen, mark the registers stale so the reads
/// missed while disconnected are made up, then mark them stale on every
/// notification; on any failure, fall back to reading on every request and
/// connect again.
#[expect(
    clippy::infinite_loop,
    reason = "a detached task that lives as long as the runtime"
)]
async fn listen(registers: Arc<LiveRegisters>) {
    loop {
        match connect(&registers).await {
            Ok(mut listener) => {
                registers.stale.store(true, Ordering::Release);
                registers.listening.store(true, Ordering::Release);
                loop {
                    match listener.try_recv().await {
                        Ok(Some(_notification)) => {
                            registers.stale.store(true, Ordering::Release);
                            registers.notified.fetch_add(1, Ordering::AcqRel);
                            registers.wake.notify_waiters();
                        }
                        Ok(None) => break,
                        Err(error) => {
                            tracing::warn!(%error, "the policy register listener failed");
                            break;
                        }
                    }
                }
                registers.listening.store(false, Ordering::Release);
            }
            Err(error) => {
                tracing::warn!(%error, "the policy register listener could not connect");
            }
        }
        tokio::time::sleep(RECONNECT).await;
    }
}

/// A listener on [`CHANNEL`] over the registers' pool.
async fn connect(registers: &LiveRegisters) -> Result<PgListener, sqlx::Error> {
    let mut listener = PgListener::connect_with(registers.pool()).await?;
    listener.listen(CHANNEL).await?;
    Ok(listener)
}
