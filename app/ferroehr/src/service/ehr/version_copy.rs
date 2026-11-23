// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! This replica's copy of the COMPOSITION versions it read by their version
//! id, so a repeated read reads the version's body from memory (#5778).
//!
//! A version never changes once committed (RM common master06 §Version
//! Identification), so the copy holds it as the store returned it, with the
//! head facts the access evaluator judges its object by. What may change is
//! whether it may be served: a new version moves the head facts, a
//! restriction (GDPR Art. 18(2)) or an access delay (EHDS Art. 3(3)) marks
//! the object, the EHR's settings move, and an erasure (GDPR Art. 17)
//! rewrites or removes it. A held version is served only after one indexed
//! read of the store confirms, at that moment, the object's head, its marks,
//! its EHR's settings revision and the version's content digest
//! (`version_repo::judged::read_marks`), so no access decision rests on a
//! notification's timing (owner decision on #5891). Every read is judged by
//! the evaluator and logged. The bound is in bytes ([`BYTES`]). No openEHR
//! spec governs the copy: our own design.

use std::sync::Arc;

use moka::future::Cache;
use moka::notification::RemovalCause;

use crate::ids::VoId;
use crate::memory::{ARC_COUNTS, CACHE_ENTRY, Weigh, bytes, gauge};
use crate::service::ehr::access_cache::{Lookup, dropped, looked_up};
use crate::storage::access_facts::HeadFacts;
use crate::storage::version_repo::read::StoredVersion;
use crate::versioning::object_version_id::TreeId;

/// The most bytes the copy holds, by `Held::bytes`: about 500 versions of
/// the 128 kB IPS composition.
pub(crate) const BYTES: u64 = 64 * 1024 * 1024;

/// The `copy` label of this copy's lookups.
const COPY: &str = "object_version";

/// The `scope` label of a version the bound evicted.
const EVICTED: &str = "object_version_evicted";

/// One version of an object, addressed by its `VERSION_TREE_ID`.
pub(crate) type Key = (VoId, TreeId);

/// A version as a judged read returned it, with its object's head facts and
/// what a later read confirms it against.
#[derive(Debug)]
pub(crate) struct Held {
    /// The facts the evaluator judges the object by.
    pub(crate) facts: HeadFacts,
    /// The version, its body as the stored text.
    pub(crate) stored: StoredVersion,
    /// The ordinal of the object's head when it was read.
    pub(crate) head: i32,
    /// The version's content digest when it was read.
    pub(crate) digest: Vec<u8>,
}

impl Held {
    /// The bytes the entry takes in the copy: its key, the shared allocation
    /// that holds it, what its members own on the heap (the body text above
    /// all) and the cache's own bookkeeping, weighed member by member as
    /// [`crate::memory`] weighs every copy, with no allocation (#6205).
    fn bytes(&self) -> u64 {
        gauge(
            size_of::<Key>()
                .saturating_add(ARC_COUNTS)
                .saturating_add(bytes(self))
                .saturating_add(CACHE_ENTRY),
        )
    }
}

impl Weigh for Held {
    fn heap(&self) -> usize {
        self.facts
            .heap()
            .saturating_add(self.stored.heap())
            .saturating_add(self.digest.capacity())
    }
}

impl Weigh for HeadFacts {
    fn heap(&self) -> usize {
        self.template
            .heap()
            .saturating_add(self.archetype.heap())
            .saturating_add(self.origins.heap())
            .saturating_add(self.revision.heap())
    }
}

impl Weigh for StoredVersion {
    fn heap(&self) -> usize {
        let text = [
            self.kind.heap(),
            self.preceding_version_uid.heap(),
            self.other_input_version_uids.heap(),
            self.lifecycle_state.heap(),
            self.creating_system_id.heap(),
            self.audit_system_id.heap(),
            self.audit_change_type.heap(),
            self.template_id.heap(),
            self.root_archetype.heap(),
            self.signature.heap(),
            self.canonical_text
                .as_ref()
                .map_or(0, |body| ARC_COUNTS.saturating_add(body.len())),
        ];
        let values = [
            self.audit_description.heap(),
            self.audit_committer.heap(),
            self.audit_attestation.heap(),
            self.wrapped_original.heap(),
            self.origins.heap(),
            self.canonical.heap(),
            self.attestations_at_committal.heap(),
            self.attestations_after_committal.heap(),
            self.access_revision.heap(),
        ];
        text.into_iter()
            .chain(values)
            .fold(0_usize, usize::saturating_add)
    }
}

/// The read versions one replica holds.
#[derive(Debug)]
pub(crate) struct VersionCopy {
    inner: Cache<Key, Arc<Held>>,
}

impl VersionCopy {
    /// An empty copy of at most [`BYTES`].
    pub(crate) fn new() -> Self {
        Self::bounded(BYTES)
    }

    /// An empty copy of at most `bytes`.
    fn bounded(bytes: u64) -> Self {
        Self {
            inner: Cache::builder()
                .weigher(|_, held: &Arc<Held>| u32::try_from(held.bytes()).unwrap_or(u32::MAX))
                .max_capacity(bytes)
                .eviction_listener(|_, _, cause| {
                    if cause == RemovalCause::Size {
                        dropped(EVICTED);
                    }
                })
                .build(),
        }
    }

    /// The held version `key`, counted as a hit or a miss.
    pub(crate) async fn get(&self, key: Key) -> Option<Arc<Held>> {
        let held = self.inner.get(&key).await;
        let result = if held.is_some() {
            Lookup::Hit
        } else {
            Lookup::Miss
        };
        looked_up(COPY, result, 1);
        held
    }

    /// Keep `held` as `key`.
    pub(crate) async fn keep(&self, key: Key, held: Held) {
        self.inner.insert(key, Arc::new(held)).await;
    }

    /// Drop the held version `key`, which the store did not confirm.
    pub(crate) async fn forget(&self, key: Key) {
        dropped(COPY);
        self.inner.invalidate(&key).await;
    }

    /// How many versions the copy holds, and their bytes.
    pub(crate) fn held(&self) -> (u64, u64) {
        (self.inner.entry_count(), self.inner.weighted_size())
    }
}

#[cfg(test)]
#[path = "version_copy_tests.rs"]
mod tests;
