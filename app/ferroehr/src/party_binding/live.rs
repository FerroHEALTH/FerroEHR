// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! [`LiveBindings`]: this replica's copy of what resolves a login to a
//! `PERSON`.
//!
//! It holds the lookups of the binding side table and the names of the bound
//! `PERSON`s, so a request whose caller is held reads neither (#5771).
//! An entry is read on a miss and kept until a notification on [`CHANNEL`]
//! drops it: a change of the side table drops every lookup, and a write of a
//! `PERSON`'s version drops its name
//! (`migrations/party/0006_baseline_binding.sql`). A listener that connects or
//! loses its connection drops everything, and while it is down every request
//! reads the store. A request whose own commit changed the side table drops
//! every lookup and name before it answers ([`tracking`]), so the change holds
//! on this replica's next request whatever the notification's delay. A lookup
//! is held for the date it was read on, since the professional and
//! representation states it carries are judged on that date. A read that a
//! drop overtook keeps nothing. No openEHR spec governs resolving a login to a
//! party: our own design.

use crate::db::domain::Domain;
use std::future::Future;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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

/// The `copy` label of the lookups.
const LOOKUPS: &str = "binding";
/// The `copy` label of the names.
const NAMES: &str = "party_name";

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
pub async fn tracking<F: Future>(fut: F) -> (F::Output, bool) {
    CHANGED
        .scope(AtomicBool::new(false), async {
            let output = fut.await;
            let changed = CHANGED.with(|changed| changed.load(Ordering::Acquire));
            (output, changed)
        })
        .await
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

/// The lookups and names this replica holds, dropped by the notifications of
/// [`CHANNEL`].
#[derive(Debug)]
struct BindingCopy {
    /// Each lookup with the binding revision of the store it was read at.
    lookups: Cache<LookupKey, (Lookup, i64)>,
    names: Cache<Uuid, Option<String>>,
    /// Moved by every drop, so a read that started before one keeps nothing.
    drops: AtomicU64,
}

impl BindingCopy {
    fn new(capacity: u64) -> Self {
        Self {
            lookups: Cache::builder().max_capacity(capacity).build(),
            names: Cache::builder().max_capacity(capacity).build(),
            drops: AtomicU64::new(0),
        }
    }

    /// Drop every lookup.
    fn forget_lookups(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped(LOOKUPS);
        self.lookups.invalidate_all();
    }

    /// Drop the name of `party`.
    async fn forget_name(&self, party: Uuid) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped(NAMES);
        self.names.invalidate(&party).await;
    }

    /// Drop every lookup and every name.
    fn forget_all(&self) {
        self.drops.fetch_add(1, Ordering::AcqRel);
        dropped("every");
        self.lookups.invalidate_all();
        self.names.invalidate_all();
    }
}

impl Subscriber for BindingCopy {
    fn notified<'a>(&'a self, payload: &'a str) -> Dropping<'a> {
        Box::pin(async move {
            // NOTE: our own design, the side table's triggers notify an empty
            // payload and a PERSON's version its container; any other drops all.
            match payload {
                "" => self.forget_lookups(),
                _ => match payload.parse::<Uuid>() {
                    Ok(party) => self.forget_name(party).await,
                    Err(_) => self.forget_all(),
                },
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

    /// Whether the listener is down, so a lookup reads the store and holds
    /// nothing; such a read is counted.
    pub(crate) fn unlistened(&self) -> bool {
        self.listened.unlistened()
    }

    /// The lookup held for `key` when it was read at the store's binding
    /// revision `revision`, which the request read, or at a later one; else
    /// `read` it with the revision it was read at and hold it unless a drop
    /// overtook the read. A lookup read before `revision` is older than a
    /// change of the side table whose notification has not arrived, and is
    /// read again (#6048).
    ///
    /// # Errors
    /// The error of `read`, which is never held.
    pub(crate) async fn lookup<E>(
        &self,
        key: LookupKey,
        revision: i64,
        read: impl Future<Output = Result<(Lookup, i64), E>>,
    ) -> Result<Lookup, E> {
        match self.copy.lookups.get(&key).await {
            Some((held, at)) if at >= revision => {
                looked_up(LOOKUPS, "hit");
                return Ok(held);
            }
            Some(_) => looked_up(LOOKUPS, "behind"),
            None => looked_up(LOOKUPS, "miss"),
        }
        let seen = self.copy.drops.load(Ordering::Acquire);
        let read = read.await;
        refilled(LOOKUPS, read.is_ok());
        let (lookup, at) = read?;
        self.copy
            .lookups
            .insert(key.clone(), (lookup.clone(), at))
            .await;
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
    crate::telemetry::access_path::metrics().copy_lookups.add(
        1,
        &[KeyValue::new("copy", copy), KeyValue::new("result", result)],
    );
}

/// Count one read that refills `copy`, by whether it succeeded.
fn refilled(copy: &'static str, ok: bool) {
    crate::telemetry::access_path::metrics().copy_refills.add(
        1,
        &[
            KeyValue::new("copy", copy),
            KeyValue::new("outcome", if ok { "ok" } else { "failed" }),
        ],
    );
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
