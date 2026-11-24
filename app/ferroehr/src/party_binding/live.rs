// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`LiveBindings`]: this replica's copy of what resolves a login to a
//! `PERSON`.
//!
//! It holds the lookups of the binding side table and the names of the bound
//! `PERSON`s, so a request whose caller is held reads neither (#5771).
//! An entry is read on a miss and kept until a notification on [`CHANNEL`]
//! drops it (`migrations/party/0006_baseline_binding.sql`). A transaction that
//! changes the side table notifies, at commit, the binding revision it moved
//! to and its change set: the parties, the keys and the group issuers of the
//! rows it wrote. Each replica drops the lookups whose key, group issuer or
//! read parties the change set names, and holds the others at that revision
//! ([`BindingCopy::apply`]); a change set it cannot apply in revision order
//! drops every lookup. A write of a `PERSON`'s version drops its name. A
//! listener that connects or loses its connection drops everything, and while
//! it is down every request reads the store. A request whose own commit changed
//! the side table drops every name before it answers ([`tracking`]); its
//! lookups are read again on its replica's next request, which reads the moved
//! revision. A lookup is held for the date it was read on, since the
//! professional and representation states it carries are judged on that date.
//! A read that a drop overtook keeps nothing. No openEHR spec governs resolving
//! a login to a party: our own design.

use crate::db::domain::Domain;
use futures::FutureExt as _;
use std::future::Future;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, Ordering};

use ferrohealth_smart::party::source::{Lookup, PartyRecord};
use ferrohealth_smart::party::{Membership, PartyRef, Represented};
use moka::future::Cache;
use opentelemetry::KeyValue;
use uuid::Uuid;

use crate::memory::{self, Held, Weigh};
use crate::policy_register::listen::{Dropping, Listened, Subscriber};

/// The channel a change of the binding side table or of a `PERSON`'s version
/// notifies, and every replica listens on.
pub const CHANNEL: &str = "ferroehr_party_binding";

/// How many logins' lookups, and how many names, a replica holds.
pub const CAPACITY: u64 = 65_536;

/// How many stripes the lookups' store reads are spread over.
const STRIPES: usize = 64;

/// The `copy` label of the lookups.
const LOOKUPS: &str = "binding";
/// The `copy` label of the names.
const NAMES: &str = "party_name";
/// The drop `scope` of a lookup a change set reached.
const REACHED: &str = "binding_changed";

tokio::task_local! {
    /// Set when a commit of the running request changed the side table.
    static CHANGED: AtomicBool;
}

/// Records that the running request's commit changed the binding side table,
/// so the request drops this replica's copy before it answers.
pub(crate) fn note_change() {
    // NOTE: tokio LocalKey::try_with, outside a tracked request (a refill, a
    // test) there is no request to answer, and the notification covers it.
    match CHANGED.try_with(|changed| changed.store(true, Ordering::Release)) {
        Ok(()) | Err(_) => {}
    }
}

/// Runs `fut`, returning its output and whether a commit inside it changed
/// the binding side table.
pub fn tracking<F: Future>(fut: F) -> impl Future<Output = (F::Output, bool)> {
    CHANGED.scope(
        AtomicBool::new(false),
        fut.map(|output| {
            let changed = CHANGED.with(|changed| changed.load(Ordering::Acquire));
            (output, changed)
        }),
    )
}

/// What a lookup is held under: the login's key, the group issuer its
/// memberships are read under, and the date its states are judged on.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct LookupKey {
    pub(crate) issuer: String,
    pub(crate) identifier_type: String,
    pub(crate) value: String,
    pub(crate) group_issuer: Option<String>,
    pub(crate) today: String,
}

impl Weigh for LookupKey {
    fn heap(&self) -> usize {
        self.issuer
            .heap()
            .saturating_add(self.identifier_type.heap())
            .saturating_add(self.value.heap())
            .saturating_add(self.group_issuer.heap())
            .saturating_add(self.today.heap())
    }
}

impl Weigh for Lookup {
    fn heap(&self) -> usize {
        match self {
            Self::Parties(records) => records.iter().fold(
                records.capacity() * size_of::<PartyRecord>(),
                |total, record| total.saturating_add(record_heap(record)),
            ),
            // NOTE: our own design, a tombstone and any later outcome own nothing beside the enum.
            _ => 0,
        }
    }
}

/// The bytes a party record owns on the heap, read through its accessors,
/// so each text counts its length.
fn record_heap(record: &PartyRecord) -> usize {
    let memberships = record
        .memberships()
        .iter()
        .map(|membership: &Membership| {
            size_of::<Membership>()
                .saturating_add(reference_heap(membership.group()))
                .saturating_add(membership.identifier().len())
        })
        .fold(0_usize, usize::saturating_add);
    let represents = record
        .represents()
        .iter()
        .map(|represented: &Represented| {
            size_of::<Represented>()
                .saturating_add(reference_heap(represented.subject()))
                .saturating_add(represented.relationship().map_or(0, str::len))
        })
        .fold(0_usize, usize::saturating_add);
    reference_heap(record.party())
        .saturating_add(memberships)
        .saturating_add(represents)
}

