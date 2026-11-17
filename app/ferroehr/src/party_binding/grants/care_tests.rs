// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The care copy's bound weighs each census by its patients and the record
//! subjects they declare (#5760).

use std::collections::BTreeMap;
use std::sync::Arc;

use uuid::Uuid;

use super::{CareCopy, Census};
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
    let copy = CareCopy::bounded(bound);
    copy.teams.insert(one, Arc::new(declaring.0)).await;
    copy.teams.insert(two, Arc::new(declaring.1)).await;
    copy.teams.run_pending_tasks().await;
    let held = copy.held();
    assert_eq!(
        held.teams, 1,
        "the declared subjects of both teams do not fit"
    );
    assert!(held.bytes <= bound, "{held:?} within {bound}");
    assert_eq!(held.patients, 1000);
    assert_eq!(held.declared, 1000);
}
