// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The upkeep of this replica's `moka` caches, run off the request path
//! (#6434).
//!
//! A `moka` cache applies the reads and writes it logged, and evicts and
//! expires entries, on the first call 300 ms or more after its last upkeep
//! (moka 0.12 `LOG_SYNC_INTERVAL_MILLIS`), so that call's request would pay
//! it. A detached task runs every cache's upkeep each [`EVERY`], so a request
//! finds it done. No openEHR spec governs the caches: our own design.

use std::sync::Arc;
use std::time::Duration;

use crate::service::FerroEhrService;

/// How often the upkeep runs: a third of `moka`'s 300 ms interval, so a
/// request finds the interval elapsed only when the runtime stalls the task.
pub const EVERY: Duration = Duration::from_millis(100);

impl FerroEhrService {
    /// Runs the pending upkeep of every cache and copy this replica holds.
    pub async fn upkeep(&self) {
        self.bindings.upkeep().await;
        self.grants.upkeep().await;
        self.ehr_access.upkeep().await;
        self.version_copy.upkeep().await;
        self.plan_cache.upkeep().await;
        self.created_ehr_repr.run_pending_tasks().await;
        let definitions = self.definitions.current();
        definitions.web_templates.upkeep().await;
        definitions.lineage.run_pending_tasks().await;
        if let Some(audit) = &self.audit {
            audit.upkeep().await;
        }
    }

    /// Runs [`Self::upkeep`] every [`EVERY`] in a task the returned handle
    /// stops; an abort between two runs leaves no cache mid-upkeep, and one
    /// during a run resumes it on the cache's next call. Called once the
    /// service is assembled, since a builder that replaces a cache afterwards
    /// leaves the task the old one.
    #[must_use = "the handle stops the upkeep when the server drains"]
    pub fn spawn_upkeep(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        tokio::spawn(upkept(Arc::clone(self)))
    }
}

/// The detached loop behind [`FerroEhrService::spawn_upkeep`].
#[expect(
    clippy::infinite_loop,
    reason = "the task ends when its handle aborts it or the runtime drops it, and `-> !` \
              cannot be spawned on stable"
)]
async fn upkept(service: Arc<FerroEhrService>) {
    let mut ticks = tokio::time::interval(EVERY);
    ticks.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        ticks.tick().await;
        service.upkeep().await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqlx::PgPool;

    use crate::db::domain::DomainPools;
    use crate::service::FerroEhrService;

    /// A service over a pool that never connects; its caches start empty.
    fn service() -> Arc<FerroEhrService> {
        let pool = PgPool::connect_lazy("postgres://localhost/unused").expect("a lazy pool");
        Arc::new(FerroEhrService::new(&DomainPools::from_shared(&pool)))
    }

    #[tokio::test]
    async fn the_upkeep_stops_when_its_handle_aborts_it() {
        let upkeep = service().spawn_upkeep();
        tokio::time::sleep(super::EVERY * 3).await;
        upkeep.abort();
        let stopped = upkeep.await.expect_err("the aborted task does not finish");
        assert!(stopped.is_cancelled(), "{stopped:?}");
    }
}
