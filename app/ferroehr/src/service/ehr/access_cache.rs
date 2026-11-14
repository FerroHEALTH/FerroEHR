// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! This replica's copy of each EHR's `EHR_ACCESS` settings and of the
//! settings classes, dropped by the notifications of their channel and by a
//! read that finds a newer revision. No openEHR spec governs the copy: our
//! own design (#5509, #5588, #5589).

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use moka::future::Cache;
use opentelemetry::KeyValue;

use crate::access_eval::class::Classes;
use crate::access_eval::condition::{EhrEntry, StaleCopies};
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::policy_register::listen::{Dropping, Listened, Subscriber};
use crate::storage::access_projection::{CHANNEL, NEW_CLASS};

/// A shared per-EHR copy of the current `EHR_ACCESS` scheme settings.
///
/// The `EHR_ACCESS` gateway clause ("All access decisions to data in the EHR
/// must be made in accordance with the policies and rules in this object" —
/// RM ehr `ehr_access.adoc`) is consulted on **every** EHR-scoped request, so
/// the current settings are copied rather than read per request. An entry is
/// dropped by this replica's own `EHR_ACCESS` commit and by every notification
/// on [`CHANNEL`] that names its EHR; a notification of another payload, and a
/// listener that connects or loses its connection, drop every entry. A load
/// that a drop overtakes keeps nothing, so a load racing a commit never leaves
/// the settings before it in force.
///
/// No openEHR spec governs this copy — our own design/extension.
#[derive(Debug)]
pub(crate) struct EhrAccessCache {
    inner: Cache<EhrId, EhrEntry>,
    classes: Cache<(), Arc<Classes>>,
    /// Moved by every drop, so a load that started before one keeps nothing.
    drops: AtomicU64,
    /// The EHRs a read found newer than their entry.
    stale: Arc<StaleCopies>,
    /// How many classes the copied entry holds, for the metrics.
    classes_held: AtomicU64,
}

impl EhrAccessCache {
    /// A copy holding up to `capacity` EHRs' settings.
    fn new(capacity: u64) -> Self {
        Self {
            inner: Cache::builder().max_capacity(capacity).build(),
            classes: Cache::builder().max_capacity(1).build(),
            drops: AtomicU64::new(0),
            stale: Arc::default(),
            classes_held: AtomicU64::new(0),
        }
    }

    /// The EHRs a read found newer than their entry.
    pub(in crate::service) fn stale(&self) -> Arc<StaleCopies> {
        Arc::clone(&self.stale)
    }

    /// Drop the entry of every EHR a read found newer than it.
    pub(in crate::service) async fn forget_stale(&self) {
        for ehr_id in self.stale.take() {
            self.forget(ehr_id).await;
        }
    }

    /// How many EHRs' settings the copy holds, and how many classes.
    pub(in crate::service) fn held_counts(&self) -> (u64, u64) {
        (
            self.inner.entry_count(),
            self.classes_held.load(Ordering::Relaxed),
        )
    }

    /// The copied entry of `ehr_id`, if this replica holds one.
    pub(in crate::service) async fn held(&self, ehr_id: EhrId) -> Option<EhrEntry> {
        self.inner.get(&ehr_id).await
    }

    /// How many drops this copy has seen, read before a load so
    /// [`Self::keep`] keeps nothing a drop overtook.
    pub(in crate::service) fn drops(&self) -> u64 {
        self.drops.load(Ordering::Acquire)
    }

    /// Copy `entry` for `ehr_id`, read after `seen` drops, unless a drop
    /// overtook the read.
    pub(in crate::service) async fn keep(&self, ehr_id: EhrId, entry: EhrEntry, seen: u64) {
        self.inner.insert(ehr_id, entry).await;
        if self.drops() != seen {
            self.inner.invalidate(&ehr_id).await;
        }
    }

