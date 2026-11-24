// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! This replica's copy of each EHR's `EHR_ACCESS` settings and of the
//! settings classes, dropped by the notifications of their channel and by a
//! read that finds a newer revision. No openEHR spec governs the copy: our
//! own design (#5509, #5588, #5589).

use crate::db::acquire::awaiting;
use crate::db::domain::Domain;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use moka::future::Cache;
use openehr_rm::draft::access_control::terminology::ForeignCode;
use opentelemetry::KeyValue;

use crate::access_eval::class::{
    Class, Classes, DataLayout, DirectiveLayout, KeeperLayout, Layout,
};
use crate::access_eval::condition::EhrEntry;
use crate::access_eval::stale::StaleCopies;
use crate::error::ServiceError;
use crate::ids::EhrId;
use crate::memory::{self, Held, Weigh};
use crate::policy_register::listen::{Dropping, Listened, Subscriber};
use crate::service::ehr::access_settings::EhrAccessSettings;
use crate::storage::access_projection::{CHANNEL, NEW_CLASS, Revision};

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
    inner: Cache<EhrId, Arc<EhrEntry>>,
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

    /// What the copy holds: the EHRs' settings, and the classes with their
    /// bytes.
    pub(in crate::service) fn held(&self) -> (Held, Held) {
        let classes = self.classes.iter().fold(0_usize, |total, (key, classes)| {
            total
                .saturating_add(memory::bytes(key.as_ref()))
                .saturating_add(memory::bytes(&classes))
                .saturating_add(memory::CACHE_ENTRY)
        });
        (
            memory::held(&self.inner),
            Held {
                entries: self.classes_held.load(Ordering::Relaxed),
                bytes: memory::gauge(classes),
            },
        )
    }

    /// The copied entry of `ehr_id`, if this replica holds one.
    pub(in crate::service) async fn entry(&self, ehr_id: EhrId) -> Option<Arc<EhrEntry>> {
        self.inner.get(&ehr_id).await
    }

    /// How many drops this copy has seen, read before a load so
    /// [`Self::keep`] keeps nothing a drop overtook.
    pub(in crate::service) fn drops(&self) -> u64 {
        self.drops.load(Ordering::Acquire)
    }

    /// Copy `entry` for `ehr_id`, read after `seen` drops, unless a drop
    /// overtook the read.
    pub(in crate::service) async fn keep(&self, ehr_id: EhrId, entry: Arc<EhrEntry>, seen: u64) {
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
    ) -> Result<Arc<EhrEntry>, Arc<ServiceError>>
    where
        Fut: Future<Output = Result<Arc<EhrEntry>, ServiceError>>,
    {
        let seen = self.drops.load(Ordering::Acquire);
        let ran = AtomicBool::new(false);
        let init = async {
            ran.store(true, Ordering::Relaxed);
            let loaded = init.await;
            refilled(EHR, &loaded);
            loaded
        };
        let loaded = single_flight(&self.inner, ehr_id, init).await;
        looked_up(EHR, Lookup::of(&ran), 1);
        if self.drops.load(Ordering::Acquire) != seen {
            self.inner.invalidate(&ehr_id).await;
        }
        loaded
    }

    /// Seed the copy with a value for `ehr_id` without a DB read — the
    /// pre-warm path of a freshly-created EHR, which is default-open. Any
    /// later `EHR_ACCESS` commit drops this entry via [`Self::forget`].
    pub(in crate::service) async fn insert(&self, ehr_id: EhrId, entry: Arc<EhrEntry>) {
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
        let loaded = single_flight(&self.classes, (), init).await;
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

/// The value `cache` holds for `key`, or the one `init` loads, run at most
/// once under contention (`moka`'s `try_get_with`).
///
/// A hit keeps what the request keeps. A miss waits for a load, this
/// caller's or another's, which needs a connection, so the request gives its
/// own back first ([`awaiting`], #6027).
async fn single_flight<K, V>(
    cache: &Cache<K, V>,
    key: K,
    init: impl Future<Output = Result<V, ServiceError>>,
) -> Result<V, Arc<ServiceError>>
where
    K: std::hash::Hash + Eq + Send + Sync + 'static,
    V: Clone + Send + Sync + 'static,
{
    if let Some(held) = cache.get(&key).await {
        return Ok(held);
    }
    awaiting(Box::pin(cache.try_get_with(key, init))).await
}

/// The listener on [`CHANNEL`], dropping from `cache`.
pub(in crate::service) fn listened(cache: &Arc<EhrAccessCache>) -> Listened {
    let cache: Arc<EhrAccessCache> = Arc::clone(cache);
    Listened::with_subscriber(CHANNEL, Domain::Clinical, cache)
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

impl Weigh for EhrId {
    fn heap(&self) -> usize {
        0
    }
}

impl Weigh for EhrEntry {
    fn heap(&self) -> usize {
        self.settings
            .heap()
            .saturating_add(self.subject.heap())
            .saturating_add(self.version.heap())
            .saturating_add(self.revision.heap())
    }
}

impl Weigh for Revision {
    fn heap(&self) -> usize {
        self.subject.heap()
    }
}

impl Weigh for EhrAccessSettings {
    fn heap(&self) -> usize {
        match self {
            // NOTE: our own design, the generated settings weigh their JSON text, as the version copy weighs its members.
            Self::Directives(settings) => {
                size_of_val(settings.as_ref()).saturating_add(memory::json_bytes(settings.as_ref()))
            }
            Self::HeldClosed => 0,
        }
    }
}

impl Weigh for Classes {
    fn heap(&self) -> usize {
        self.classes.heap()
    }
}

impl Weigh for Class {
    fn heap(&self) -> usize {
        self.settings.heap().saturating_add(self.layout.heap())
    }
}

impl Weigh for Layout {
    fn heap(&self) -> usize {
        let labels = self.labels.iter().fold(
            self.labels.capacity() * size_of::<(ForeignCode, usize)>(),
            |total, (code, _)| {
                total
                    .saturating_add(code.terminology.capacity())
                    .saturating_add(code.code.capacity())
            },
        );
        self.directives
            .heap()
            .saturating_add(self.keepers.capacity() * size_of::<KeeperLayout>())
            .saturating_add(labels)
    }
}

impl Weigh for DirectiveLayout {
    fn heap(&self) -> usize {
        (self.parties.capacity() * size_of::<Option<usize>>())
            .saturating_add(self.data.capacity() * size_of::<DataLayout>())
            .saturating_add(self.exceptions.heap())
    }
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

    /// The entry the store holds for the `n`th EHR of a test.
    fn stored(n: u64) -> EhrEntry {
        let subject = n.is_multiple_of(2).then(|| {
            crate::storage::access_projection::EhrSubject::new("demographic", n.to_string())
        });
        EhrEntry {
            settings: Arc::new(n.is_multiple_of(5).then_some(EhrAccessSettings::HeldClosed)),
            subject: subject.clone(),
            version: n.is_multiple_of(5).then(|| format!("{n}::ferroehr::1")),
            revision: Revision {
                settings: i32::try_from(n).ok(),
                subject,
            },
        }
    }

    /// The copy holds at most its capacity of EHRs, and an EHR the cap
    /// evicted is loaded again with the store's entry, so the cap changes
    /// no answer (#5926).
    #[tokio::test]
    async fn the_ehrs_stay_under_the_cap_and_an_evicted_one_loads_again() {
        let cap = 8;
        let cache = EhrAccessCache::new(cap);
        let ehrs: Vec<EhrId> = (0..64).map(|_| EhrId(uuid::Uuid::now_v7())).collect();
        let loads = std::sync::atomic::AtomicUsize::new(0);
        for round in 0..2 {
            for (n, ehr_id) in (0_u64..).zip(&ehrs) {
                let load = async {
                    loads.fetch_add(1, Ordering::Relaxed);
                    Ok(Arc::new(stored(n)))
                };
                let entry = cache
                    .get_or_load(*ehr_id, load)
                    .await
                    .expect("the load succeeds");
                let expected = stored(n);
                assert_eq!(entry.subject, expected.subject, "round {round}, EHR {n}");
                assert_eq!(entry.version, expected.version, "round {round}, EHR {n}");
                assert_eq!(entry.revision, expected.revision, "round {round}, EHR {n}");
                assert_eq!(
                    entry.settings.is_some(),
                    expected.settings.is_some(),
                    "round {round}, EHR {n}"
                );
            }
            cache.inner.run_pending_tasks().await;
            let (held, _) = cache.held();
            assert!(held.entries <= cap, "{held:?} within {cap}");
            assert!(held.bytes > 0, "{held:?} weighs its entries");
        }
        assert!(
            loads.load(Ordering::Relaxed) > 64,
            "the evicted EHRs were loaded again"
        );
    }

    /// The class copy holds one set of classes, and classes dropped are
    /// loaded again as the store holds them.
    #[tokio::test]
    async fn the_classes_stay_under_their_cap_and_load_again_when_dropped() {
        let cache = EhrAccessCache::new(16);
        for round in 0..4_i64 {
            let load = async move {
                Ok(Classes {
                    classes: vec![(
                        round,
                        Class {
                            settings: Arc::new(None),
                            layout: Arc::new(Layout::default()),
                        },
                    )],
                })
            };
            let classes = cache
                .classes_or_load(load)
                .await
                .expect("the load succeeds");
            assert_eq!(classes.classes.first().map(|(id, _)| *id), Some(round));
            cache.classes.run_pending_tasks().await;
            let (_, held) = cache.held();
            assert_eq!(held.entries, 1, "round {round}: {held:?}");
            assert!(held.bytes > 0, "{held:?}");
            cache.forget_classes();
        }
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
