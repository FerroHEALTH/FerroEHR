// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Care relationship commits that contend on one relationship (#6257). A
//! relationship write judges its pre-checks on one gate read, holds its
//! access record first and commits behind one guarded version statement,
//! which takes the key-space gate shared; so of several updates or deletes
//! naming one preceding version one commits, every other answers as a stale
//! write does, its records are discarded at once, and the rows the winner
//! leaves are those a refill writes. No openEHR spec governs how a commit is
//! run against the database: our own design.

#![expect(
    clippy::disallowed_methods,
    reason = "a test holds and reads rows on raw pool connections, outside any request"
)]

use std::sync::Arc;
use std::time::Duration;

use ferroehr::db::domain::DomainPools;
use ferroehr::error::ServiceError;
use ferroehr::error::refusal::{Malformed, Refusal};
use ferroehr::ids::VoId;
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::types::PartyKind;
use ferroehr::service::version_update::WriteRequest;
use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::recorder::{WriteRecorder, with_recorder};
use ferroehr::system_log::sender;
use sqlx::PgPool;
use uuid::Uuid;

use crate::care_bounds::{care_of, interval, party};
use crate::fixtures::{delete_precondition, if_match, object};
use crate::party_binding_source::{LOGIN, PERSON_TEMPLATE, create, key, mapping, person, version};
use crate::party_commit_contention::{held, primed};
use crate::policy_fixtures::{CARE_PROVIDER, care_provider};
use crate::relationship_commit_parity::relate;

/// The advisory lock a party commit's guarded statement takes shared
/// (`ferroehr::party_binding::key_space`).
const GATE: i64 = 0x5013_b1d1;

/// A service over a fresh database with the binding mapping installed, so
/// each relationship commit takes the key-space gate, and its writes recorded
/// when `audited`.
async fn service(audited: bool) -> (testkit::TestDb, Arc<FerroEhrService>) {
    let db = testkit::db().await.expect("testkit database");
    crate::fixtures::open_key_spaces(&db, &[LOGIN]).await;
    let mut svc = FerroEhrService::new(&DomainPools::from_shared(&db.pool()));
    if audited {
        let config = AuditConfig {
            enabled: true,
            ..AuditConfig::default()
        };
        let (sender, handle) = sender::start(config, None, db.pool())
            .await
            .expect("the audit sender");
        drop(handle);
        svc = svc.with_audit(sender);
    }
    (db, Arc::new(svc.with_binding_mapping(mapping())))
}

/// A transaction that holds the key-space gate exclusive, so every
/// relationship write judges its gate read and then waits at its guarded
/// statement until the transaction ends.
async fn hold(pool: &PgPool) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = pool.begin().await.expect("begin the hold");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(GATE)
        .execute(&mut *tx)
        .await
        .expect("hold the key-space gate");
    tx
}

/// The patient and the team a care relationship joins.
struct Scene {
    patient: VoId,
    team: String,
}

impl Scene {
    /// The care relationship of the patient with the team over November
    /// to its day `upper`.
    fn stay(&self, upper: u8) -> serde_json::Value {
        care_of(
            self.patient,
            ("GROUP", &self.team),
            "ATND",
            &interval("2026-11-01", Some(&format!("2026-11-{upper:02}"))),
        )
    }
}

/// The patient, the team and the provider of a care relationship, committed.
async fn scene(svc: &FerroEhrService) -> Scene {
    let (patient, _) = create(
        svc,
        PartyKind::Person,
        &person(
            PERSON_TEMPLATE,
            &[key("8b9c0d1e-2f3a-4b4c-9d5e-7f8a9b0c1d2e")],
        ),
    )
    .await;
    care_provider(svc, CARE_PROVIDER).await;
    let (team, _) = create(svc, PartyKind::Group, &party("GROUP", "Cardiology")).await;
    Scene {
        patient,
        team: team.to_string(),
    }
}

/// One write of the care relationship `vo` at `uid`: an update naming `uid`
/// to an end of its own, or its delete when `delete`.
async fn write(
    svc: &FerroEhrService,
    scene: &Scene,
    (vo, uid): (VoId, &str),
    n: usize,
    delete: bool,
) -> Result<(), ServiceError> {
    if delete {
        return Box::pin(svc.party_relationship_delete(
            object(uid),
            delete_precondition(uid, None),
            WriteRequest::new(()),
        ))
        .await
        .map(|_| ());
    }
    let upper = u8::try_from(n).map_or(28, |n| 10_u8.saturating_add(n).min(28));
    Box::pin(svc.party_relationship_update(
        vo,
        if_match(uid),
        version(&scene.stay(upper), "532", Some(uid)),
    ))
    .await
    .map(|_| ())
}

