// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care copy's bound weighs each census by its patients and the record
//! subjects they declare (#5760), and each cap holds while an evicted entry
//! is read again with the store's answer (#5926).

use std::collections::BTreeMap;
use std::convert::Infallible;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use uuid::Uuid;

use super::{CENSUS_BYTES, CareCopy, Census, SUBJECTS};
use crate::party_binding::grants::caller::CareTeam;
use crate::party_binding::grants::store::Dates;
use crate::storage::access_projection::EhrSubject;

/// A census of `patients` under `team`, each declaring `each` record
/// subjects of an identity source's namespace.
fn census(team: Uuid, patients: usize, each: usize) -> Census {
    let dates = Dates {
        lower: None,
        upper: None,
    };
    let cared: Vec<_> = (0..patients)
        .map(|_| (Uuid::now_v7(), CareTeam::new(team, dates)))
        .collect();
    let declared: BTreeMap<Uuid, Vec<EhrSubject>> = cared
        .iter()
        .filter(|_| each > 0)
        .map(|(patient, _)| {
            let subjects = (0..each)
                .map(|n| {
                    EhrSubject::new(
                        "urn:oid:2.16.840.1.113883.2.4.6.3",
                        format!("{patient}-{n}"),
                    )
                })
                .collect();
            (*patient, subjects)
        })
        .collect();
    Census { cared, declared }
}

#[test]
fn a_census_weighs_the_subjects_its_patients_declare() {
    let team = Uuid::now_v7();
    let bare = census(team, 1000, 0);
    let declaring = census(team, 1000, 1);
    let subject_text = declaring
        .declared
        .values()
        .flatten()
        .map(|subject| subject.namespace.len() + subject.id.len())
        .sum::<usize>();
    let floor = u64::try_from(1000 * size_of::<EhrSubject>() + subject_text).expect("fits");
    assert!(
        declaring.bytes() >= bare.bytes() + floor,
        "declared {} against bare {} and the subjects' {floor}",
        declaring.bytes(),
        bare.bytes(),
    );
}

#[tokio::test]
async fn the_bound_holds_the_declared_subjects_too() {
    let (one, two) = (Uuid::now_v7(), Uuid::now_v7());
    let bare = census(one, 1000, 0).bytes();
    let declaring = (census(one, 1000, 1), census(two, 1000, 1));
    let bound = declaring.0.bytes() + bare;
    assert!(
        2 * bare <= bound,
        "the patients of both teams alone fit the bound"
    );
    let copy = CareCopy::bounded(SUBJECTS, bound);
    copy.teams.insert(one, Arc::new(declaring.0)).await;
    copy.teams.insert(two, Arc::new(declaring.1)).await;
    copy.teams.run_pending_tasks().await;
    let held = copy.held();
    assert_eq!(
        held.teams.entries, 1,
        "the declared subjects of both teams do not fit"
    );
    assert!(held.census_bytes <= bound, "{held:?} within {bound}");
    assert_eq!(held.patients.entries, 1000);
    assert_eq!(held.declared.entries, 1000);
}

/// The entries the copies keep in these tests.
const CAP: u64 = 8;

/// More keys than [`CAP`].
const KEYS: u64 = 64;

/// The care relationships the store holds for `subject` in these tests.
fn relationships(subject: Uuid) -> Arc<[CareTeam]> {
    let dates = Dates {
        lower: None,
        upper: None,
    };
    vec![CareTeam::new(subject, dates)].into()
}

#[tokio::test]
async fn the_subjects_stay_under_their_cap_and_an_evicted_one_reads_the_store() {
    let copy = CareCopy::bounded(CAP, CENSUS_BYTES);
    let reads = AtomicUsize::new(0);
    let subjects: Vec<Uuid> = (0..KEYS).map(|_| Uuid::now_v7()).collect();
    for round in 0..2 {
        for subject in &subjects {
            let read = async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(relationships(*subject))
            };
            let answer = copy
                .hold(&copy.subjects, *subject, read, |_| true)
                .await
                .expect("the read succeeds");
            assert_eq!(
                answer,
                relationships(*subject),
                "round {round}: the copy answers what the store holds"
            );
        }
        copy.subjects.run_pending_tasks().await;
        let held = copy.held().subjects;
        assert!(held.entries <= CAP, "{held:?} within {CAP}");
        assert!(held.bytes > 0, "{held:?} weighs its entries");
    }
    assert!(
        reads.load(Ordering::Relaxed) > usize::try_from(KEYS).expect("fits"),
        "the evicted subjects were read again"
    );
}

