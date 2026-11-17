// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! This replica's copy of the COMPOSITION versions it read by their version
//! id, so a repeated read makes no PostgreSQL statement (#5778).
//!
//! A version never changes once committed (RM common master06 §Version
//! Identification), so the copy holds it as the store returned it, with the
//! head facts the access evaluator judges its object by. What may change is
//! whether it may be served: a new version moves the head facts, a
//! restriction (GDPR Art. 18(2)) or an access delay (EHDS Art. 3(3)) marks
//! the object, and an erasure (GDPR Art. 17) rewrites or removes it. Each of
//! these notifies [`CHANNEL`] with the objects it changed
//! (`clinical/0051_object_version_notify.sql`), and every replica drops them;
//! this replica also drops what its own restriction and erasure commit. A
//! listener that connects or loses its connection drops the whole copy, and
//! while it is down the copy is neither read nor filled. A read that a drop
//! overtook keeps nothing. Every read served from the copy is still judged
//! by the evaluator and logged; the copy changes only where the bytes come
//! from. The bound is in bytes ([`BYTES`]). No openEHR spec governs the
//! copy: our own design.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use moka::future::Cache;
use moka::notification::RemovalCause;

use crate::ids::{EhrId, VoId};
use crate::policy_register::listen::{Dropping, Subscriber};
use crate::service::ehr::access_cache::{Lookup, dropped, looked_up};
use crate::storage::access_facts::HeadFacts;
use crate::storage::version_repo::read::StoredVersion;
use crate::versioning::object_version_id::TreeId;

/// The channel a change to a stored object notifies.
pub(crate) const CHANNEL: &str = "ferroehr_object_version";

/// The most bytes the copy holds, by `Held::bytes`: about 20 000 versions
/// of a 3 kB compact composition.
pub(crate) const BYTES: u64 = 64 * 1024 * 1024;

/// The `copy` label of this copy's lookups.
const COPY: &str = "object_version";

/// The `scope` label of a version the bound evicted.
const EVICTED: &str = "object_version_evicted";

/// One version of an object, addressed by its `VERSION_TREE_ID`.
pub(crate) type Key = (VoId, TreeId);

/// A version as a judged read returned it, with its object's head facts.
#[derive(Debug)]
pub(crate) struct Held {
    /// The facts the evaluator judges the object by.
    pub(crate) facts: HeadFacts,
    /// The version, its body as the stored text.
    pub(crate) stored: StoredVersion,
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
        u64::try_from(size_of::<Self>() + text + values).unwrap_or(u64::MAX)
    }
}

/// The read versions one replica holds.
#[derive(Debug)]
pub(crate) struct VersionCopy {
    inner: Cache<Key, Arc<Held>>,
    /// Moved by every drop, so a read that started before one keeps nothing.
    drops: AtomicU64,
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
            drops: AtomicU64::new(0),
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

    /// How many drops this copy has seen, read before a store read so
    /// [`Self::keep`] keeps nothing a drop overtook.
    pub(crate) fn drops(&self) -> u64 {
        self.drops.load(Ordering::Acquire)
    }

    /// Keep `held` as `key`, read after `seen` drops, unless a drop overtook
    /// the read.
    pub(crate) async fn keep(&self, key: Key, held: Held, seen: u64) {
        self.inner.insert(key, Arc::new(held)).await;
        if self.drops() != seen {
            self.inner.invalidate(&key).await;
        }
    }

    /// Drop every held version of `vo_id`.
    pub(crate) async fn forget_object(&self, vo_id: VoId) {
        self.forget_where(|held| held.facts.vo_id == vo_id).await;
    }

    /// Drop every held version of an object of `ehr_id`.
    pub(crate) async fn forget_ehr(&self, ehr_id: EhrId) {
        self.forget_where(|held| held.facts.ehr_id == Some(ehr_id))
            .await;
    }

    /// Drop the held versions `drop` picks.
    async fn forget_where(&self, drop: impl Fn(&Held) -> bool) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped(COPY);
        let keys: Vec<Key> = self
            .inner
            .iter()
            .filter(|(_, held)| drop(held))
            .map(|(key, _)| *key)
            .collect();
        for key in keys {
            self.inner.invalidate(&key).await;
        }
    }

    /// Drop every held version.
    pub(crate) fn forget_every(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped(COPY);
        self.inner.invalidate_all();
    }

    /// How many versions the copy holds, and their bytes.
    pub(crate) fn held(&self) -> (u64, u64) {
        (self.inner.entry_count(), self.inner.weighted_size())
    }
}

impl Subscriber for VersionCopy {
    fn notified<'a>(&'a self, payload: &'a str) -> Dropping<'a> {
        // NOTE: our own design, a payload that names no object, the trigger's `*` for a
        // statement that changed many, drops the whole copy.
        let object = payload.parse::<uuid::Uuid>().ok().map(VoId);
        Box::pin(self.forget(object))
    }

    fn reset(&self) -> Dropping<'_> {
        Box::pin(self.forget(None))
    }
}

impl VersionCopy {
    /// Drop the held versions of `object`, or every held version.
    async fn forget(&self, object: Option<VoId>) {
        match object {
            Some(vo_id) => self.forget_object(vo_id).await,
            None => self.forget_every(),
        }
    }
}

#[cfg(test)]
#[path = "version_copy_tests.rs"]
mod tests;
