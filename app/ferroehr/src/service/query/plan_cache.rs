// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Bounded AQL plan cache (the parse/plan overhead item).
//!
//! Parsing an AQL query and lowering it to the typed [`QueryIr`]
//! ([`crate::aql::lower_query`]) is a deterministic function of the query text:
//! no parameter value, paging window, EHR scope or system id is baked into the
//! IR, all of them binding at SQL-build time ([`crate::aql::sql::build`]). This
//! cache holds the lowered plan keyed on the exact query text, sparing every
//! repeat the parse, path analysis and IR lowering.
//!
//! No openEHR spec governs this cache — our own performance design/extension.
//! Two properties keep it semantically identical to the uncached path:
//!
//! * Nothing request-specific is keyed or stored. The cached [`QueryIr`] records
//!   only the names of the `$parameters` it references ([`QueryIr::params`]);
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

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use moka::future::Cache;

use crate::aql::ir::QueryIr;

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
///
/// `moka`'s [`Cache`] is `Arc`-backed, so every clone of the owning service
/// shares one cache (mirroring [`openehr_sdt::flat::cache::WebTemplateCache`]
/// and `crate::service::ehr::access::EhrAccessCache`); the hit/miss counters
/// are shared the same way.
#[derive(Clone)]
pub struct PlanCache {
    /// `None` when the cache is disabled (capacity `0`).
    inner: Option<Cache<String, Arc<QueryIr>>>,
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
    /// The effective capacity comes from `[query].plan_cache_capacity`
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
    pub async fn get(&self, aql: &str) -> Option<Arc<QueryIr>> {
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

    /// Store the lowered `ir` under the query text `aql`. A no-op when the
    /// cache is disabled. The caller must only insert a plan that is a pure
    /// function of the query text — in particular, one with no resolved
    /// terminology operand (see the module docs).
    pub async fn insert(&self, aql: String, ir: Arc<QueryIr>) {
        if let Some(cache) = &self.inner {
            cache.insert(aql, ir).await;
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use openehr_query::parser::parse_str;

    /// Lower a parameterless query to an `Arc<QueryIr>` for the cache tests.
    fn ir(q: &str) -> Arc<QueryIr> {
        let ast = parse_str(q).expect("parse");
        Arc::new(
            crate::aql::lower_query(&ast, crate::config::profile::SpecProfile::default())
                .expect("lower"),
        )
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
