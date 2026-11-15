// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The render policy and context the drain writes under, replaced when the
//! access policy's retention or the category register changes, and the
//! retention reaper that reads the period in force on each run.
//!
//! No openEHR spec governs how a running system takes up a new policy: our
//! own design.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

use super::AuditSender;
use crate::system_log::categories::CategoryMapConfig;
use crate::system_log::message::AuditContext;
use crate::system_log::render::RenderPolicy;
use crate::system_log::retention::LogRetention;
use crate::system_log::store::AuditStore;

/// The interval between two runs of the retention reaper.
const REAP_INTERVAL: Duration = Duration::from_hours(1);

/// The render policy in force and the context compiled from it.
#[derive(Debug)]
pub(super) struct LiveContext {
    render: Mutex<RenderPolicy>,
    current: RwLock<Arc<AuditContext>>,
    /// Whether the newest access policy version's retention did not read, so
    /// the reaper purges nothing.
    purge_held: AtomicBool,
}

impl LiveContext {
    /// The context `context`, compiled from `render`.
    pub(super) fn new(render: RenderPolicy, context: AuditContext) -> Self {
        Self {
            render: Mutex::new(render),
            current: RwLock::new(Arc::new(context)),
            purge_held: AtomicBool::new(false),
        }
    }

    /// The context in force.
    pub(super) fn current(&self) -> Arc<AuditContext> {
        // NOTE: a writer swaps one `Arc` whole, so a poisoned lock holds no torn value.
        Arc::clone(&self.current.read().unwrap_or_else(PoisonError::into_inner))
    }

    /// Whether the reaper purges nothing, because the newest access policy
    /// version's retention did not read.
    pub(super) fn purge_held(&self) -> bool {
        self.purge_held.load(Ordering::Acquire)
    }

    /// The access log retention in force.
    pub(super) fn retention(&self) -> LogRetention {
        self.render
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retention
            .clone()
    }

    /// Put the render policy `change` makes of the one in force in force,
    /// compiling its context; a policy that does not compile leaves the one in
    /// force and is logged, since the access policy and the register refuse
    /// such a policy before it is stored.
    fn replace(&self, change: impl FnOnce(&mut RenderPolicy)) {
        let mut render = self.render.lock().unwrap_or_else(PoisonError::into_inner);
        let mut next = render.clone();
        change(&mut next);
        match next.compile() {
            Ok(compiled) => {
                *self.current.write().unwrap_or_else(PoisonError::into_inner) =
                    Arc::new(compiled.context);
                *render = next;
            }
            Err(error) => tracing::error!(
                %error,
                "a render policy does not compile; the policy in force stays"
            ),
        }
    }
}

impl AuditSender {
    /// Write every record from now on under `retention`, the access log
    /// retention of the access policy in force.
    pub fn set_retention(&self, retention: &LogRetention) {
        if let Some(store) = &self.inner.store {
            store.change_render_policy(|policy| policy.retention = retention.clone());
        }
        self.inner
            .context
            .replace(|render| render.retention = retention.clone());
    }

    /// Have the reaper purge nothing while `held`: the newest access policy
    /// version's retention did not read, so no period is known to be the
    /// controller's. Records keep being written under the last retention
    /// that read.
    pub fn hold_purge(&self, held: bool) {
        self.inner.context.purge_held.store(held, Ordering::Release);
    }

    /// Whether the reaper purges nothing ([`Self::hold_purge`]).
    #[must_use]
    pub fn purge_held(&self) -> bool {
        self.inner.context.purge_held()
    }

    /// The access log retention every record is written under.
    #[must_use]
    pub fn retention(&self) -> LogRetention {
        self.inner.context.retention()
    }

    /// Have the drain's context render with `map` as its category map.
    pub(super) fn set_context_categories(&self, map: &CategoryMapConfig) {
        self.inner
            .context
            .replace(|render| render.categories = map.clone());
    }
}

/// The hourly retention reaper, which also runs the access log's retention,
/// keeping each record outside the access log for the period in force.
#[expect(
    clippy::infinite_loop,
    reason = "the retention reaper is a detached background task with no \
              shutdown channel — it ends when the runtime drops the task; \
              declaring `-> !` is not an option because `tokio::spawn` would \
              then need the never type as a type argument, which is unstable"
)]
pub(super) async fn reaper(store: AuditStore, live: Arc<LiveContext>) {
    let access_log = crate::access_log::store::AccessLogStore::new(store.pool.clone());
    loop {
        tokio::time::sleep(REAP_INTERVAL).await;
        if live.purge_held() {
            tracing::error!(
                "the access log retention in force does not read; the reaper purges nothing"
            );
            continue;
        }
        let now = jiff::Timestamp::now();
        let today = now.to_zoned(jiff::tz::TimeZone::UTC).date();
        crate::access_log::expiry::scheduled(&access_log, now).await;
        match store.reap_due(today, live.retention().period).await {
            Ok(0) => {}
            Ok(n) => {
                crate::telemetry::metrics::metrics()
                    .atna_audit_reaped
                    .add(n, &[]);
                tracing::debug!("ATNA audit retention reaped {n} records");
            }
            Err(e) => tracing::warn!("ATNA audit retention reap failed: {e}"),
        }
    }
}
