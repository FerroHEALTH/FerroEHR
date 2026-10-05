// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An async cache of built [`WebTemplate`]s, keyed by template id.
//!
//! `WebTemplate` construction is deterministic and moderately expensive (a full
//! OPT walk + compaction); the REST `wt+json` endpoint builds each template once
//! and serves the shared [`Arc`] thereafter. Mirrors the `moka::future::Cache`
//! usage in `ferroehr-rest`'s JWKS cache.

use std::sync::Arc;

use moka::future::Cache;

use crate::flat::webtemplate::model::WebTemplate;

/// A shared, cloneable `WebTemplate` cache.
#[derive(Debug, Clone)]
pub struct WebTemplateCache {
    inner: Cache<String, Arc<WebTemplate>>,
}

impl WebTemplateCache {
    /// Create a cache holding up to `capacity` templates.
    #[must_use]
    pub fn new(capacity: u64) -> Self {
        Self {
            inner: Cache::builder().max_capacity(capacity).build(),
        }
    }

    /// Return the cached template for `template_id`, or build it with `build`
    /// (run at most once per key under contention) and cache the result.
    ///
    /// # Errors
    /// Propagates the `build` error (shared across concurrent callers).
    pub async fn get_or_build<F>(
        &self,
        template_id: &str,
        build: F,
    ) -> Result<Arc<WebTemplate>, Arc<crate::flat::error::FlatError>>
    where
        F: FnOnce() -> Result<WebTemplate, crate::flat::error::FlatError>,
    {
        self.inner
            .try_get_with_by_ref(template_id, async { build().map(Arc::new) })
            .await
    }

    /// The cached template for `template_id` **without** building it — the fast
    /// path for callers that must avoid the backing-store round-trip on a hit
    /// (e.g. the per-commit validation path). Counts as an access for the
    /// cache's recency policy, so hot templates stay resident. Returns `None`
    /// on a miss; the caller then loads + [`get_or_build`](Self::get_or_build)s.
    #[must_use]
    pub async fn get(&self, template_id: &str) -> Option<Arc<WebTemplate>> {
        self.inner.get(template_id).await
    }

    /// Drop the cached entry for `template_id` (e.g. on template replacement).
    pub async fn invalidate(&self, template_id: &str) {
        self.inner.invalidate(template_id).await;
    }
}

impl Default for WebTemplateCache {
    fn default() -> Self {
        Self::new(256)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::flat::webtemplate::model::WebTemplate;

    fn stub_template(id: &str) -> WebTemplate {
        let mut tree = crate::flat::webtemplate::model::WebTemplateNode::new(
            "COMPOSITION".to_owned(),
            String::new(),
        );
        tree.id = "root".to_owned();
        tree.min = Some(1);
        tree.max = 1;
        WebTemplate {
            template_id: id.to_owned(),
            sem_ver: None,
            version: "2.3".to_owned(),
            default_language: "en".to_owned(),
            languages: vec!["en".to_owned()],
            tree,
            other_details: indexmap::IndexMap::new(),
        }
    }

    #[tokio::test]
    async fn builds_once_then_serves_from_cache() {
        let cache = super::WebTemplateCache::default();
        let builds = AtomicUsize::new(0);
        let build = || {
            builds.fetch_add(1, Ordering::SeqCst);
            Ok(stub_template("t1"))
        };

        let first = cache.get_or_build("t1", build).await.expect("first build");
        assert_eq!(first.template_id, "t1");

        // Second call is a cache hit: the builder must not run again.
        let second = cache
            .get_or_build("t1", || {
                builds.fetch_add(1, Ordering::SeqCst);
                Ok(stub_template("t1"))
            })
            .await
            .expect("cache hit");
        assert_eq!(second.template_id, "t1");
        assert_eq!(builds.load(Ordering::SeqCst), 1, "build ran more than once");

        // Invalidation forces a rebuild.
        cache.invalidate("t1").await;
        let _third = cache.get_or_build("t1", build).await.expect("rebuild");
        assert_eq!(
            builds.load(Ordering::SeqCst),
            2,
            "invalidation should rebuild"
        );
    }

    #[tokio::test]
    async fn get_is_a_nonbuilding_peek() {
        let cache = super::WebTemplateCache::default();
        // Miss: `get` returns `None` and never builds.
        assert!(cache.get("t1").await.is_none(), "cold `get` is a miss");
        // After a build, `get` serves the cached entry without rebuilding.
        let _built = cache
            .get_or_build("t1", || Ok(stub_template("t1")))
            .await
            .expect("build");
        assert!(cache.get("t1").await.is_some(), "warm `get` is a hit");
        // Invalidation makes `get` miss again.
        cache.invalidate("t1").await;
        assert!(
            cache.get("t1").await.is_none(),
            "`get` misses after invalidation"
        );
    }
}