/// The bytes a party reference owns on the heap: its three texts.
fn reference_heap(reference: &PartyRef) -> usize {
    reference
        .namespace()
        .len()
        .saturating_add(reference.type_name().len())
        .saturating_add(reference.id().len())
}

/// A lookup as the store answered it: the lookup, the binding revision it
/// was read at, and the parties of the rows it read.
#[derive(Debug)]
pub(crate) struct Filled {
    /// The lookup.
    pub(crate) lookup: Lookup,
    /// The binding revision the first statement read.
    pub(crate) revision: i64,
    /// Every key row's party, membership group and represented subject the
    /// read met, so a change of any of their rows drops the lookup.
    pub(crate) parties: Vec<Uuid>,
}

/// A held lookup with the revision it was read at and the parties it rests
/// on, sorted.
#[derive(Debug)]
struct Kept {
    lookup: Lookup,
    at: i64,
    parties: Box<[Uuid]>,
}

impl Weigh for Kept {
    fn heap(&self) -> usize {
        self.lookup
            .heap()
            .saturating_add(self.parties.len().saturating_mul(size_of::<Uuid>()))
    }
}

/// The change set a transaction that moved the binding revision notifies.
#[derive(Debug, serde::Deserialize)]
struct Changed {
    /// The binding revision the transaction moved to.
    r: i64,
    /// What it changed.
    c: Vec<Change>,
}

/// One change of a [`Changed`] set.
#[derive(Debug, serde::Deserialize)]
#[serde(untagged)]
enum Change {
    /// The rows of a party changed.
    Party { p: Uuid },
    /// The key rows of a login key changed: its issuer, type and value.
    Key { k: (String, String, String) },
    /// A group identifier under an issuer changed.
    Groups { g: String },
    /// A change no row names, `"*"`, or one this copy does not read.
    Every(String),
}

impl Change {
    /// Whether the change can alter what `kept`, held under `key`, answers.
    fn reaches(&self, key: &LookupKey, kept: &Kept) -> bool {
        match self {
            Self::Party { p } => kept.parties.binary_search(p).is_ok(),
            Self::Key {
                k: (issuer, identifier_type, value),
            } => {
                key.issuer == *issuer
                    && key.identifier_type == *identifier_type
                    && key.value == *value
            }
            Self::Groups { g } => key.group_issuer.as_deref() == Some(g.as_str()),
            Self::Every(_) => true,
        }
    }
}

/// The lookups and names this replica holds, dropped by the notifications of
/// [`CHANNEL`].
#[derive(Debug)]
struct BindingCopy {
    /// Each lookup with the binding revision of the store it was read at.
    lookups: Cache<LookupKey, Arc<Kept>>,
    names: Cache<Uuid, Option<String>>,
    /// The binding revision whose change set, and every one before it back
    /// to the last drop of every lookup, this copy applied: a held lookup
    /// answers for every revision from its own to this one.
    confirmed: AtomicI64,
    /// Moved by every drop, so a read that started before one keeps nothing.
    drops: AtomicU64,
    /// A lookup's store read runs under the stripe its key hashes to.
    reading: Box<[tokio::sync::Mutex<()>]>,
}

impl BindingCopy {
    fn new(capacity: u64) -> Self {
        Self::striped(capacity, STRIPES)
    }

    fn striped(capacity: u64, stripes: usize) -> Self {
        Self {
            lookups: Cache::builder().max_capacity(capacity).build(),
            names: Cache::builder().max_capacity(capacity).build(),
            confirmed: AtomicI64::new(i64::MIN),
            drops: AtomicU64::new(0),
            reading: (0..stripes.max(1))
                .map(|_| tokio::sync::Mutex::new(()))
                .collect(),
        }
    }

