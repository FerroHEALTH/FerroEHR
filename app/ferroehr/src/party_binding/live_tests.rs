// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binding copy holds at most its capacity of lookups and of names, and
//! a lookup or a name the cap evicted is read again with the store's
//! answer, so the cap changes no answer (#5926).

use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use ferrohealth_smart::party::source::{Lookup, PartyRecord};
use ferrohealth_smart::party::{Markers, Membership, PartyRef, PartyState};
use uuid::Uuid;

use super::{BindingCopy, LiveBindings, LookupKey};
use crate::db::domain::Domain;
use crate::policy_register::listen::{Listened, Subscriber};

/// The entries the copies keep in these tests.
const CAP: u64 = 8;

/// More keys than [`CAP`].
const KEYS: usize = 64;

/// A copy of [`CAP`] lookups and names whose listener is connected, so it
/// holds what it reads.
async fn listening() -> LiveBindings {
    let copy = Arc::new(BindingCopy::new(CAP));
    let dropping: Arc<dyn Subscriber> = Arc::<BindingCopy>::clone(&copy);
    let listened = Arc::new(Listened::with_subscriber(
        super::CHANNEL,
        Domain::Party,
        dropping,
    ));
    listened.connected().await;
    LiveBindings { copy, listened }
}

/// The key of the `n`th login.
fn key(n: usize) -> LookupKey {
    LookupKey {
        issuer: "https://idp.example".to_owned(),
        identifier_type: "sub".to_owned(),
        value: format!("login-{n}"),
        group_issuer: n
            .is_multiple_of(2)
            .then(|| "https://groups.example".to_owned()),
        today: "2026-11-17".to_owned(),
    }
}

/// What the store answers for the `n`th login.
fn found(n: usize) -> Lookup {
    if n.is_multiple_of(7) {
        return Lookup::Deprovisioned;
    }
    let party = PartyRef::new("ferroehr", "PERSON", format!("{n}"));
    let ward = PartyRef::new("ferroehr", "GROUP", "ward");
    Lookup::Parties(vec![
        PartyRecord::new(party, PartyState::Active, Markers::PROFESSIONAL)
            .with_memberships(vec![Membership::new(ward, "ward-1")]),
    ])
}

#[tokio::test]
async fn the_lookups_stay_under_the_cap_and_an_evicted_one_reads_the_store() {
    let bindings = listening().await;
    let reads = AtomicUsize::new(0);
    for round in 0..2 {
        for n in 0..KEYS {
            let read = async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(found(n))
            };
            let lookup = bindings
                .lookup(key(n), read)
                .await
                .expect("the read succeeds");
            assert_eq!(lookup, found(n), "round {round}, login {n}");
        }
        bindings.copy.lookups.run_pending_tasks().await;
        let (lookups, _) = bindings.held();
        assert!(lookups.entries <= CAP, "{lookups:?} within {CAP}");
        assert!(lookups.bytes > 0, "{lookups:?} weighs its entries");
    }
    assert!(
        reads.load(Ordering::Relaxed) > KEYS,
        "the evicted lookups were read again"
    );
}

#[tokio::test]
async fn the_names_stay_under_the_cap_and_an_evicted_one_reads_the_store() {
    let bindings = listening().await;
    let reads = AtomicUsize::new(0);
    let parties: Vec<(Uuid, Option<String>)> = (0..KEYS)
        .map(|n| (Uuid::now_v7(), (n % 3 != 0).then(|| format!("Person {n}"))))
        .collect();
    for round in 0..2 {
        for (party, name) in &parties {
            let read = async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(name.clone())
            };
            let answer = bindings
                .name(*party, read)
                .await
                .expect("the read succeeds");
            assert_eq!(&answer, name, "round {round}");
        }
        bindings.copy.names.run_pending_tasks().await;
        let (_, names) = bindings.held();
        assert!(names.entries <= CAP, "{names:?} within {CAP}");
    }
    assert!(
        reads.load(Ordering::Relaxed) > KEYS,
        "the evicted names were read again"
    );
}
