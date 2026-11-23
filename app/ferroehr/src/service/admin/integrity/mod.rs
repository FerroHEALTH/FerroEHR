// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The storage-parity sweep: the two stored copies of every version's content
//! are re-derived and compared.
//!
//! NOTE: no openEHR spec governs storage mechanics — our own design/extension;
//! the storage keeps a version's content twice, as `version.body` (the
//! materialized projection point reads serve) and as the decomposed `node`
//! rows (the AQL index), and their equality is an invariant of the commit path.
//!
//! The read-time signature verification in `crate::versioning::integrity` covers
//! the `body` copy, recomputing at a point read the signature taken over the
//! version's canonical serialized form (RM common
//! `master06-change_control_package.adoc` §Digital Signature), and says nothing
//! about the `node` rows. This sweep reassembles each version from its node rows
//! and compares the result to the stored body, so a tampered or corrupt row in
//! either copy becomes visible.
//!
//! A version whose rows agree is decomposed again from its body and the two row
//! sets compared, so rows an older decomposition wrote are found too.
//!
//! It never logs content: a mismatch is reported by identifier alone.

#![expect(
    clippy::disallowed_types,
    reason = "owner-approved 2026-08-03 (#1694 family 1): stored canonical fragments — a typed \
              round-trip drops forward-compatible keys (the openEHR release strategy: minors are compatible supersets)"
)]

pub mod rebuild;

use std::collections::HashMap;
use std::time::Instant;

use serde_json::Value;
use uuid::Uuid;

use crate::error::ServiceError;
use crate::ids::VoId;
use crate::service::FerroEhrService;
use crate::storage::codec::reassemble;
use crate::storage::integrity::{self, StorageParityScope};
use crate::storage::node_repo::read_version_rows_all;
use crate::storage::row::ReadRow;

/// How many version rows one page of the sweep's cursor query returns.
const PAGE_SIZE: i64 = 500;

/// How many versions' contents are read per round trip.
///
/// Bounds memory to 32 reassembled trees and stored bodies at a time, at two
/// statements per chunk.
const CONTENT_CHUNK: usize = 32;

/// How many mismatches the returned report carries in full.
///
/// Every mismatch is also logged at `warn`, so nothing is lost when a sweep
/// over a badly damaged store passes this cap; the report still counts them
/// all and says it was truncated.
const MAX_REPORTED_MISMATCHES: usize = 1000;

/// Which pseudonymisation domain a sweep is reading.
///
/// A domain is carried on every finding and every count, so a report over half
/// the store never reads as one over all of it.
///
/// NOTE: no openEHR spec governs storage layout — our own design/extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum StorageDomain {
    /// The clinical schema: compositions, `EHR_STATUS`, folders.
    Clinical,
    /// The demographic schema: PARTY versioned objects.
    Demographic,
}

impl StorageDomain {
    /// Returns the stable wire token for this domain.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Clinical => "clinical",
            Self::Demographic => "demographic",
        }
    }
}

/// What the sweep found wrong with one stored version's two content copies.
///
/// NOTE: no openEHR spec governs storage mechanics — our own design/extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StorageParityDefect {
    /// The version has both copies and they are not the same value.
    ContentDiffers,
    /// The version has a stored body but no node rows at all.
    NodesMissing,
    /// The version has node rows that do not reassemble into one tree, so the
    /// node copy cannot be compared to anything.
    NodesUnreadable,
    /// The version is a logical delete (data Void, RM common
    /// `master06-change_control_package.adoc` §Logical Deletion) and therefore
    /// stores no body, yet node rows exist for it.
    UnexpectedNodes,
    /// The rows reassemble into exactly the stored body, but they are not the
    /// rows the current decomposition writes for it: an older decomposition
    /// produced them. The content is intact and every point read serves it
    /// correctly; what is stale is the index over it, so a row-level predicate
    /// bound to a part that now earns its own row does not reach this version
    /// until its rows are rebuilt.
    StaleDecomposition,
    /// Node rows exist that no stored version reads: their version is gone,
    /// in another tier, or reads another version's rows. The node rows carry
    /// no foreign key into `version` (#5969), so this is what reports a row
    /// set the writer or a delete left behind. The mismatch's `kind` is the
    /// RM type of the set's root row.
    OrphanNodes,
}

