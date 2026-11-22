// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A COMPOSITION write's audit records are held before its one statement
//! (#6256): the write writes its operation record into the audit intake
//! under a reservation, durable when the intake returns, and only then runs
//! the guarded statement that inserts the reservation beside its rows, so
//! the records are durable before anything of the write is visible (#5747,
//! #5986) and the reservation commits with the write or not at all (#5762).
//! A write the statement refuses, or that is judged again, discards its
//! records at once; the reconciler is the fallback. No openEHR spec governs
//! the audit storage: our own design.

#![expect(
    clippy::disallowed_methods,
    reason = "a test holds and reads rows on raw pool connections, outside any request"
)]

use std::sync::Arc;
use std::time::Duration;

use ferroehr::db::domain::DomainPools;
use ferroehr::error::ServiceError;
use ferroehr::ids::EhrId;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::AuditConfig;
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass};
use ferroehr::system_log::recorder::{WriteFacts, WriteRecorder, with_recorder};
use ferroehr::system_log::sender;
use ferroehr::versioning::change::Committed;
use sqlx::PgPool;
use uuid::Uuid;

use crate::fixtures::{composition, uv};

/// A service over the test database whose writes are recorded.
async fn audited(pool: &PgPool) -> Arc<FerroEhrService> {
    let config = AuditConfig {
        enabled: true,
        ..AuditConfig::default()
    };
    let (sender, handle) = sender::start(config, None, pool.clone())
        .await
        .expect("the audit sender");
    drop(handle);
    Arc::new(FerroEhrService::new(&DomainPools::from_shared(pool)).with_audit(sender))
}

/// A recorder primed as a protocol adapter primes it: one operation record of
/// a COMPOSITION update, which writes an access-log entry.
fn primed() -> WriteRecorder {
    let recorder = WriteRecorder::default();
    recorder.prime(Arc::new(|facts: &WriteFacts| {
        let mut event = AuditEvent::new(
            EventActionCode::Update,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        event.event_type = Some(EventType::rest("composition_update"));
        "clinician-7".clone_into(&mut event.user_id);
        event.ehr_id.clone_from(&facts.ehr_id);
        event.object_id.clone_from(&facts.object_id);
        vec![event]
    }));
    recorder
}

/// The access-log entries held under any reservation.
async fn held(pool: &PgPool) -> Vec<Uuid> {
    sqlx::query_scalar(
        "SELECT i.reserved_by FROM audit.access_log_intake i \
         CROSS JOIN LATERAL unnest(i.entries) AS n(id) \
         WHERE i.reserved_by IS NOT NULL ORDER BY 1",
    )
    .fetch_all(pool)
    .await
    .expect("the held entries")
}

/// Whether the clinical domain holds `reservation`.
async fn reserved(pool: &PgPool, reservation: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM clinical.audit_reservation WHERE id = $1)")
        .bind(reservation)
        .fetch_one(pool)
        .await
        .expect("the reservation")
}

/// An EHR with one COMPOSITION, committed unrecorded.
async fn object(svc: &FerroEhrService) -> (EhrId, Committed) {
    let ehr = crate::fixtures::create_ehr(svc).await;
    let created = Box::pin(svc.create_composition(ehr, uv(&composition("v1"), "249", None)))
        .await
        .expect("the first version");
    (ehr, created)
}

/// `writers` recorded updates of `created`, each naming `preceding` (or none),
/// held at their statement by a transaction holding the EHR row; each
/// writer's outcome with its recorder.
async fn updates(
    pool: &PgPool,
    svc: &Arc<FerroEhrService>,
    (ehr, created): (EhrId, &Committed),
    preceding: Option<&str>,
    writers: usize,
) -> Vec<(Result<Committed, ServiceError>, WriteRecorder)> {
    let mut hold = pool.begin().await.expect("begin the hold");
    sqlx::query("SELECT 1 FROM ehr WHERE id = $1 FOR UPDATE")
        .bind(ehr)
        .execute(&mut *hold)
        .await
        .expect("hold the EHR row");
    let tasks: Vec<_> = (0..writers)
        .map(|n| {
            let svc = Arc::clone(svc);
            let recorder = primed();
            let request = uv(&composition(&format!("writer {n}")), "251", preceding);
            let vo = created.vo_id;
            let kept = recorder.clone();
            let task = tokio::spawn(ferroehr::db::acquire::request_scope(with_recorder(
                recorder,
                async move { Box::pin(svc.update_composition(ehr, vo, request)).await },
            )));
            (task, kept)
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(400)).await;
    hold.commit().await.expect("release the hold");
    let mut outcomes = Vec::new();
    for (task, recorder) in tasks {
        outcomes.push((task.await.expect("the writer completes"), recorder));
    }
    outcomes
}

/// A recorded update holds its record under a reservation that its own
/// statement committed beside the version.
#[tokio::test]
async fn a_recorded_update_holds_its_record_under_the_reservation_its_statement_commits() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = audited(&pool).await;
    let (ehr, created) = object(&svc).await;
    let outcomes = updates(&pool, &svc, (ehr, &created), None, 1).await;
    let (outcome, recorder) = outcomes.into_iter().next().expect("one writer");
    outcome.expect("the update commits");
    let reservations = recorder.reserved();
    assert_eq!(reservations.len(), 1, "the write took one reservation");
    assert!(
        reserved(&pool, reservations[0].id).await,
        "the statement committed the reservation beside the version"
    );
    assert_eq!(
        held(&pool).await,
        vec![reservations[0].id],
        "its one entry is held under it"
    );
}

/// Conditioned updates that name one version, held at their statement: the
/// one that commits holds its record; every other answers `412` after its
/// intake, its record discarded at once, its recorder handed back the
/// operation record, so nothing of it is held for any read.
#[tokio::test]
async fn a_write_refused_after_its_intake_leaves_no_held_record() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = audited(&pool).await;
    let (ehr, created) = object(&svc).await;
    let preceding = created.version_uid();
    let outcomes = updates(&pool, &svc, (ehr, &created), Some(&preceding), 4).await;
    let mut kept = Vec::new();
    for (outcome, recorder) in outcomes {
        match outcome {
            Ok(_) => kept.extend(recorder.reserved().into_iter().map(|r| r.id)),
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
    assert_eq!(
        held(&pool).await,
        kept,
        "only the committed write's record is held"
    );
}

/// Unconditioned updates that all judged version 1: each commits, those that
/// were judged again after another commit discarded the records of their
/// earlier attempts, and each write holds exactly one record, its final one.
#[tokio::test]
async fn a_write_judged_again_holds_one_record() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = audited(&pool).await;
    let (ehr, created) = object(&svc).await;
    let outcomes = updates(&pool, &svc, (ehr, &created), None, 4).await;
    let mut kept = Vec::new();
    for (outcome, recorder) in outcomes {
        outcome.expect("every unconditioned update commits");
        let reservations = recorder.reserved();
        assert_eq!(reservations.len(), 1, "each write keeps one reservation");
        assert!(reserved(&pool, reservations[0].id).await, "and committed it");
        kept.push(reservations[0].id);
    }
    kept.sort_unstable();
    assert_eq!(
        held(&pool).await,
        kept,
        "each write holds its one final record, and no attempt's record is left"
    );
}
