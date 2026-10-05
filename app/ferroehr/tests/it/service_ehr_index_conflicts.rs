// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The advisory EHR-Index duplicate detection
//! (`FerroEhrService::index_conflicts`) against a real `PostgreSQL` 18 (shared
//! testkit harness).
//!
//! `master07 §Overview` names the two error states the index metadata exists
//! "to detect and rectify": multiple EHRs recorded for one subject, and
//! multiple subjects recorded for one EHR. The SM declares no detection
//! OPERATION, so the read itself is our own design — what these tests pin is
//! that it reports exactly those two states, carries every association of each,
//! is purely advisory (it mutates nothing and never refuses), and clears the
//! moment the operator rectifies through the SM write operations (I4/I5).

#![expect(
    clippy::expect_used,
    clippy::panic,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions and direct \
              fixture indexing are the intended shape here (the Rust Book ch11)"
)]

use std::collections::BTreeSet;

use ferroehr::service::FerroEhrService;
use ferroehr::service::ehr_index::conflicts::IndexConflict;
use ferroehr::service::ehr_index::types::{
    EhrIndexEntry, ResourceInstanceType, ResourceStatus, SubjectRef,
};

use crate::admin_fixture::repository;

/// Every conflict one scan reports.
async fn scan(svc: &FerroEhrService) -> Vec<IndexConflict> {
    svc.index_conflicts()
        .await
        .expect("the advisory scan never refuses")
}

/// Associate `subject` with a freshly created EHR, returning that EHR's id.
async fn associate(svc: &FerroEhrService, subject: &SubjectRef) -> String {
    let ehr = svc.create_ehr(None).await.expect("ehr").to_string();
    svc.add_ehr_subject(ehr.clone(), subject.clone(), None, None)
        .await
        .expect("add subject");
    ehr
}

/// The EHR ids an association group names, as a set — the order of a
/// subject-keyed group is fixed by its own query and asserted where it matters.
fn ehr_ids(entries: &[EhrIndexEntry]) -> BTreeSet<String> {
    entries.iter().map(|e| e.ehr_id.clone()).collect()
}

/// The one `SubjectWithMultipleEhrs` conflict in `conflicts`.
fn only_subject_conflict(conflicts: &[IndexConflict]) -> (&SubjectRef, &[EhrIndexEntry]) {
    assert_eq!(
        conflicts.len(),
        1,
        "one conflicting subject, got {conflicts:?}"
    );
    match conflicts.first() {
        Some(IndexConflict::SubjectWithMultipleEhrs { subject, entries }) => (subject, entries),
        other => panic!("expected a SubjectWithMultipleEhrs conflict, got {other:?}"),
    }
}

/// A clean index reports nothing: neither an empty index nor a set of
/// well-formed 1:1 associations is an error state.
#[tokio::test]
async fn a_one_to_one_index_reports_no_conflict() {
    let (_db, _pool, svc) = repository().await;

    let empty = scan(&svc).await;
    assert!(empty.is_empty(), "an empty index has nothing to report");

    associate(&svc, &SubjectRef::person("PID-A", "mpi")).await;
    associate(&svc, &SubjectRef::person("PID-B", "mpi")).await;

    let conflicts = scan(&svc).await;
    assert!(
        conflicts.is_empty(),
        "two subjects on two EHRs is the normal state, got {conflicts:?}"
    );
}

/// The "multiple EHRs … created in different locations" state: one subject
/// associated with more than one EHR is reported once, carrying EVERY
/// association of that subject so the operator can pick the `Primary`.
#[tokio::test]
async fn one_subject_on_several_ehrs_is_reported_with_all_its_associations() {
    let (_db, _pool, svc) = repository().await;

    let subject = SubjectRef::person("PID-SHARED", "mpi");
    let first = associate(&svc, &subject).await;
    let second = associate(&svc, &subject).await;
    let third = associate(&svc, &subject).await;
    // An unrelated 1:1 association must not appear in the report.
    let lone = associate(&svc, &SubjectRef::person("PID-LONE", "mpi")).await;

    let conflicts = scan(&svc).await;
    let (reported, entries) = only_subject_conflict(&conflicts);
    assert_eq!(*reported, subject, "the conflicting subject is named");
    assert_eq!(
        ehr_ids(entries),
        BTreeSet::from([first, second, third]),
        "every association of the subject is carried, and only those"
    );
    assert!(
        entries.iter().all(|e| e.subject == subject),
        "a 1:1 association ({lone}) is not part of the conflict, got {entries:?}"
    );
}