impl StorageParityDefect {
    /// Returns the stable wire token for this defect.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ContentDiffers => "content_differs",
            Self::NodesMissing => "nodes_missing",
            Self::NodesUnreadable => "nodes_unreadable",
            Self::UnexpectedNodes => "unexpected_nodes",
            Self::StaleDecomposition => "stale_decomposition",
            Self::OrphanNodes => "orphan_nodes",
        }
    }
}

/// One stored version whose two content copies did not pass the sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageParityMismatch {
    /// Which domain's storage the version lives in.
    pub domain: StorageDomain,
    /// The versioned object the version belongs to.
    pub vo_id: Uuid,
    /// The per-object storage commit ordinal of the version.
    pub sys_version: i32,
    /// The `version.kind` discriminator (`COMPOSITION` / `EHR_STATUS` /
    /// `FOLDER` / a demographic PARTY class / …).
    pub kind: String,
    /// What the sweep found wrong with the two copies.
    pub defect: StorageParityDefect,
}

/// What a sweep has read so far, or in total.
///
/// NOTE: no openEHR spec governs storage mechanics — our own design/extension.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct StorageParityCounts {
    /// How many stored versions were read, across both storage tiers.
    pub versions_checked: u64,
    /// How many of them carry a materialized body (a content version).
    pub versions_with_body: u64,
    /// How many of them carry no body (a logical delete).
    pub versions_without_body: u64,
    /// How many mismatches were found.
    pub mismatch_count: u64,
}

/// One event of a running storage-parity sweep.
///
/// NOTE: no openEHR spec governs storage mechanics — our own design/extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StorageParityEvent {
    /// One stored version whose two content copies disagree.
    Mismatch(StorageParityMismatch),
    /// The counts so far, emitted once per enumerated page.
    Progress {
        /// What has been read up to this point.
        counts: StorageParityCounts,
    },
    /// The final counts, emitted once, after the last page.
    Summary {
        /// What the whole sweep read.
        counts: StorageParityCounts,
        /// Wall-clock duration of the sweep, in milliseconds.
        elapsed_ms: u64,
    },
}

/// The outcome of one storage-parity sweep.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageParityReport {
    /// How many stored versions were read, across both storage tiers.
    pub versions_checked: u64,
    /// How many of them carry a materialized body (a content version).
    pub versions_with_body: u64,
    /// How many of them carry no body (a logical delete).
    pub versions_without_body: u64,
    /// How many mismatches were found in total, whatever `mismatches` holds.
    pub mismatch_count: u64,
    /// The mismatches, up to the reporting cap.
    pub mismatches: Vec<StorageParityMismatch>,
    /// Whether `mismatches` was cut short by the cap.
    pub truncated: bool,
    /// Wall-clock duration of the sweep, in milliseconds.
    pub elapsed_ms: u64,
}

impl StorageParityReport {
    /// Returns `true` when every stored version's two content copies agree.
    #[must_use]
    pub fn is_clean(&self) -> bool {
        self.mismatch_count == 0
    }
}

/// One page row of the sweep cursor: the identifiers of a stored version plus
/// whether it carries a body. No content is fetched here.
struct VersionKey {
    vo_id: Uuid,
    sys_version: i32,
    kind: String,
    has_body: bool,
}

