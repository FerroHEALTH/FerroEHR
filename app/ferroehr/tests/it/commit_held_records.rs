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
use ferroehr::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
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
    sqlx::query_scalar("SELECT h.reserved_by FROM audit.access_log_hold h ORDER BY 1")
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
        assert!(
            reserved(&pool, reservations[0].id).await,
            "and committed it"
        );
        kept.push(reservations[0].id);
    }
    kept.sort_unstable();
    assert_eq!(
        held(&pool).await,
        kept,
        "each write holds its one final record, and no attempt's record is left"
    );
}

/// The audit store over its own pool of `db`, and an access record of an EHR
/// read, which writes an access-log entry.
async fn store_and_entry(
    db: &testkit::TestDb,
) -> (
    ferroehr::system_log::store::AuditStore,
    PgPool,
    ferroehr::system_log::store::AuditRecord,
) {
    let config = ferroehr::db::DbConfig::new(db.url());
    let audit = ferroehr::db::connect_domain(&config, ferroehr::db::domain::Domain::Audit)
        .await
        .expect("the audit pool connects");
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    event.ehr_id = Some(Uuid::now_v7().to_string());
    "clinician-7".clone_into(&mut event.user_id);
    let record = ferroehr::system_log::store::AuditRecord {
        event,
        subject: None,
        entry: None,
    };
    (
        ferroehr::system_log::store::AuditStore::new(audit.clone()),
        audit,
        record,
    )
}

/// A held write of access-log entries alone is one statement on the caller's
/// connection: it leaves no transaction open there, and its entry is
/// committed, so another connection sees it held, when the write returns.
#[tokio::test]
async fn a_held_write_of_entries_alone_commits_on_its_own() {
    let db = testkit::db().await.expect("testkit database");
    let (store, audit, record) = store_and_entry(&db).await;
    let reservation = Uuid::now_v7();
    let mut conn = audit.acquire().await.expect("an audit connection");
    let pid: i32 = sqlx::query_scalar("SELECT pg_backend_pid()")
        .fetch_one(&mut *conn)
        .await
        .expect("the backend");
    Box::pin(store.write_held_on(&mut conn, vec![record], &[], reservation))
        .await
        .expect("the held write");
    let state: String = sqlx::query_scalar("SELECT state FROM pg_stat_activity WHERE pid = $1")
        .bind(pid)
        .fetch_one(&db.pool())
        .await
        .expect("the writer's state");
    assert_eq!(state, "idle", "the write left no transaction open");
    let held: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit.access_log_hold WHERE reserved_by = $1")
            .bind(reservation)
            .fetch_one(&db.pool())
            .await
            .expect("the hold");
    assert_eq!(
        held, 1,
        "the entry is durable and held when the write returns"
    );
}

/// A held write of an entry beside a system-log record takes a transaction:
/// when its entry fails after the record went in, nothing of it is left.
#[tokio::test]
async fn a_held_write_of_more_than_entries_leaves_nothing_when_a_later_statement_fails() {
    let db = testkit::db().await.expect("testkit database");
    let (store, audit, record) = store_and_entry(&db).await;
    sqlx::raw_sql(
        "CREATE FUNCTION audit.refuse_hold() RETURNS trigger LANGUAGE plpgsql AS \
         $$ BEGIN RAISE EXCEPTION 'refused by the test'; END $$; \
         CREATE TRIGGER refuse_hold BEFORE INSERT ON audit.access_log_hold \
         FOR EACH ROW EXECUTE FUNCTION audit.refuse_hold()",
    )
    .execute(&db.pool())
    .await
    .expect("a trigger that refuses every hold");
    let mut login = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Authentication,
        EventOutcome::Success,
    );
    login.event_type = Some(EventType::Login);
    "clinician-7".clone_into(&mut login.user_id);
    let system = ferroehr::system_log::store::AuditRecord {
        event: login,
        subject: None,
        entry: None,
    };
    let reservation = Uuid::now_v7();
    let mut conn = audit.acquire().await.expect("an audit connection");
    Box::pin(store.write_held_on(&mut conn, vec![system, record], &[], reservation))
        .await
        .expect_err("the entry's hold is refused");
    let left: i64 = sqlx::query_scalar(
        "SELECT (SELECT count(*) FROM audit.audit_event_hold WHERE reserved_by = $1) \
              + (SELECT count(*) FROM audit.access_log_hold WHERE reserved_by = $1)",
    )
    .bind(reservation)
    .fetch_one(&db.pool())
    .await
    .expect("what is left");
    assert_eq!(
        left, 0,
        "the system record went back with the refused entry"
    );
}