/// `writers` writes of one care relationship naming its one version, held at
/// their guarded statement, each with its recorder: each outcome and
/// recorder.
async fn contend(
    db: &testkit::TestDb,
    svc: &Arc<FerroEhrService>,
    writers: usize,
    delete: bool,
) -> Vec<(Result<(), ServiceError>, WriteRecorder)> {
    let scene = Arc::new(scene(svc).await);
    let (vo, uid) = relate(svc, &scene.stay(30)).await;
    let gate = hold(&db.pool()).await;
    let tasks: Vec<_> = (0..writers)
        .map(|n| {
            let svc = Arc::clone(svc);
            let scene = Arc::clone(&scene);
            let uid = uid.clone();
            let recorder = primed();
            let kept = recorder.clone();
            let task = tokio::spawn(ferroehr::db::acquire::request_scope(with_recorder(
                recorder,
                async move { write(&svc, &scene, (vo, &uid), n, delete).await },
            )));
            (task, kept)
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(400)).await;
    gate.commit().await.expect("release the hold");
    let mut outcomes = Vec::new();
    for (task, recorder) in tasks {
        outcomes.push((task.await.expect("the writer completes"), recorder));
    }
    outcomes
}

/// `writers` updates, or deletes when `delete`, of one care relationship
/// naming the same preceding version: one commits; every other update
/// answers `412` for the moved tip, and every other delete answers `400` for
/// the relationship it finds already deleted, the check a delete makes before
/// its precondition. The rows the winner leaves are those a refill writes.
async fn one_relationship_write_wins(writers: usize, delete: bool) {
    let (db, svc) = service(false).await;
    let mut committed = 0;
    for (outcome, _) in contend(&db, &svc, writers, delete).await {
        match outcome {
            Ok(()) => committed += 1,
            Err(ServiceError::VersionConflict(_)) if !delete => {}
            Err(ServiceError::BadRequest(Refusal {
                kind: Malformed::Precondition,
                ..
            })) if delete => {}
            Err(error) => panic!("a losing relationship write answers as a stale one: {error:?}"),
        }
    }
    assert_eq!(committed, 1, "exactly one relationship write wins");
    let pool = db.pool();
    let rows = crate::party_binding_source::side_rows(&pool).await;
    svc.refill_binding_table().await.expect("the refill runs");
    assert_eq!(
        crate::party_binding_source::side_rows(&pool).await,
        rows,
        "the winner leaves the rows a refill writes"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_two_care_relationship_updates_wins() {
    one_relationship_write_wins(2, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_eight_care_relationship_updates_wins() {
    one_relationship_write_wins(8, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_two_care_relationship_deletes_wins() {
    one_relationship_write_wins(2, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_eight_care_relationship_deletes_wins() {
    one_relationship_write_wins(8, true).await;
}

/// Recorded updates of one care relationship naming its one version, held
/// at their guarded statement: the one that commits holds its record under
/// the reservation its statement committed; every other answers `412` after
/// its records were held, discards them at once and hands its operation
/// record back, so nothing of it is held for any read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_relationship_write_refused_after_its_records_leaves_no_held_record() {
    let (db, svc) = service(true).await;
    let pool = db.pool();
    let before = held(&pool).await;
    let mut kept = Vec::new();
    for (outcome, recorder) in contend(&db, &svc, 4, false).await {
        match outcome {
            Ok(()) => kept.extend(recorder.reserved().into_iter().map(|r| r.id)),
            Err(refused) => {
                assert!(
                    matches!(refused, ServiceError::VersionConflict(_)),
                    "a losing update answers 412: {refused:?}"
                );
                assert!(recorder.reserved().is_empty(), "it keeps no reservation");
                assert!(
                    !recorder.holds_operation_record(),
                    "its operation record goes back to the adapter"
                );
            }
        }
    }
    assert_eq!(kept.len(), 1, "one update commits");
    let reserved: bool =
        sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM party.audit_reservation WHERE id = $1)")
            .bind(kept[0])
            .fetch_one(&pool)
            .await
            .expect("the reservation");
    assert!(reserved, "the statement committed the reservation");
    let mut now: Vec<Uuid> = held(&pool)
        .await
        .into_iter()
        .filter(|r| !before.contains(r))
        .collect();
    now.sort_unstable();
    assert_eq!(now, kept, "only the committed write's records are held");
}