/// Whether one version's stored rows are the rows the current decomposition
/// writes for its body.
///
/// A body that does not decompose is not current.
fn rows_are_current(stored: &[ReadRow], body: &Value) -> bool {
    // NOTE: the root identity decides only how a row slims, never its whole
    // fragment or its place in the tree, which is all this compares.
    let root = crate::storage::compact::RootIdentity {
        version_uid: "",
        template_id: None,
    };
    let Ok(rendered) = crate::storage::render::index::render(body) else {
        return false;
    };
    let Ok((fresh, whole)) = crate::storage::render::nodes::decompose(&rendered, &root, true)
    else {
        return false;
    };
    stored.len() == fresh.len()
        && stored
            .iter()
            .zip(fresh.iter().zip(&whole))
            .all(|(stored, (fresh, whole))| {
                stored.num == fresh.num
                    && stored.num_cap == fresh.num_cap
                    && stored.parent_num == fresh.parent_num
                    && stored.path == fresh.path
                    && serde_json::from_str::<Value>(whole).is_ok_and(|w| stored.data == w)
                    && fresh
                        .shared
                        .as_deref()
                        .map(serde_json::from_str::<Value>)
                        .transpose()
                        .ok()
                        == Some(stored.shared.clone())
            })
}

impl FerroEhrService {
    /// Starts a sweep the caller advances itself, one batch of events at a
    /// time.
    ///
    /// NOTE: no openEHR spec governs storage mechanics — our own
    /// design/extension.
    #[must_use]
    pub fn storage_parity_sweep(&self, scope: StorageParityScope) -> StorageParitySweep<'_> {
        StorageParitySweep {
            service: self,
            remaining: vec![StorageDomain::Demographic],
            domain: StorageDomain::Clinical,
            scope,
            cursor: None,
            page: Vec::new().into_iter(),
            counts: StorageParityCounts::default(),
            started: Instant::now(),
            page_finished: false,
            orphans_read: false,
            done: false,
        }
    }

    /// Re-derives every stored version's content from its `node` rows and
    /// compares it to the materialized `version.body`, reporting every
    /// disagreement.
    ///
    /// Reads both storage tiers. It takes no lock and holds no transaction; a
    /// commit writes both copies in one transaction, so no version is seen
    /// half-written.
    ///
    /// NOTE: no openEHR spec governs storage mechanics — our own
    /// design/extension (the module docs carry the full derivation).
    ///
    /// # Errors
    /// - `exception` — a database fault while enumerating or reading versions.
    ///   A version whose node rows are unreadable is a REPORTED mismatch, not
    ///   an error: one damaged record must not abort the sweep that found it.
    pub async fn verify_storage_parity(
        &self,
        scope: StorageParityScope,
    ) -> Result<StorageParityReport, ServiceError> {
        let mut sweep = self.storage_parity_sweep(scope);
        let mut report = StorageParityReport {
            versions_checked: 0,
            versions_with_body: 0,
            versions_without_body: 0,
            mismatch_count: 0,
            mismatches: Vec::new(),
            truncated: false,
            elapsed_ms: 0,
        };
        while let Some(events) = sweep.next_batch().await? {
            for event in events {
                match event {
                    StorageParityEvent::Mismatch(mismatch) => {
                        if report.mismatches.len() < MAX_REPORTED_MISMATCHES {
                            report.mismatches.push(mismatch);
                        } else {
                            report.truncated = true;
                        }
                    }
                    // The aggregated form answers once, at the end, so a
                    // running count has nobody to tell.
                    StorageParityEvent::Progress { .. } => {}
                    StorageParityEvent::Summary { counts, elapsed_ms } => {
                        report.versions_checked = counts.versions_checked;
                        report.versions_with_body = counts.versions_with_body;
                        report.versions_without_body = counts.versions_without_body;
                        report.mismatch_count = counts.mismatch_count;
                        report.elapsed_ms = elapsed_ms;
                    }
                }
            }
        }
        Ok(report)
    }

    /// The pool that reaches one domain's storage.
    fn domain_pool(&self, domain: StorageDomain) -> &sqlx::PgPool {
        match domain {
            StorageDomain::Clinical => &self.pool,
            StorageDomain::Demographic => &self.demographic_pool,
        }
    }

    /// Compare one chunk of versions in two round trips: the reassembled node
    /// content of all of them, then their stored bodies.
    ///
    /// Returns one verdict per input key, in input order, so the caller's
    /// counters follow the enumeration rather than the map's iteration.
    async fn chunk_parity_defects<'k>(
        &self,
        domain: StorageDomain,
        chunk: &'k [VersionKey],
    ) -> Result<Vec<(&'k VersionKey, Option<StorageParityDefect>)>, ServiceError> {
        let keys: Vec<(VoId, i32)> = chunk
            .iter()
            .map(|key| (VoId(key.vo_id), key.sys_version))
            .collect();
        let rows = read_version_rows_all(self.domain_pool(domain), &keys).await?;
        let bodies = self.chunk_bodies(domain, &keys).await?;

        Ok(chunk
            .iter()
            .map(|key| {
                let id = (VoId(key.vo_id), key.sys_version);
                // A set of rows that does not form one tree is a verdict, never
                // a propagated error: the sweep exists to find damaged records,
                // so it reports them and keeps going.
                let nodes = rows.get(&id).map(|rows| reassemble(rows));
                let defect = match (key.has_body, bodies.get(&id), nodes) {
                    // A logical delete (RM common master06 §Logical Deletion)
                    // stores no body, so any node row for it is unexpected.
                    (false, _, Some(_)) => Some(StorageParityDefect::UnexpectedNodes),
                    // Nothing to compare: a logical delete with no node rows is
                    // correct, and a version with no body left the primary tier
                    // or the repository between the page read and this one.
                    (false, _, None) | (true, None, _) => None,
                    (true, Some(_), None) => Some(StorageParityDefect::NodesMissing),
                    (true, Some(_), Some(Err(_))) => Some(StorageParityDefect::NodesUnreadable),
                    (true, Some(body), Some(Ok(value))) if &value != body => {
                        Some(StorageParityDefect::ContentDiffers)
                    }
                    // The rows say what the body says. Whether they say it in
                    // the shape this build writes is the second question, and
                    // the only one that catches an older decomposition.
                    (true, Some(body), Some(Ok(_))) => rows
                        .get(&id)
                        .is_some_and(|stored| !rows_are_current(stored, body))
                        .then_some(StorageParityDefect::StaleDecomposition),
                };
                (key, defect)
            })
            .collect())
    }

    /// The materialized bodies of one chunk of versions, in one statement.
    ///
    /// A version with a `NULL` body, or one that left the repository between
    /// the page read and this one, is absent from the map.
    async fn chunk_bodies(
        &self,
        domain: StorageDomain,
        keys: &[(VoId, i32)],
    ) -> Result<HashMap<(VoId, i32), Value>, ServiceError> {
        let vo_ids: Vec<Uuid> = keys.iter().map(|(vo_id, _)| vo_id.0).collect();
        let sys_versions: Vec<i32> = keys.iter().map(|(_, version)| *version).collect();
        let rows =
            integrity::stored_bodies(self.domain_pool(domain), &vo_ids, &sys_versions).await?;
        let mut out = HashMap::with_capacity(rows.len());
        for (vo_id, sys_version, text) in rows {
            let body = serde_json::from_str(&text)
                .map_err(|e| ServiceError::internal("parse the stored canonical body", e))?;
            out.insert((VoId(vo_id), sys_version), body);
        }
        Ok(out)
    }

    /// One page of stored-version identifiers after `cursor`, in
    /// `(vo_id, sys_version)` order — a keyset walk, so a long sweep never
    /// pays a growing `OFFSET`.
    async fn parity_page(
        &self,
        domain: StorageDomain,
        cursor: Option<(Uuid, i32)>,
        scope: StorageParityScope,
    ) -> Result<Vec<VersionKey>, ServiceError> {
        let rows =
            integrity::parity_page(self.domain_pool(domain), cursor, PAGE_SIZE, scope).await?;
        Ok(rows
            .into_iter()
            .map(|(vo_id, sys_version, kind, has_body)| VersionKey {
                vo_id,
                sys_version,
                kind,
                has_body,
            })
            .collect())
    }
}