    /// The copied settings for `ehr_id`, or load them via `init` (run at most
    /// once per key under contention) and copy the result unless a drop
    /// overtook the load.
    ///
    /// # Errors
    /// Propagates the `init` error (shared across concurrent callers as an
    /// `Arc<ServiceError>`).
    pub(in crate::service) async fn get_or_load<Fut>(
        &self,
        ehr_id: EhrId,
        init: Fut,
    ) -> Result<EhrEntry, Arc<ServiceError>>
    where
        Fut: Future<Output = Result<EhrEntry, ServiceError>>,
    {
        let seen = self.drops.load(Ordering::Acquire);
        let ran = AtomicBool::new(false);
        let init = async {
            ran.store(true, Ordering::Relaxed);
            let loaded = init.await;
            refilled(EHR, &loaded);
            loaded
        };
        let loaded = self.inner.try_get_with(ehr_id, init).await;
        looked_up(EHR, Lookup::of(&ran), 1);
        if self.drops.load(Ordering::Acquire) != seen {
            self.inner.invalidate(&ehr_id).await;
        }
        loaded
    }

    /// Seed the copy with a value for `ehr_id` without a DB read — the
    /// pre-warm path of a freshly-created EHR, which is default-open. Any
    /// later `EHR_ACCESS` commit drops this entry via [`Self::forget`].
    pub(in crate::service) async fn insert(&self, ehr_id: EhrId, entry: EhrEntry) {
        self.inner.insert(ehr_id, entry).await;
    }

    /// Drop the copied settings for `ehr_id`.
    pub(in crate::service) async fn forget(&self, ehr_id: EhrId) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped(EHR);
        self.inner.invalidate(&ehr_id).await;
    }

    /// Drop the copied classes.
    pub(in crate::service) fn forget_classes(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped(CLASSES);
        self.classes.invalidate_all();
        self.classes_held.store(0, Ordering::Relaxed);
    }

    /// Drop every copied entry and the classes.
    pub(in crate::service) fn forget_every(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped("every");
        self.inner.invalidate_all();
        self.classes.invalidate_all();
        self.classes_held.store(0, Ordering::Relaxed);
    }

    /// The copied classes, or load them via `init` (run at most once under
    /// contention) and copy the result unless a drop overtook the load.
    ///
    /// # Errors
    /// Propagates the `init` error (shared across concurrent callers as an
    /// `Arc<ServiceError>`).
    pub(in crate::service) async fn classes_or_load<Fut>(
        &self,
        init: Fut,
    ) -> Result<Arc<Classes>, Arc<ServiceError>>
    where
        Fut: Future<Output = Result<Classes, ServiceError>>,
    {
        let seen = self.drops();
        let ran = AtomicBool::new(false);
        let init = async {
            ran.store(true, Ordering::Relaxed);
            let loaded = init.await;
            refilled(CLASSES, &loaded);
            loaded.map(Arc::new)
        };
        let loaded = self.classes.try_get_with((), init).await;
        looked_up(CLASSES, Lookup::of(&ran), 1);
        if let Ok(classes) = &loaded {
            let held = u64::try_from(classes.classes.len()).unwrap_or(u64::MAX);
            self.classes_held.store(held, Ordering::Relaxed);
        }
        if self.drops() != seen {
            self.classes.invalidate(&()).await;
            self.classes_held.store(0, Ordering::Relaxed);
        }
        loaded
    }
}

impl Subscriber for EhrAccessCache {
    fn notified<'a>(&'a self, payload: &'a str) -> Dropping<'a> {
        Box::pin(async move {
            // NOTE: our own design, a payload that names no EHR and no new class, the
            // refill's, drops every EHR and every class.
            match payload.parse::<uuid::Uuid>() {
                Ok(ehr_id) => self.forget(EhrId(ehr_id)).await,
                Err(_) if payload == NEW_CLASS => self.forget_classes(),
                Err(_) => self.forget_every(),
            }
        })
    }

    fn reset(&self) -> Dropping<'_> {
        self.forget_every();
        Box::pin(std::future::ready(()))
    }
}

/// The listener on [`CHANNEL`], dropping from `cache`.
pub(in crate::service) fn listened(cache: &Arc<EhrAccessCache>) -> Listened {
    let cache: Arc<EhrAccessCache> = Arc::clone(cache);
    Listened::with_subscriber(CHANNEL, cache)
}

