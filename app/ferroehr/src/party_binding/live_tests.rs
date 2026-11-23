// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The binding copy holds at most its capacity of lookups and of names, and
//! a lookup or a name the cap evicted is read again with the store's
//! answer, so the cap changes no answer (#5926). Concurrent misses of one
//! login share one store read (#6374).

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
                Ok::<_, Infallible>((found(n), 0))
            };
            let lookup = bindings
                .lookup(key(n), Some(0), read)
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

/// A held lookup is taken at the binding revision it was read at and at none
/// after it: a request that reads a later revision from the store reads the
/// lookup again, whatever the notification's delay (#6048).
#[tokio::test]
async fn a_lookup_read_before_the_stores_revision_is_read_again() {
    let bindings = listening().await;
    let reads = AtomicUsize::new(0);
    let at = |revision: i64, read_at: i64| {
        let (bindings, reads) = (&bindings, &reads);
        async move {
            let read = async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>((found(1), read_at))
            };
            bindings
                .lookup(key(1), Some(revision), read)
                .await
                .expect("the read succeeds")
        }
    };
    at(3, 3).await;
    at(3, 3).await;
    assert_eq!(reads.load(Ordering::Relaxed), 1, "held at its revision");
    at(2, 2).await;
    assert_eq!(reads.load(Ordering::Relaxed), 1, "held for an older read");
    at(4, 5).await;
    assert_eq!(reads.load(Ordering::Relaxed), 2, "read again past it");
    at(5, 5).await;
    assert_eq!(
        reads.load(Ordering::Relaxed),
        2,
        "held at the revision read"
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

/// What `misses` concurrent lookups of login 1 at `revision` answer, and how
/// many store reads they made, each read answering `answer` after a pause.
async fn concurrent<E: Clone>(
    bindings: &LiveBindings,
    misses: usize,
    revision: i64,
    answer: Result<(Lookup, i64), E>,
) -> (Vec<Result<Lookup, E>>, usize) {
    let reads = AtomicUsize::new(0);
    let lookups = (0..misses).map(|_| {
        let read = async {
            reads.fetch_add(1, Ordering::Relaxed);
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
            answer.clone()
        };
        bindings.lookup(key(1), Some(revision), read)
    });
    let answers = futures::future::join_all(lookups).await;
    (answers, reads.load(Ordering::Relaxed))
}

/// Concurrent misses of one login on a cold copy share one store read and
/// are all answered what the store holds (#6374).
#[tokio::test]
async fn concurrent_misses_of_one_login_make_one_store_read() {
    let bindings = listening().await;
    let (answers, reads) = concurrent(&bindings, 16, 0, Ok::<_, Infallible>((found(1), 0))).await;
    for answer in answers {
        assert_eq!(answer.expect("the read succeeds"), found(1));
    }
    assert_eq!(reads, 1, "one store read");
}

/// A failed read holds nothing, so each waiter behind it reads on its own.
#[tokio::test]
async fn a_failed_read_leaves_each_miss_to_read_on_its_own() {
    let bindings = listening().await;
    let (answers, reads) = concurrent(&bindings, 4, 0, Err::<(Lookup, i64), _>("down")).await;
    for answer in answers {
        assert_eq!(answer, Err("down"));
    }
    assert_eq!(reads, 4, "each miss read the store");
    assert!(
        bindings.copy.lookups.get(&key(1)).await.is_none(),
        "nothing held"
    );
}

/// Concurrent lookups that find the held lookup behind the store's revision
/// share one read of it.
#[tokio::test]
async fn concurrent_lookups_behind_the_store_make_one_store_read() {
    let bindings = listening().await;
    let (_, reads) = concurrent(&bindings, 1, 1, Ok::<_, Infallible>((found(1), 1))).await;
    assert_eq!(reads, 1, "the first read");
    let (answers, reads) = concurrent(&bindings, 16, 2, Ok::<_, Infallible>((found(2), 2))).await;
    for answer in answers {
        assert_eq!(answer.expect("the read succeeds"), found(2));
    }
    assert_eq!(reads, 1, "one read past the held revision");
}