/// A storage-parity sweep in progress, advanced one batch of events at a time.
///
/// Dropping the sweep stops the reads at the next batch boundary.
///
/// NOTE: no openEHR spec governs storage mechanics — our own design/extension.
pub struct StorageParitySweep<'a> {
    service: &'a FerroEhrService,
    /// The domains still to read, most recent first; the cursor pops one when
    /// its pages run out, so a single sweep walks the whole store.
    remaining: Vec<StorageDomain>,
    /// The domain currently being read.
    domain: StorageDomain,
    scope: StorageParityScope,
    cursor: Option<(Uuid, i32)>,
    page: std::vec::IntoIter<VersionKey>,
    counts: StorageParityCounts,
    started: Instant,
    page_finished: bool,
    /// Whether the current domain's unreached node rows were read, which the
    /// sweep does once its version pages run out.
    orphans_read: bool,
    done: bool,
}

impl std::fmt::Debug for StorageParitySweep<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StorageParitySweep")
            .field("domain", &self.domain)
            .field("remaining", &self.remaining)
            .field("scope", &self.scope)
            .field("cursor", &self.cursor)
            .field("counts", &self.counts)
            .field("done", &self.done)
            .finish_non_exhaustive()
    }
}

impl StorageParitySweep<'_> {
    /// Advances the sweep, returning the next batch of events, or `None` once
    /// the summary has been handed over.
    ///
    /// A batch is empty when its versions agreed; the last batch before `None`
    /// carries the [`StorageParityEvent::Summary`].
    ///
    /// # Errors
    /// - `exception` — a database fault while enumerating or reading versions.
    ///   A version whose node rows are unreadable is a REPORTED mismatch, not
    ///   an error: one damaged record must not abort the sweep that found it.
    pub async fn next_batch(&mut self) -> Result<Option<Vec<StorageParityEvent>>, ServiceError> {
        loop {
            let chunk: Vec<VersionKey> = self.page.by_ref().take(CONTENT_CHUNK).collect();
            if !chunk.is_empty() {
                let domain = self.domain;
                return Ok(Some(self.check_chunk(domain, &chunk).await?));
            }
            if self.done {
                return Ok(None);
            }
            // A whole page has been read. The tick both tells a consumer the
            // sweep is advancing and keeps a streamed response writing bytes,
            // which is what stops an intermediary closing a long clean sweep.
            if self.page_finished {
                self.page_finished = false;
                return Ok(Some(vec![StorageParityEvent::Progress {
                    counts: self.counts,
                }]));
            }
            let page = self
                .service
                .parity_page(self.domain, self.cursor, self.scope)
                .await?;
            let Some(last) = page.last() else {
                // Every version of this domain is read; what is left is the
                // node rows none of them reads.
                if !self.orphans_read {
                    self.orphans_read = true;
                    let domain = self.domain;
                    return Ok(Some(self.orphan_events(domain).await?));
                }
                // This domain is read. Move to the next before finishing: a
                // sweep that stopped at the first exhausted domain would
                // report a clean whole-store result having read half of it.
                if let Some(next) = self.remaining.pop() {
                    self.domain = next;
                    self.cursor = None;
                    self.orphans_read = false;
                    continue;
                }
                self.done = true;
                let elapsed_ms =
                    u64::try_from(self.started.elapsed().as_millis()).unwrap_or(u64::MAX);
                return Ok(Some(vec![StorageParityEvent::Summary {
                    counts: self.counts,
                    elapsed_ms,
                }]));
            };
            self.cursor = Some((last.vo_id, last.sys_version));
            self.page = page.into_iter();
            self.page_finished = true;
        }
    }

    /// The node row sets of `domain` that no stored version reads, as
    /// [`StorageParityDefect::OrphanNodes`] mismatches.
    async fn orphan_events(
        &mut self,
        domain: StorageDomain,
    ) -> Result<Vec<StorageParityEvent>, ServiceError> {
        let sets =
            integrity::unreached_node_sets(self.service.domain_pool(domain), self.scope).await?;
        let mut events = Vec::with_capacity(sets.len());
        for (vo_id, sys_version, kind) in sets {
            self.counts.mismatch_count += 1;
            tracing::warn!(
                domain = domain.as_str(),
                vo_id = %vo_id,
                sys_version,
                kind = %kind,
                defect = StorageParityDefect::OrphanNodes.as_str(),
                "storage parity sweep: node rows exist that no stored version reads"
            );
            events.push(StorageParityEvent::Mismatch(StorageParityMismatch {
                domain,
                vo_id,
                sys_version,
                kind,
                defect: StorageParityDefect::OrphanNodes,
            }));
        }
        Ok(events)
    }

    /// Compare one chunk, count what it read, and turn its defects into events.
    async fn check_chunk(
        &mut self,
        domain: StorageDomain,
        chunk: &[VersionKey],
    ) -> Result<Vec<StorageParityEvent>, ServiceError> {
        let mut events = Vec::new();
        for (key, defect) in self.service.chunk_parity_defects(domain, chunk).await? {
            self.counts.versions_checked += 1;
            if key.has_body {
                self.counts.versions_with_body += 1;
            } else {
                self.counts.versions_without_body += 1;
            }
            if let Some(defect) = defect {
                self.counts.mismatch_count += 1;
                // The log line carries identifiers and the defect token only: a
                // body fragment would put clinical content into an operational
                // log. It is emitted here, in the one sweep, so a finding is
                // logged whichever response shape asked for it.
                tracing::warn!(
                    domain = domain.as_str(),
                    vo_id = %key.vo_id,
                    sys_version = key.sys_version,
                    kind = %key.kind,
                    defect = defect.as_str(),
                    "storage parity sweep: the node rows and the materialized body of a stored version disagree"
                );
                events.push(StorageParityEvent::Mismatch(StorageParityMismatch {
                    domain,
                    vo_id: key.vo_id,
                    sys_version: key.sys_version,
                    kind: key.kind.clone(),
                    defect,
                }));
            }
        }
        Ok(events)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_tokens_are_the_documented_wire_values() {
        assert_eq!(StorageDomain::Clinical.as_str(), "clinical");
        assert_eq!(StorageDomain::Demographic.as_str(), "demographic");
    }

    #[test]
    fn defect_tokens_are_the_documented_wire_values() {
        assert_eq!(
            StorageParityDefect::ContentDiffers.as_str(),
            "content_differs"
        );
        assert_eq!(StorageParityDefect::NodesMissing.as_str(), "nodes_missing");
        assert_eq!(
            StorageParityDefect::NodesUnreadable.as_str(),
            "nodes_unreadable"
        );
        assert_eq!(
            StorageParityDefect::UnexpectedNodes.as_str(),
            "unexpected_nodes"
        );
        assert_eq!(
            StorageParityDefect::StaleDecomposition.as_str(),
            "stale_decomposition"
        );
    }

    #[test]
    fn a_report_with_no_mismatch_count_is_clean() {
        let mut report = StorageParityReport {
            versions_checked: 3,
            versions_with_body: 2,
            versions_without_body: 1,
            mismatch_count: 0,
            mismatches: Vec::new(),
            truncated: false,
            elapsed_ms: 1,
        };
        assert!(report.is_clean());
        report.mismatch_count = 1;
        assert!(!report.is_clean());
    }
}
