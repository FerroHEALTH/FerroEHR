// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype specialisation-lineage index over the ADL2/OPT2 artefact
//! store — the read the AQL planner takes before lowering a query.
//!
//! BASE Artefact Identification, Referencing §Supporting Archetype-based
//! Querying: "for
//! specialised archetypes, the specialisation lineage can only be obtained from
//! the operational form of the archetype, found in the template used to create
//! the data". The `specialize` parent of every uploaded artefact is extracted
//! by the `openehr-adl` engine at validation time and stored alongside it
//! (`adl2_artefact.parent_hrid`), so resolving the lineage is one small indexed
//! read rather than a re-parse of every stored source.
//!
//! No openEHR spec governs the caching: our own design. The index is memoised
//! in the replica's generation of definition copies
//! ([`crate::service::definition::copies`]), which a request replaces when it
//! reads a newer definition revision, so an AQL execution does not pay the
//! read per query and never widens a predicate with a superseded graph.

use std::sync::Arc;

use crate::aql::lineage::ArchetypeLineage;
use crate::error::ServiceError;
use crate::service::FerroEhrService;
use crate::storage::adl2_repo;
use crate::storage::error::StorageError;

/// In-process memo of the resolved [`ArchetypeLineage`]. Single-entry (the
/// index is store-wide, not per-key).
pub(crate) type ArchetypeLineageCache = moka::future::Cache<(), Arc<ArchetypeLineage>>;

/// Build the lineage memo of one generation of definition copies.
pub(crate) fn archetype_lineage_cache() -> ArchetypeLineageCache {
    moka::future::Cache::builder().max_capacity(1).build()
}

impl FerroEhrService {
    /// The stored archetype specialisation graph, memoised in the current
    /// generation of definition copies.
    ///
    /// # Errors
    ///
    /// `service_overloaded` (`503`) when the artefact store does not answer:
    /// a query answered with a narrower index would match fewer rows than the
    /// store holds, so it is refused instead.
    pub(crate) async fn archetype_lineage(&self) -> Result<Arc<ArchetypeLineage>, ServiceError> {
        let generation = self.definitions.current();
        // `try_get_with` is moka's single-flight: concurrent misses share one
        // artefact-store read instead of stampeding it.
        let loading = generation
            .lineage
            .try_get_with((), self.load_archetype_lineage());
        crate::db::acquire::awaiting(loading)
            .await
            .map_err(|error| {
                ServiceError::overloaded("the archetype lineage could not be read", error)
            })
    }

    /// Read every stored `specialize` edge and resolve it into the index.
    async fn load_archetype_lineage(&self) -> Result<Arc<ArchetypeLineage>, StorageError> {
        let edges = adl2_repo::parent_edges(&self.pool).await?;
        Ok(Arc::new(ArchetypeLineage::from_parent_edges(edges)))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use super::archetype_lineage_cache;
    use crate::aql::lineage::ArchetypeLineage;
    use crate::memory::held;

    /// The memo holds one index at most, and an index it dropped is read
    /// again as the store holds it (#5926).
    #[tokio::test]
    async fn the_memo_holds_one_index_and_reads_a_dropped_one_again() {
        let cache = archetype_lineage_cache();
        let reads = AtomicUsize::new(0);
        for round in 0..4 {
            let index = cache
                .get_with((), async {
                    reads.fetch_add(1, Ordering::Relaxed);
                    Arc::new(ArchetypeLineage::from_parent_edges([(
                        "org.a::openEHR-EHR-OBSERVATION.hdl_result.v1.0.0",
                        "openEHR-EHR-OBSERVATION.lipid_panel.v1.2.0",
                    )]))
                })
                .await;
            assert!(!index.is_empty(), "round {round}: the store's index");
            cache.run_pending_tasks().await;
            let memo = held(&cache);
            assert_eq!(memo.entries, 1, "round {round}: {memo:?}");
            assert!(memo.bytes > 0, "{memo:?}");
            cache.invalidate(&()).await;
        }
        assert_eq!(
            reads.load(Ordering::Relaxed),
            4,
            "each dropped index is read again"
        );
    }
}
