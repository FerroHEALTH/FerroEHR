// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Party commits that contend on one PERSON (#6257). A party write judges
//! its pre-checks on one gate read, holds its access record first and commits
//! behind one guarded version statement, which takes the key-space gate
//! shared; so of several updates or deletes naming one preceding version one
//! commits and every other answers as a stale write does today, and a write
//! refused after its records were held discards them at once. No openEHR spec governs how a
//! commit is run against the database: our own design.

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
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::recorder::{WriteFacts, WriteRecorder, with_recorder};
use ferroehr::system_log::sender;
use sqlx::PgPool;
use uuid::Uuid;

use crate::fixtures::{delete_precondition, if_match, object};
use crate::party_binding_source::{LOGIN, PERSON_TEMPLATE, create, key, mapping, person, version};

/// The advisory lock a party commit's guarded statement takes shared
/// (`ferroehr::party_binding::key_space`).
const GATE: i64 = 0x5013_b1d1;

/// The binding key of the contended PERSON, which no update may change.
const KEY: &str = "4d5e6f7a-8b9c-4d0e-9f1a-3b4c5d6e7f8a";

/// A service over a fresh database with the binding mapping installed, so
/// each party commit takes the key-space gate, and its writes recorded when
/// `audited`.
async fn service(audited: bool) -> (testkit::TestDb, Arc<FerroEhrService>) {
    let db = testkit::db().await.expect("testkit database");
    crate::fixtures::open_key_spaces(&db, &[(LOGIN, false)]).await;
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

/// A transaction that holds the key-space gate exclusive, so every party
/// write judges its gate read and then waits at its guarded statement until
/// the transaction ends.
async fn hold(pool: &PgPool) -> sqlx::Transaction<'static, sqlx::Postgres> {
    let mut tx = pool.begin().await.expect("begin the hold");
    sqlx::query("SELECT pg_advisory_xact_lock($1)")
        .bind(GATE)
        .execute(&mut *tx)
        .await
        .expect("hold the key-space gate");
    tx
}

/// A recorder primed as a protocol adapter primes it: one operation record
/// of a party write.
pub(crate) fn primed() -> WriteRecorder {
    let recorder = WriteRecorder::default();
    recorder.prime(Arc::new(|facts: &WriteFacts| {
        let mut event = AuditEvent::new(
            EventActionCode::Update,
            ObjectClass::Demographic,
            EventOutcome::Success,
        );
        "clinician-7".clone_into(&mut event.user_id);
        event.object_id.clone_from(&facts.object_id);
        vec![event]
    }));
    recorder
}

/// The reservations any record is held under, in either log.
pub(crate) async fn held(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT reserved_by FROM audit.audit_event_hold \
         UNION SELECT reserved_by FROM audit.access_log_hold ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("the held records")
}

/// A PERSON carrying the login key `id`, and its version uid.
async fn a_person(svc: &FerroEhrService, id: &str) -> (VoId, String) {
    create(svc, PartyKind::Person, &person(PERSON_TEMPLATE, &[key(id)])).await
}

/// One write of `writers` against the PERSON `vo` at `uid`: an update naming
/// `uid`, or its delete when `delete`.
async fn write(
    svc: &FerroEhrService,
    (vo, uid): (VoId, &str),
    n: usize,
    delete: bool,
) -> Result<(), ServiceError> {
    if delete {
        return Box::pin(svc.party_delete(
            PartyKind::Person,
            object(uid),
            delete_precondition(uid, None),
            WriteRequest::new(()),
        ))
        .await
        .map(|_| ());
    }
    let mut body = person(PERSON_TEMPLATE, &[key(KEY)]);
    body["name"]["value"] = format!("writer {n}").into();
    Box::pin(svc.party_update(
        PartyKind::Person,
        object(vo.to_string()),
        if_match(uid),
        version(&body, "532", Some(uid)),
    ))
    .await
    .map(|_| ())
}

/// `writers` writes of one PERSON naming its one version, held at their
/// guarded statement, each with its recorder: each outcome and recorder.
async fn contend(
    db: &testkit::TestDb,
    svc: &Arc<FerroEhrService>,
    writers: usize,
    delete: bool,
) -> Vec<(Result<(), ServiceError>, WriteRecorder)> {
    let (vo, uid) = a_person(svc, KEY).await;
    let held = hold(&db.pool()).await;
    let tasks: Vec<_> = (0..writers)
        .map(|n| {
            let svc = Arc::clone(svc);
            let uid = uid.clone();
            let recorder = primed();
            let kept = recorder.clone();
            let task = tokio::spawn(ferroehr::db::acquire::request_scope(with_recorder(
                recorder,
                async move { write(&svc, (vo, &uid), n, delete).await },
            )));
            (task, kept)
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(400)).await;
    held.commit().await.expect("release the hold");
    let mut outcomes = Vec::new();
    for (task, recorder) in tasks {
        outcomes.push((task.await.expect("the writer completes"), recorder));
    }
    outcomes
}

/// `writers` updates, or deletes when `delete`, of one PERSON naming the same
/// preceding version: one commits; every other update answers `412` for the
/// moved tip, and every other delete answers `400` for the party it finds
/// already deleted, the check a delete makes before its precondition.
async fn one_party_write_wins(writers: usize, delete: bool) {
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
            Err(error) => panic!("a losing party write answers as a stale one: {error:?}"),
        }
    }
    assert_eq!(committed, 1, "exactly one party write wins");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_two_person_updates_wins() {
    one_party_write_wins(2, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_eight_person_updates_wins() {
    one_party_write_wins(8, false).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_two_person_deletes_wins() {
    one_party_write_wins(2, true).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn one_of_eight_person_deletes_wins() {
    one_party_write_wins(8, true).await;
}

/// Recorded updates of one PERSON naming its one version, held at their
/// guarded statement: the one that commits holds its record under the
/// reservation its statement committed; every other answers `412` after its
/// records were held, discards them at once and hands its operation record
/// back, so nothing of it is held for any read.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_party_write_refused_after_its_records_leaves_no_held_record() {
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