/// One recorded update of `created` with content `n`, as a protocol adapter
/// sends it; the committed version and the write's recorder.
async fn recorded_update(
    svc: &Arc<FerroEhrService>,
    (ehr, created): (EhrId, &Committed),
    n: usize,
) -> (Committed, WriteRecorder) {
    let recorder = primed();
    let request = uv(&composition(&format!("recorded {n}")), "251", None);
    let vo = created.vo_id;
    let svc = Arc::clone(svc);
    let committed = Box::pin(ferroehr::db::acquire::request_scope(with_recorder(
        recorder.clone(),
        async move { Box::pin(svc.update_composition(ehr, vo, request)).await },
    )))
    .await
    .expect("the recorded update commits");
    (committed, recorder)
}

/// The access-log entries written, held or not.
async fn entries(pool: &PgPool) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit.access_log_entry")
        .fetch_one(pool)
        .await
        .expect("the entries")
}

/// Asserts that the write `recorder` recorded holds exactly one entry, under
/// the reservation its statement committed beside the version, and that
/// every entry's shared part is held.
async fn holds_one_entry(pool: &PgPool, recorder: &WriteRecorder) {
    let reservations = recorder.reserved();
    assert_eq!(reservations.len(), 1, "the write took one reservation");
    assert!(
        reserved(pool, reservations[0].id).await,
        "the statement committed the reservation beside the version"
    );
    let under: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit.access_log_hold WHERE reserved_by = $1")
            .bind(reservations[0].id)
            .fetch_one(pool)
            .await
            .expect("the held entries");
    assert_eq!(under, 1, "exactly one entry is held under it");
    let dangling: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit.access_log_entry e WHERE e.shared IS NOT NULL \
         AND NOT EXISTS (SELECT 1 FROM audit.access_log_shared s WHERE s.digest = e.shared)",
    )
    .fetch_one(pool)
    .await
    .expect("the dangling entries");
    assert_eq!(dangling, 0, "every entry's shared part is held");
}

/// A held write skips the shared part an earlier write stored (#6244); when
/// the store no longer holds it, the intake refuses the entry (`23503`), and
/// the write sends it again: exactly one entry is recorded, durable before
/// the statement that commits the version, which still commits.
#[tokio::test]
async fn a_held_write_sends_again_a_shared_part_the_store_lost() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = audited(&pool).await;
    let (ehr, created) = object(&svc).await;
    let (first, recorder) = Box::pin(recorded_update(&svc, (ehr, &created), 1)).await;
    holds_one_entry(&pool, &recorder).await;
    let removed = sqlx::query("DELETE FROM audit.access_log_shared")
        .execute(&pool)
        .await
        .expect("the shared parts go")
        .rows_affected();
    assert!(removed > 0, "the first write stored a shared part");
    let before = entries(&pool).await;
    let (second, recorder) = Box::pin(recorded_update(&svc, (ehr, &first), 2)).await;
    assert_eq!(entries(&pool).await, before + 1, "one entry is recorded");
    holds_one_entry(&pool, &recorder).await;
    assert!(
        second.version_uid() != first.version_uid(),
        "the version committed after its record"
    );
}

/// A held write names the search tokens an earlier write resolved by id
/// under their generation (#6244); when the store's generation moved on, the
/// intake refuses the ids (`FE001`), and the write resolves its tokens again:
/// exactly one entry is recorded, and the version still commits after it.
#[tokio::test]
async fn a_held_write_resolves_again_search_tokens_of_a_stale_generation() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let svc = audited(&pool).await;
    let (ehr, created) = object(&svc).await;
    let (first, recorder) = Box::pin(recorded_update(&svc, (ehr, &created), 1)).await;
    holds_one_entry(&pool, &recorder).await;
    let mut tx = pool.begin().await.expect("begin");
    for statement in [
        "ALTER TABLE audit.audit_search_token_generation DISABLE TRIGGER USER",
        "UPDATE audit.audit_search_token_generation SET generation = generation + 1",
        "ALTER TABLE audit.audit_search_token_generation ENABLE TRIGGER USER",
    ] {
        sqlx::query(statement)
            .execute(&mut *tx)
            .await
            .expect("the generation moves on");
    }
    tx.commit().await.expect("commit");
    let before = entries(&pool).await;
    let (second, recorder) = Box::pin(recorded_update(&svc, (ehr, &first), 2)).await;
    assert_eq!(entries(&pool).await, before + 1, "one entry is recorded");
    holds_one_entry(&pool, &recorder).await;
    let tokens: i32 = sqlx::query_scalar(
        "SELECT cardinality(e.search_tokens) FROM audit.access_log_entry e \
         ORDER BY e.id DESC LIMIT 1",
    )
    .fetch_one(&pool)
    .await
    .expect("the newest entry's tokens");
    assert!(tokens > 0, "the newest entry names its search tokens");
    assert!(
        second.version_uid() != first.version_uid(),
        "the version committed after its record"
    );
}
