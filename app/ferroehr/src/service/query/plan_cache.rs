// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Bounded AQL plan cache (the parse/plan overhead item).
//!
//! No openEHR spec governs this cache — our own performance design/extension.
//! Two properties keep it semantically identical to the uncached path:
//!
//! * Nothing request-specific is keyed, and the IR stores nothing of a request.
//!   The cached [`QueryIr`] records only the names of the `$parameters` it references ([`QueryIr::params`]);
//!   the query service re-runs [`crate::aql::check_params`] against the caller's
//!   bindings on every execution and binds the values, paging and scope
//!   downstream, so two callers of the same text with different parameters get
//!   independent results from the one shared plan.
//! * Terminology-resolving plans are never cached. A query whose `WHERE` uses
//!   `TERMINOLOGY(…)` (QUERY master03 §TERMINOLOGY) has its value lists resolved
//!   through the terminology service at plan time
//!   ([`crate::aql::terminology::expand_matches`]), and that resolution may
//!   differ on a later execution, so the caller excludes such a plan. There is
//!   no staleness window rather than a TTL that could serve a stale expansion.
//!
//! Each cached plan carries the statement shapes it has been executed in
//! ([`Shapes`]), so a repeated request of one shape binds its values without
//! building its SQL. A shape is matched on every input of the build, the
//! values its slots carry aside, so a request reads only a shape its own build
//! would have rendered.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use moka::future::Cache;

use crate::aql::ir::QueryIr;
use crate::aql::sql::shape::Shapes;

/// A lowered plan, with the statement shapes of the plan when it is cached.
#[derive(Debug)]
pub struct Plan {
    /// The lowered query.
    pub ir: QueryIr,
    /// The statement shapes of a cached plan; `None` for a plan that is
    /// lowered for one request only.
    pub shapes: Option<Shapes>,
}

impl Plan {
    /// A plan to cache, its shapes none known yet.
    #[must_use]
    pub fn cached(ir: QueryIr) -> Self {
        Self {
            shapes: Some(Shapes::new(&ir)),
            ir,
        }
    }

    /// A plan for one request.
    #[must_use]
    pub fn once(ir: QueryIr) -> Self {
        Self { ir, shapes: None }
    }
}

/// A point-in-time view of the plan cache's activity, for observability and
/// tests (`entries` is `moka`'s eventually-consistent estimate).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PlanCacheStats {
    /// Lookups served from the cache.
    pub hits: u64,
    /// Lookups that found nothing (including every lookup when disabled).
    pub misses: u64,
    /// Estimated number of plans currently held.
    pub entries: u64,
}

/// A shared, cloneable bounded cache of lowered AQL query plans keyed on the
/// query text.
#[derive(Clone)]
pub struct PlanCache {
    /// `None` when the cache is disabled (capacity `0`).
    inner: Option<Cache<String, Arc<Plan>>>,
    hits: Arc<AtomicU64>,
    misses: Arc<AtomicU64>,
}

impl std::fmt::Debug for PlanCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PlanCache")
            .field("enabled", &self.inner.is_some())
            .field("hits", &self.hits.load(Ordering::Relaxed))
            .field("misses", &self.misses.load(Ordering::Relaxed))
            .finish()
    }
}

impl Default for PlanCache {
    /// Holds up to 256 distinct plans — bounded so a churn of one-off ad-hoc
    /// queries cannot grow the cache without limit.
    ///
    /// The effective capacity comes from `[record.query].plan_cache_capacity`
    /// ([`super::config::QueryConfig`]), applied when the binary builds the service; a
    /// bare service (tests/embeddings) uses this default. No openEHR spec
    /// governs this — our own tuning knob.
    fn default() -> Self {
        Self::new(256)
    }
}

