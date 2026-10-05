// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The archetype specialisation-lineage index over the ADL2/OPT2 artefact
//! store — the read the AQL planner takes before lowering a query.
//!
//! AM `Identification` master07 §Supporting Archetype-based Querying: "for
//! specialised archetypes, the specialisation lineage can only be obtained from
//! the operational form of the archetype, found in the template used to create
//! the data". The `specialize` parent of every uploaded artefact is extracted
//! by the `openehr-adl` engine at validation time and stored alongside it
//! (`adl2_artefact.parent_hrid`), so resolving the lineage is one small indexed
//! read rather than a re-parse of every stored source.
//!
//! No openEHR spec governs the caching — our own design/extension: the index is
//! memoised in-process so an AQL execution does not pay the read per query,
//! invalidated on every local artefact write, with a short time-to-live that
//! doubles as the convergence window for an upload made on another instance.

use std::sync::Arc;
use std::time::Duration;

use crate::aql::lineage::ArchetypeLineage;
use crate::service::FerroEhrService;
use crate::service::error::ServiceError;

/// In-process memo of the resolved [`ArchetypeLineage`]. Single-entry (the
/// index is store-wide, not per-key) and `Arc`-backed, so every clone of the
/// service shares it.
pub(crate) type ArchetypeLineageCache = moka::future::Cache<(), Arc<ArchetypeLineage>>;

/// How long a resolved index is reused. Short enough that an ADL2 upload on a
/// sibling instance is picked up promptly, long enough that a query stream
/// costs one artefact-store read per window rather than one per query.
const LINEAGE_TTL: Duration = Duration::from_mins(1);

/// Build the lineage memo (the service constructs one per instance).
pub(crate) fn archetype_lineage_cache() -> ArchetypeLineageCache {
    moka::future::Cache::builder()
        .max_capacity(1)
        .time_to_live(LINEAGE_TTL)
        .build()
}

impl FerroEhrService {
    /// The stored archetype specialisation graph, memoised.
    ///
    /// A read failure is not fatal to querying: the lineage only *widens* an
    /// archetype predicate's matching set, so an unavailable index degrades to
    /// exact + ADL 1.4 concept-prefix matching (the pre-lineage behaviour)
    /// rather than failing the query. The error is logged and an empty index
    /// returned.
    pub(crate) async fn archetype_lineage(&self) -> Arc<ArchetypeLineage> {
        // `try_get_with` is moka's single-flight: concurrent misses share one
        // artefact-store read instead of stampeding it.
        match self
            .archetype_lineage
            .try_get_with((), self.load_archetype_lineage())
            .await
        {
            Ok(lineage) => lineage,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "archetype lineage unavailable; archetype predicates fall back to \
                     exact + ADL 1.4 concept-prefix matching"
                );
                Arc::new(ArchetypeLineage::default())
            }
        }
    }

    /// Read every stored `specialize` edge and resolve it into the index.
    async fn load_archetype_lineage(&self) -> Result<Arc<ArchetypeLineage>, ServiceError> {
        let edges: Vec<(String, String)> = sqlx::query_as(
            "SELECT hrid, parent_hrid FROM adl2_artefact WHERE parent_hrid IS NOT NULL",
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(Arc::new(ArchetypeLineage::from_parent_edges(edges)))
    }

    /// Drop the memoised index after a local artefact write (upload, replace,
    /// delete), so the next query sees the new family immediately.
    pub(super) async fn invalidate_archetype_lineage(&self) {
        self.archetype_lineage.invalidate(&()).await;
    }
}