/// The conflicting subject is reported with its STORED type, not the `PERSON`
/// default the scan starts from — the grouping key is (id, namespace) only, so
/// an `ORGANISATION` subject must not be reported as a person.
#[tokio::test]
async fn the_reported_subject_carries_the_stored_type() {
    let (_db, _pool, svc) = repository().await;

    let subject = SubjectRef {
        id: "ORG-1".to_owned(),
        namespace: "registry".to_owned(),
        r#type: "ORGANISATION".to_owned(),
    };
    associate(&svc, &subject).await;
    associate(&svc, &subject).await;

    let conflicts = scan(&svc).await;
    let (reported, entries) = only_subject_conflict(&conflicts);
    assert_eq!(reported.r#type, "ORGANISATION");
    assert_eq!(*reported, subject);
    assert!(
        entries.iter().all(|e| e.subject.r#type == "ORGANISATION"),
        "every carried association keeps the stored type, got {entries:?}"
    );
}

/// The "records merged … multiple subject ids" state: one EHR associated with
/// more than one subject is reported once, carrying every association ordered
/// by subject key, with the stored `RESOURCE_INSTANCE_TYPE` intact so the
/// operator can see which association was already flagged `Duplicate`.
#[tokio::test]
async fn one_ehr_with_several_subjects_is_reported_with_all_its_associations() {
    let (_db, _pool, svc) = repository().await;

    let primary = SubjectRef::person("PID-1", "mpi");
    let duplicate = SubjectRef::person("PID-2", "mpi");
    let ehr = associate(&svc, &primary).await;
    svc.add_ehr_subject(
        ehr.clone(),
        duplicate.clone(),
        Some(ResourceStatus {
            instance_type: ResourceInstanceType::Duplicate,
            ..ResourceStatus::default()
        }),
        None,
    )
    .await
    .expect("second subject");

    let conflicts = scan(&svc).await;
    assert_eq!(conflicts.len(), 1, "one conflicting EHR, got {conflicts:?}");
    let (ehr_id, entries) = match conflicts.first() {
        Some(IndexConflict::EhrWithMultipleSubjects { ehr_id, entries }) => (ehr_id, entries),
        other => panic!("expected an EhrWithMultipleSubjects conflict, got {other:?}"),
    };
    assert_eq!(ehr_id.to_string(), ehr, "the conflicting EHR is named");
    // `ORDER BY subject_id, subject_namespace`: PID-1 before PID-2.
    assert_eq!(entries.len(), 2, "both associations are carried");
    assert_eq!(
        entries.iter().map(|e| &e.subject).collect::<Vec<_>>(),
        vec![&primary, &duplicate]
    );
    assert_eq!(
        entries
            .iter()
            .map(|e| e.status.instance_type)
            .collect::<Vec<_>>(),
        vec![
            ResourceInstanceType::Primary,
            ResourceInstanceType::Duplicate
        ],
        "each association keeps its stored instance type"
    );
}

/// An index in BOTH error states reports both, and the scan is advisory: it
/// never refuses, and it changes nothing — the associations read back exactly
/// as they were, and a second scan reports the same thing.
#[tokio::test]
async fn both_error_states_are_reported_together_and_the_scan_mutates_nothing() {
    let (_db, _pool, svc) = repository().await;

    let shared = SubjectRef::person("PID-SHARED", "mpi");
    let other = SubjectRef::person("PID-OTHER", "mpi");
    let first = associate(&svc, &shared).await;
    let second = associate(&svc, &shared).await;
    // …and the second EHR also carries a second subject.
    svc.add_ehr_subject(second.clone(), other.clone(), None, None)
        .await
        .expect("second subject");

    let before = svc.ehr_subjects(second.clone()).await.expect("read before");
    let conflicts = scan(&svc).await;
    assert_eq!(
        conflicts.len(),
        2,
        "both states reported, got {conflicts:?}"
    );
    assert!(
        conflicts.iter().any(|c| matches!(
            c,
            IndexConflict::SubjectWithMultipleEhrs { subject, entries }
                if *subject == shared
                    && ehr_ids(entries) == BTreeSet::from([first.clone(), second.clone()])
        )),
        "the shared subject's multiple EHRs are reported, got {conflicts:?}"
    );
    assert!(
        conflicts.iter().any(|c| matches!(
            c,
            IndexConflict::EhrWithMultipleSubjects { ehr_id, entries }
                if ehr_id.to_string() == second
                    && entries.iter().any(|e| e.subject == other)
                    && entries.len() == 2
        )),
        "the second EHR's multiple subjects are reported, got {conflicts:?}"
    );

    let after = svc.ehr_subjects(second).await.expect("read after");
    assert_eq!(after, before, "detection is a read: it mutates nothing");
    assert_eq!(
        scan(&svc).await.len(),
        2,
        "a repeated scan reports the same states"
    );
}

/// Rectification through the SM write operations clears the report: the scan is
/// a live read over the index, never a stored flag. `remove_ehr_subject` (I4)
/// drops one association and `remove_subject` (I5) drops them all.
#[tokio::test]
async fn rectifying_through_the_sm_writes_clears_the_report() {
    let (_db, _pool, svc) = repository().await;

    let shared = SubjectRef::person("PID-SHARED", "mpi");
    let other = SubjectRef::person("PID-OTHER", "mpi");
    let first = associate(&svc, &shared).await;
    let second = associate(&svc, &shared).await;
    svc.add_ehr_subject(second.clone(), other.clone(), None, None)
        .await
        .expect("second subject");
    assert_eq!(scan(&svc).await.len(), 2);

    // I4: drop the extra subject from the second EHR — that state clears, the
    // subject-with-multiple-EHRs one remains.
    svc.remove_ehr_subject(second, other)
        .await
        .expect("remove the duplicate subject");
    let conflicts = scan(&svc).await;
    let (reported, _entries) = only_subject_conflict(&conflicts);
    assert_eq!(*reported, shared, "only the shared subject is left");

    // I4 again: with one association left, a subject is no longer duplicated.
    svc.remove_ehr_subject(first, shared.clone())
        .await
        .expect("remove the duplicate association");
    let rectified = scan(&svc).await;
    assert!(
        rectified.is_empty(),
        "a rectified index reports nothing, got {rectified:?}"
    );

    // I5: dropping the subject entirely leaves the index clean too.
    svc.remove_subject(shared).await.expect("remove subject");
    let cleared = scan(&svc).await;
    assert!(cleared.is_empty(), "got {cleared:?}");
}