    /// The stripe `key`'s store read runs under.
    #[expect(
        clippy::indexing_slicing,
        reason = "the index is reduced modulo the stripe count, at least 1, so it lies in bounds"
    )]
    fn stripe(&self, key: &LookupKey) -> &tokio::sync::Mutex<()> {
        let mut hasher = DefaultHasher::new();
        key.hash(&mut hasher);
        let [low, ..] = hasher.finish().to_le_bytes();
        &self.reading[usize::from(low) % self.reading.len()]
    }

    /// Applies the change set `changed`: drops every lookup it reaches and
    /// confirms the others at its revision, or drops every lookup when it
    /// does not follow the confirmed revision or names a change no row names.
    async fn apply(&self, changed: Changed) {
        let follows = changed.r.checked_sub(1) == Some(self.confirmed.load(Ordering::Acquire));
        let every = changed.c.iter().find_map(|change| match change {
            Change::Every(text) => Some(text.as_str()),
            Change::Party { .. } | Change::Key { .. } | Change::Groups { .. } => None,
        });
        if let Some(change) = every {
            tracing::debug!(change, "a binding change no row names drops every lookup");
        }
        self.drops.fetch_add(1, Ordering::AcqRel);
        if !follows || every.is_some() {
            dropped(LOOKUPS);
            self.lookups.invalidate_all();
        } else {
            let reached: Vec<Arc<LookupKey>> = self
                .lookups
                .iter()
                .filter(|(key, kept)| changed.c.iter().any(|change| change.reaches(key, kept)))
                .map(|(key, _)| key)
                .collect();
            for key in reached {
                dropped(REACHED);
                self.lookups.invalidate(key.as_ref()).await;
            }
        }
        self.confirmed.store(changed.r, Ordering::Release);
    }

    /// Drop the name of `party`.
    async fn forget_name(&self, party: Uuid) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped(NAMES);
        self.names.invalidate(&party).await;
    }

    /// Drop every name.
    fn forget_names(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped(NAMES);
        self.names.invalidate_all();
    }

    /// Drop every lookup and every name, and confirm no revision.
    fn forget_all(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped("every");
        self.confirmed.store(i64::MIN, Ordering::Release);
        self.lookups.invalidate_all();
        self.names.invalidate_all();
    }
}

impl Subscriber for BindingCopy {
    fn notified<'a>(&'a self, payload: &'a str) -> Dropping<'a> {
        Box::pin(async move {
            // NOTE: our own design (#6403), the side table's commit notifies its change set and
            // a PERSON's version its container; any other payload drops all.
            if let Ok(party) = payload.parse::<Uuid>() {
                self.forget_name(party).await;
            } else {
                match serde_json::from_str::<Changed>(payload) {
                    Ok(changed) => self.apply(changed).await,
                    Err(_) => self.forget_all(),
                }
            }
        })
    }

    fn reset(&self) -> Dropping<'_> {
        self.forget_all();
        Box::pin(std::future::ready(()))
    }
}

/// This replica's copy of the binding lookups and the bound `PERSON`s'
/// names, with the listener that keeps it fresh.
#[derive(Debug)]
pub struct LiveBindings {
    copy: Arc<BindingCopy>,
    listened: Arc<Listened>,
}

impl Default for LiveBindings {
    fn default() -> Self {
        Self::new()
    }
}

impl LiveBindings {
    /// An empty copy whose listener is not yet connected, so every request
    /// reads the store until [`crate::policy_register::listen::Listeners::spawn`] connects it.
    #[must_use]
    pub fn new() -> Self {
        let copy = Arc::new(BindingCopy::new(CAPACITY));
        let dropping: Arc<dyn Subscriber> = Arc::<BindingCopy>::clone(&copy);
        Self {
            copy,
            listened: Arc::new(Listened::with_subscriber(CHANNEL, Domain::Party, dropping)),
        }
    }

    /// The listener on [`CHANNEL`].
    #[must_use]
    pub fn listened(&self) -> &Arc<Listened> {
        &self.listened
    }

    /// What the copy holds: the lookups, and the names, with their bytes.
    #[must_use]
    pub fn held(&self) -> (Held, Held) {
        (
            memory::held(&self.copy.lookups),
            memory::held(&self.copy.names),
        )
    }

    /// Drop every lookup and name this replica holds.
    pub fn forget_all(&self) {
        self.copy.forget_all();
    }

    /// The binding revision whose change set this replica applied last;
    /// `None` until it applies one in order after a drop of every lookup.
    #[must_use]
    pub fn confirmed(&self) -> Option<i64> {
        Some(self.copy.confirmed.load(Ordering::Acquire)).filter(|at| *at != i64::MIN)
    }

    /// Drop every name this replica holds.
    pub fn forget_names(&self) {
        self.copy.forget_names();
    }

    /// Whether the listener is down, so a lookup reads the store and holds
    /// nothing; such a read is counted.
    pub(crate) fn unlistened(&self) -> bool {
        self.listened.unlistened()
    }