/// The outcome of a single-flight load, with an error of the caller's own.
///
/// The caller that ran a failed load holds its error alone and gets it as it
/// was raised. A concurrent waiter only shares it, and a `ServiceError` cannot
/// be cloned, so the waiter reads again and answers with its own error of the
/// same class and body. No openEHR spec governs the cache: our own design.
pub(in crate::service) async fn owned_or_reload<T, Fut>(
    loaded: Result<T, Arc<ServiceError>>,
    reload: impl FnOnce() -> Fut,
) -> Result<T, ServiceError>
where
    Fut: Future<Output = Result<T, ServiceError>>,
{
    match loaded {
        Ok(value) => Ok(value),
        Err(shared) => match Arc::try_unwrap(shared) {
            Ok(error) => Err(error),
            Err(_) => reload().await,
        },
    }
}

/// The `copy` label of the per-EHR copy.
pub(in crate::service) const EHR: &str = "ehr";
/// The `copy` label of the class copy.
pub(in crate::service) const CLASSES: &str = "classes";

/// Whether a lookup found its entry in the copy.
#[derive(Debug, Clone, Copy)]
pub(in crate::service) enum Lookup {
    Hit,
    Miss,
}

impl Lookup {
    /// A miss when this caller ran the load, `ran`; a caller that waited on
    /// another's load is served from the copy.
    fn of(ran: &AtomicBool) -> Self {
        if ran.load(Ordering::Relaxed) {
            Self::Miss
        } else {
            Self::Hit
        }
    }
}

/// Count `lookups` lookups in `copy` as `result`.
pub(in crate::service) fn looked_up(copy: &'static str, result: Lookup, lookups: usize) {
    let result = match result {
        Lookup::Hit => "hit",
        Lookup::Miss => "miss",
    };
    crate::telemetry::access_path::metrics().copy_lookups.add(
        u64::try_from(lookups).unwrap_or(u64::MAX),
        &[KeyValue::new("copy", copy), KeyValue::new("result", result)],
    );
}

/// Count one read that refills `copy`, by its outcome.
pub(in crate::service) fn refilled<T, E>(copy: &'static str, read: &Result<T, E>) {
    crate::telemetry::access_path::metrics().copy_refills.add(
        1,
        &[
            KeyValue::new("copy", copy),
            crate::telemetry::access_path::outcome(read),
        ],
    );
}

/// Count one drop of `scope`.
pub(in crate::service) fn dropped(scope: &'static str) {
    crate::telemetry::access_path::metrics()
        .copy_drops
        .add(1, &[KeyValue::new("scope", scope)]);
}

impl Default for EhrAccessCache {
    fn default() -> Self {
        Self::new(4096)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::refusal::{Missing, Refusal};

    /// The refusal a missing EHR's settings load answers.
    fn missing() -> ServiceError {
        ServiceError::not_found(Missing::Ehr, "no EHR with this id")
    }

    /// Two concurrent loads of one EHR's settings that fail answer the same
    /// refusal: the caller that ran the load keeps its error, and the waiter
    /// that only shared it reads again for its own.
    #[tokio::test]
    async fn concurrent_loads_of_a_missing_ehr_answer_the_same_refusal() {
        let cache = EhrAccessCache::new(16);
        let ehr_id = EhrId(uuid::Uuid::now_v7());
        let load = || async {
            tokio::time::sleep(std::time::Duration::from_millis(50)).await;
            Err(missing())
        };
        let (first, second) = tokio::join!(
            cache.get_or_load(ehr_id, load()),
            cache.get_or_load(ehr_id, load())
        );
        let (first, second) = tokio::join!(
            owned_or_reload(first, || async { Err(missing()) }),
            owned_or_reload(second, || async { Err(missing()) }),
        );
        for answer in [first, second] {
            assert!(
                matches!(
                    &answer,
                    Err(ServiceError::NotFound(Refusal {
                        kind: Missing::Ehr,
                        message,
                        ..
                    })) if message == "no EHR with this id"
                ),
                "every concurrent caller answers the loader's refusal: {answer:?}"
            );
        }
    }
}
