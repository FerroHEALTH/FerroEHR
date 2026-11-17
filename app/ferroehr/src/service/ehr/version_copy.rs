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
use crate::service::ehr::access_cache::{Lookup, dropped, looked_up};
use crate::storage::access_facts::HeadFacts;
use crate::storage::version_repo::read::StoredVersion;
use crate::versioning::object_version_id::TreeId;

/// The most bytes the copy holds, by `Held::bytes`: about 20 000 versions
/// of a 3 kB compact composition.
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
    /// The bytes the entry takes: the body text, the other text and JSON
    /// members at their serialized length, and the structs themselves. An
    /// estimate of our own design, dominated by the body.
    fn bytes(&self) -> u64 {
        let stored = &self.stored;
        let json = |value: &serde_json::Value| serde_json::to_string(value).map_or(0, |t| t.len());
        let text: usize = [
            Some(&stored.kind),
            stored.preceding_version_uid.as_ref(),
            Some(&stored.lifecycle_state),
            Some(&stored.creating_system_id),
            Some(&stored.audit_system_id),
            Some(&stored.audit_change_type),
            stored.template_id.as_ref(),
            stored.root_archetype.as_ref(),
            stored.signature.as_ref(),
            stored.canonical_text.as_ref(),
            self.facts.template.as_ref(),
            self.facts.archetype.as_ref(),
        ]
        .into_iter()
        .flatten()
        .map(String::len)
        .chain(stored.other_input_version_uids.iter().map(String::len))
        .chain(self.facts.origins.iter().flatten().map(String::len))
        .sum();
        let values: usize = [
            stored.audit_description.as_ref(),
            Some(&stored.audit_committer),
            stored.audit_attestation.as_ref(),
            stored.wrapped_original.as_ref(),
            stored.origins.as_ref(),
            Some(&stored.canonical),
        ]
        .into_iter()
        .flatten()
        .map(json)
        .chain(stored.attestations_at_committal.iter().map(json))
        .chain(stored.attestations_after_committal.iter().map(json))
        .sum();
        u64::try_from(size_of::<Self>() + text + values + self.digest.len()).unwrap_or(u64::MAX)
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

    /// Drop the held version `key`, which the store no longer confirms.
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