    /// The lookup held for `key` when it answers for the store's binding
    /// revision `revision`, which the request read: it was read at that
    /// revision or a later one, or this copy confirmed the change sets up to
    /// it and none reached the lookup; else `read` it and hold it unless a
    /// drop overtook the read. Any other held lookup is older than a change of
    /// the side table whose notification has not arrived, and is read again
    /// (#6048, #6403). With no `revision`, a ride's, any held lookup is
    /// taken and the revision it was read at noted for the ride (#6320).
    /// Concurrent misses of one key wait for one read under the key's stripe,
    /// and each reads on its own only when that read failed (#6374).
    ///
    /// # Errors
    /// The error of `read`, which is never held.
    pub(crate) async fn lookup<E>(
        &self,
        key: LookupKey,
        revision: Option<i64>,
        read: impl Future<Output = Result<Filled, E>>,
    ) -> Result<Lookup, E> {
        // NOTE: our own design (#6403), the confirmed revision is read before the entry, so a
        // change set applied after the read cannot confirm an entry it dropped.
        let confirmed = self.copy.confirmed.load(Ordering::Acquire);
        let current =
            |at: i64| revision.is_none_or(|revision| at >= revision || confirmed >= revision);
        match self.copy.lookups.get(&key).await {
            Some(kept) if current(kept.at) => {
                crate::party_binding::request::held_binding(kept.at.max(confirmed));
                looked_up(LOOKUPS, "hit");
                return Ok(kept.lookup.clone());
            }
            Some(_) => looked_up(LOOKUPS, "behind"),
            None => looked_up(LOOKUPS, "miss"),
        }
        self.read_through(key, current, read).await
    }

    /// [`Self::lookup`] past a miss: under `key`'s stripe, the lookup a miss
    /// it waited for just held, or `read` it and hold it unless a drop
    /// overtook the read.
    async fn read_through<E>(
        &self,
        key: LookupKey,
        current: impl Fn(i64) -> bool,
        read: impl Future<Output = Result<Filled, E>>,
    ) -> Result<Lookup, E> {
        let stripe = self.copy.stripe(&key);
        let _reading = if let Ok(reading) = stripe.try_lock() {
            reading
        } else {
            let reading = stripe.lock().await;
            if let Some(kept) = self.copy.lookups.get(&key).await
                && current(kept.at)
            {
                crate::party_binding::request::held_binding(kept.at);
                return Ok(kept.lookup.clone());
            }
            reading
        };
        let seen = self.copy.drops.load(Ordering::Acquire);
        let read = read.await;
        refilled(LOOKUPS, read.is_ok());
        let Filled {
            lookup,
            revision: at,
            mut parties,
        } = read?;
        crate::party_binding::request::held_binding(at);
        parties.sort_unstable();
        parties.dedup();
        let kept = Kept {
            lookup: lookup.clone(),
            at,
            parties: parties.into_boxed_slice(),
        };
        self.copy.lookups.insert(key.clone(), Arc::new(kept)).await;
        if self.copy.drops.load(Ordering::Acquire) != seen {
            self.copy.lookups.invalidate(&key).await;
        }
        Ok(lookup)
    }

    /// The name held for the `PERSON` `party`, or `read` it and hold it
    /// unless a drop overtook the read; with the listener down, `read` it and
    /// hold nothing.
    ///
    /// # Errors
    /// The error of `read`, which is never held.
    pub(crate) async fn name<E>(
        &self,
        party: Uuid,
        read: impl Future<Output = Result<Option<String>, E>>,
    ) -> Result<Option<String>, E> {
        if self.listened.unlistened() {
            return read.await;
        }
        if let Some(held) = self.copy.names.get(&party).await {
            looked_up(NAMES, "hit");
            return Ok(held);
        }
        looked_up(NAMES, "miss");
        let seen = self.copy.drops.load(Ordering::Acquire);
        let read = read.await;
        refilled(NAMES, read.is_ok());
        let name = read?;
        self.copy.names.insert(party, name.clone()).await;
        if self.copy.drops.load(Ordering::Acquire) != seen {
            self.copy.names.invalidate(&party).await;
        }
        Ok(name)
    }
}

/// Count one lookup in `copy` as `result`.
fn looked_up(copy: &'static str, result: &'static str) {
    crate::party_binding::request::defer(move || {
        crate::telemetry::access_path::metrics().copy_lookups.add(
            1,
            &[KeyValue::new("copy", copy), KeyValue::new("result", result)],
        );
    });
}

/// Count one read that refills `copy`, by whether it succeeded.
fn refilled(copy: &'static str, ok: bool) {
    crate::party_binding::request::defer(move || {
        crate::telemetry::access_path::metrics().copy_refills.add(
            1,
            &[
                KeyValue::new("copy", copy),
                KeyValue::new("outcome", if ok { "ok" } else { "failed" }),
            ],
        );
    });
}

/// Count one drop of `scope`.
fn dropped(scope: &'static str) {
    crate::telemetry::access_path::metrics()
        .copy_drops
        .add(1, &[KeyValue::new("scope", scope)]);
}

#[cfg(test)]
#[path = "live_tests.rs"]
mod tests;
