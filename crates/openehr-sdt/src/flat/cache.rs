// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An async cache of built [`WebTemplate`]s, keyed by template id.

use std::sync::Arc;

use moka::future::Cache;

use crate::flat::webtemplate::model::WebTemplate;

/// The templates a cache holds by default.
pub const TEMPLATES: u64 = 256;

/// What a `moka` cache spends on each entry beyond its key and value,
/// measured with moka 0.12.16 at about 220 to 250 bytes.
const CACHE_ENTRY: u64 = 256;

/// A built template with its weight.
#[derive(Debug, Clone)]
struct Kept {
    template: Arc<WebTemplate>,
    /// The length of the template's JSON text, which stands for its bytes.
    bytes: u64,
}

impl Kept {
    /// `template` weighed by its JSON text.
    fn of(template: WebTemplate) -> Self {
        let mut counted = Counted(0);
        let bytes = match serde_json::to_writer(&mut counted, &template) {
            // NOTE: our own design, a template that does not serialize weighs what was written of it.
            Ok(()) | Err(_) => counted.0,
        };
        Self {
            template: Arc::new(template),
            bytes,
        }
    }
}

/// A writer that counts the bytes written to it.
struct Counted(u64);

impl std::io::Write for Counted {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let written = u64::try_from(buf.len()).unwrap_or(u64::MAX);
        self.0 = self.0.saturating_add(written);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// A shared, cloneable `WebTemplate` cache.
#[derive(Debug, Clone)]
pub struct WebTemplateCache {
    inner: Cache<String, Kept>,
}

impl WebTemplateCache {
    /// Create a cache holding up to `capacity` templates.
    #[must_use]
    pub fn new(capacity: u64) -> Self {
        Self {
            inner: Cache::builder().max_capacity(capacity).build(),
        }
    }

    /// The cached template for `template_id`, or `build`'s, run at most once per
    /// key; an evicted template is built again.
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
            .try_get_with_by_ref(template_id, async { build().map(Kept::of) })
            .await
            .map(|kept| kept.template)
    }

    /// The cached template for `template_id` without building it; `None` on a
    /// miss. Counts as an access.
    #[must_use]
    pub async fn get(&self, template_id: &str) -> Option<Arc<WebTemplate>> {
        self.inner.get(template_id).await.map(|kept| kept.template)
    }

    /// Drop the cached entry for `template_id` (e.g. on template replacement).
    pub async fn invalidate(&self, template_id: &str) {
        self.inner.invalidate(template_id).await;
    }

    /// Runs the cache's pending upkeep: what `moka` would otherwise run on
    /// the first call 300 ms after its last upkeep.
    pub async fn upkeep(&self) {
        self.inner.run_pending_tasks().await;
    }

    /// How many templates the cache holds, and an estimate of their bytes.
    #[must_use]
    pub fn held(&self) -> (u64, u64) {
        self.inner
            .iter()
            .fold((0_u64, 0_u64), |(entries, bytes), (id, kept)| {
                let id = u64::try_from(id.len()).unwrap_or(u64::MAX);
                let entry = kept.bytes.saturating_add(id).saturating_add(CACHE_ENTRY);
                (entries.saturating_add(1), bytes.saturating_add(entry))
            })
    }
}

impl Default for WebTemplateCache {
    fn default() -> Self {
        Self::new(TEMPLATES)
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
            archetype_refs: None,
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

    /// The cache holds at most its capacity, and a template the cap evicted
    /// is built again with the same answer.
    #[tokio::test]
    async fn the_cap_holds_and_an_evicted_template_builds_again() {
        let cap = 4;
        let cache = super::WebTemplateCache::new(cap);
        let builds = AtomicUsize::new(0);
        let ids: Vec<String> = (0..32).map(|n| format!("t{n}")).collect();
        for round in 0..2 {
            for id in &ids {
                let built = cache
                    .get_or_build(id, || {
                        builds.fetch_add(1, Ordering::SeqCst);
                        Ok(stub_template(id))
                    })
                    .await
                    .expect("the build succeeds");
                assert_eq!(&built.template_id, id, "round {round}");
            }
            cache.inner.run_pending_tasks().await;
            let (entries, bytes) = cache.held();
            assert!(entries <= cap, "{entries} within {cap}");
            assert!(bytes > 0, "the held templates weigh their text");
        }
        assert!(
            builds.load(Ordering::SeqCst) > ids.len(),
            "the evicted templates were built again"
        );
    }
}