impl PlanCache {
    /// A cache holding up to `capacity` distinct plans; `capacity == 0`
    /// disables it (every lookup misses, nothing is stored).
    #[must_use]
    pub fn new(capacity: u64) -> Self {
        let inner = (capacity > 0).then(|| Cache::builder().max_capacity(capacity).build());
        Self {
            inner,
            hits: Arc::new(AtomicU64::new(0)),
            misses: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The cached plan for `aql`, or `None` on a miss (or when disabled).
    /// Records the hit/miss both on the [`AQL_PLAN_CACHE_EVENTS`](crate::telemetry::metrics::AQL_PLAN_CACHE_EVENTS) counter and
    /// on the in-process [`PlanCache::stats`] view.
    pub async fn get(&self, aql: &str) -> Option<Arc<Plan>> {
        let hit = match &self.inner {
            Some(cache) => cache.get(aql).await,
            None => None,
        };
        if hit.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
            crate::telemetry::metrics::metrics()
                .aql_plan_cache_events
                .add(1, &[opentelemetry::KeyValue::new("event", "hit")]);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            crate::telemetry::metrics::metrics()
                .aql_plan_cache_events
                .add(1, &[opentelemetry::KeyValue::new("event", "miss")]);
        }
        hit
    }

    /// Store the lowered `plan` under the query text `aql`. A no-op when the
    /// cache is disabled. The caller must only insert a plan that is a pure
    /// function of the query text — in particular, one with no resolved
    /// terminology operand (see the module docs).
    pub async fn insert(&self, aql: String, plan: Arc<Plan>) {
        if let Some(cache) = &self.inner {
            cache.insert(aql, plan).await;
        }
    }

    /// Runs the cache's pending upkeep ([`crate::service::upkeep`]).
    pub(crate) async fn upkeep(&self) {
        if let Some(inner) = &self.inner {
            inner.run_pending_tasks().await;
        }
    }

    /// How many plans the cache holds, and their bytes (#5926).
    #[must_use]
    pub fn held(&self) -> crate::memory::Held {
        self.inner
            .as_ref()
            .map(crate::memory::held)
            .unwrap_or_default()
    }

    /// A snapshot of the cache's activity (observability + tests).
    #[must_use]
    pub fn stats(&self) -> PlanCacheStats {
        let entries = self.inner.as_ref().map_or(0, Cache::entry_count);
        PlanCacheStats {
            hits: self.hits.load(Ordering::Relaxed),
            misses: self.misses.load(Ordering::Relaxed),
            entries,
        }
    }
}

impl crate::memory::Weigh for Plan {
    fn heap(&self) -> usize {
        // NOTE: our own design (#5926), a plan weighs its Debug rendering, since its IR is too deep to walk.
        crate::memory::debug_bytes(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_query::parser::parse_str;

    /// Lower a parameterless query to a cached plan for the cache tests.
    fn ir(q: &str) -> Arc<Plan> {
        let ast = parse_str(q).expect("parse");
        Arc::new(Plan::cached(
            crate::aql::lower_query(&ast, crate::config::profile::SpecProfile::default())
                .expect("lower"),
        ))
    }

    const Q1: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c";
    const Q2: &str = "SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c LIMIT 5";

    #[tokio::test]
    async fn hit_after_insert_and_miss_when_absent() {
        let cache = PlanCache::new(16);
        assert!(cache.get(Q1).await.is_none(), "cold lookup misses");

        let plan = ir(Q1);
        cache.insert(Q1.to_owned(), Arc::clone(&plan)).await;

        let got = cache.get(Q1).await.expect("warm lookup hits");
        assert!(Arc::ptr_eq(&got, &plan), "the exact cached Arc is returned");
        assert!(
            cache.get(Q2).await.is_none(),
            "a different text still misses"
        );

        let stats = cache.stats();
        assert_eq!(stats.hits, 1);
        assert_eq!(stats.misses, 2);
    }

    #[tokio::test]
    async fn disabled_cache_never_stores() {
        let cache = PlanCache::new(0);
        cache.insert(Q1.to_owned(), ir(Q1)).await;
        assert!(
            cache.get(Q1).await.is_none(),
            "a disabled cache always misses"
        );
        let stats = cache.stats();
        assert_eq!(stats.entries, 0);
        assert_eq!(stats.hits, 0);
        assert_eq!(stats.misses, 1);
    }

    #[tokio::test]
    async fn capacity_is_bounded() {
        let cap = 4;
        let cache = PlanCache::new(cap);
        for n in 1..=32 {
            let q = format!("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c LIMIT {n}");
            cache.insert(q.clone(), ir(&q)).await;
        }
        if let Some(inner) = &cache.inner {
            inner.run_pending_tasks().await;
            assert!(
                inner.entry_count() <= cap,
                "entry count {} must not exceed the {cap}-plan capacity",
                inner.entry_count()
            );
        }
    }

    /// A plan the cap evicted is lowered again from its text into the same
    /// plan, so the cap changes no answer (#5926).
    #[tokio::test]
    async fn an_evicted_plan_lowers_again_into_the_same_plan() {
        let cap = 4;
        let cache = PlanCache::new(cap);
        let queries: Vec<String> = (1..=32)
            .map(|n| format!("SELECT c/uid/value FROM EHR e CONTAINS COMPOSITION c LIMIT {n}"))
            .collect();
        for q in &queries {
            cache.insert(q.clone(), ir(q)).await;
        }
        if let Some(inner) = &cache.inner {
            inner.run_pending_tasks().await;
        }
        let mut lowered = 0;
        for q in &queries {
            let plan = if let Some(plan) = cache.get(q).await {
                plan
            } else {
                lowered += 1;
                let plan = ir(q);
                cache.insert(q.clone(), Arc::clone(&plan)).await;
                plan
            };
            assert_eq!(
                format!("{:?}", plan.ir),
                format!("{:?}", ir(q).ir),
                "{q}: the plan its text lowers to"
            );
        }
        assert!(lowered > 0, "the cap evicted some plans");
        if let Some(inner) = &cache.inner {
            inner.run_pending_tasks().await;
        }
        let held = cache.held();
        assert!(held.entries <= cap, "{held:?} within {cap}");
        assert!(held.bytes > 0, "{held:?} weighs its plans");
    }

    #[tokio::test]
    async fn clones_share_one_cache() {
        let cache = PlanCache::new(16);
        let clone = cache.clone();
        cache.insert(Q1.to_owned(), ir(Q1)).await;
        assert!(
            clone.get(Q1).await.is_some(),
            "an insert through one handle is visible through a clone",
        );
        // The shared counters see the clone's hit.
        assert_eq!(cache.stats().hits, 1);
    }
}