#[tokio::test]
async fn the_persons_stay_under_their_cap_and_an_evicted_one_reads_the_store() {
    let copy = CareCopy::bounded(CAP, CENSUS_BYTES);
    let reads = AtomicUsize::new(0);
    let declared: Vec<(EhrSubject, Option<Uuid>)> = (0..KEYS)
        .map(|n| {
            let person = (n % 3 != 0).then(Uuid::now_v7);
            (
                EhrSubject::new("urn:oid:2.16.840.1.113883.2.4.6.3", format!("{n}")),
                person,
            )
        })
        .collect();
    for round in 0..2 {
        for (subject, person) in &declared {
            let read = async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(*person)
            };
            let answer = copy
                .hold(&copy.persons, subject.clone(), read, |_| true)
                .await
                .expect("the read succeeds");
            assert_eq!(answer, *person, "round {round}: the store's PERSON");
        }
        copy.persons.run_pending_tasks().await;
        let held = copy.held().persons;
        assert!(held.entries <= CAP, "{held:?} within {CAP}");
    }
    assert!(
        reads.load(Ordering::Relaxed) > usize::try_from(KEYS).expect("fits"),
        "the evicted record subjects were read again"
    );
}

#[tokio::test]
async fn the_censuses_stay_under_their_bound_and_an_evicted_one_reads_the_store() {
    let teams: Vec<Uuid> = (0..16).map(|_| Uuid::now_v7()).collect();
    let one = census(Uuid::now_v7(), 100, 1).bytes();
    let bound = 4 * one;
    let copy = CareCopy::bounded(SUBJECTS, bound);
    let reads = AtomicUsize::new(0);
    let stored: BTreeMap<Uuid, Arc<Census>> = teams
        .iter()
        .map(|team| (*team, Arc::new(census(*team, 100, 1))))
        .collect();
    for _round in 0..2 {
        for team in &teams {
            let held = stored.get(team).map(Arc::clone).expect("a stored census");
            let read = async {
                reads.fetch_add(1, Ordering::Relaxed);
                Ok::<_, Infallible>(Arc::clone(&held))
            };
            let answer = copy
                .hold(&copy.teams, *team, read, |census| census.bytes() <= bound)
                .await
                .expect("the read succeeds");
            assert_eq!(answer.cared, held.cared, "the store's patients");
            assert_eq!(answer.declared, held.declared, "the store's subjects");
        }
        copy.teams.run_pending_tasks().await;
        let held = copy.held();
        assert!(held.census_bytes <= bound, "{held:?} within {bound}");
        assert!(held.teams.entries < 16, "{held:?} evicted some teams");
        assert_eq!(held.patients.entries, 100 * held.teams.entries);
    }
    assert!(
        reads.load(Ordering::Relaxed) > 16,
        "the evicted censuses were read again"
    );
}

#[tokio::test]
async fn a_census_past_the_bound_is_answered_and_never_held() {
    let team = Uuid::now_v7();
    let large = Arc::new(census(team, 100, 1));
    let bound = large.bytes() - 1;
    let copy = CareCopy::bounded(SUBJECTS, bound);
    for _ in 0..2 {
        let answer = copy
            .hold(
                &copy.teams,
                team,
                async { Ok::<_, Infallible>(Arc::clone(&large)) },
                |census| census.bytes() <= bound,
            )
            .await
            .expect("the read succeeds");
        assert_eq!(answer.cared, large.cared);
    }
    copy.teams.run_pending_tasks().await;
    assert_eq!(copy.held().teams.entries, 0);
}
