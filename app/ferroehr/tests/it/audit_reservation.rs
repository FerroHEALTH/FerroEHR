// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The durable reservation of a write's audit records (#5762): the records
//! are held in the intake before the write commits and the reservation joins
//! the commit, so a process that stops between the commit and the release
//! leaves records the reconciler releases, exactly as written, once; records
//! whose write never committed are discarded.

use std::sync::Arc;
use std::time::Duration;

use ferroehr::access_log::chainer::Seal;
use ferroehr::db::domain::DomainPools;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::event::{
    AuditEvent, EventActionCode, EventOutcome, EventType, ObjectClass,
};
use ferroehr::system_log::recorder::{WriteFacts, WriteRecorder, with_recorder};
use ferroehr::system_log::sender::{self, AuditSender, reconciler};
use ferroehr::system_log::store::intake::Chains;
use sqlx::PgPool;
use uuid::Uuid;

/// A sender over the test database, its records verified at once.
async fn audited(pool: &PgPool) -> (AuditSender, FerroEhrService) {
    let config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            max_verify_lag: ferrohealth_config::units::Duration::from_secs(0),
        },
        ..AuditConfig::default()
    };
    let (sender, handle) = sender::start(config, None, pool.clone())
        .await
        .expect("the audit sender");
    drop(handle);
    let svc = FerroEhrService::new(&DomainPools::from_shared(pool)).with_audit(sender.clone());
    (sender, svc)
}

/// A recorder primed as a protocol adapter primes it: the operation record of
/// an EHR creation by `clinician-7`, which writes an access-log entry, and a
/// record of the system log beside it.
fn primed() -> WriteRecorder {
    let recorder = WriteRecorder::default();
    recorder.prime(Arc::new(|facts: &WriteFacts| {
        let mut event = AuditEvent::new(
            EventActionCode::Create,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        event.event_type = Some(EventType::rest("ehr_create"));
        "clinician-7".clone_into(&mut event.user_id);
        event.ehr_id.clone_from(&facts.ehr_id);
        event.object_id.clone_from(&facts.object_id);
        vec![event, system_record()]
    }));
    recorder
}

/// A record of the system log, which writes no access-log entry.
fn system_record() -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Authentication,
        EventOutcome::Success,
    );
    event.event_type = Some(EventType::Login);
    "clinician-7".clone_into(&mut event.user_id);
    event
}

/// The access-log entries and system-log records held under `reservation`:
/// their ids and their content digests.
async fn held(pool: &PgPool, reservation: Uuid) -> (Vec<(Uuid, Vec<u8>)>, Vec<Uuid>) {
    let entries = sqlx::query_as(
        "SELECT e.uid, e.content_digest FROM audit.access_log_intake i \
         CROSS JOIN LATERAL unnest(i.entries) AS n(id) JOIN audit.access_log_entry e ON e.id = n.id \
         WHERE i.reserved_by = $1 ORDER BY e.uid",
    )
    .bind(reservation)
    .fetch_all(pool)
    .await
    .expect("the held entries");
    let records = sqlx::query_scalar(
        "SELECT id FROM audit.audit_event_intake WHERE reserved_by = $1 ORDER BY id",
    )
    .bind(reservation)
    .fetch_all(pool)
    .await
    .expect("the held records");
    (entries, records)
}

/// The chained access-log entries among `uids`, with their content digests.
async fn chained(pool: &PgPool, uids: &[Uuid]) -> Vec<(Uuid, Vec<u8>)> {
    sqlx::query_as(
        "SELECT uid, content_digest FROM audit.access_log_record WHERE uid = ANY($1) ORDER BY uid",
    )
    .bind(uids)
    .fetch_all(pool)
    .await
    .expect("the chained entries")
}

/// How many system-log records among `ids` are chained.
async fn stored(pool: &PgPool, ids: &[Uuid]) -> i64 {
    sqlx::query_scalar("SELECT count(*) FROM audit.audit_event WHERE id = ANY($1)")
        .bind(ids)
        .fetch_one(pool)
        .await
        .expect("the chained records")
}

/// Whether the clinical domain still holds `reservation`.
async fn reserved(pool: &PgPool, reservation: Uuid) -> bool {
    sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM clinical.audit_reservation WHERE id = $1)")
        .bind(reservation)
        .fetch_one(pool)
        .await
        .expect("the reservation")
}

/// The domains the reconciler reads, all on the test's one database.
fn domains(pool: &PgPool) -> reconciler::Domains {
    reconciler::Domains {
        clinical: pool.clone(),
        party: pool.clone(),
    }
}

/// A write committed and its process stopped before the release, as a kill
/// between the domain's COMMIT and the release leaves it: the records wait
/// held, out of the chain, until the reconciler of any replica releases
/// them; then each entry is chained once with the content it was written
/// with, and a second reconciliation finds nothing.
#[tokio::test]
async fn a_write_whose_process_stopped_before_the_release_is_recorded_once() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (sender, svc) = audited(&pool).await;
    let recorder = primed();

    with_recorder(recorder.clone(), crate::fixtures::create_ehr(&svc)).await;
    // The process stops here: no `confirm_reserved`.
    let reservations = recorder.reserved();
    assert_eq!(reservations.len(), 1, "the write took one reservation");
    let reservation = reservations[0].id;
    assert!(
        reserved(&pool, reservation).await,
        "the reservation joined the commit"
    );
    let (entries, records) = held(&pool, reservation).await;
    assert_eq!(entries.len(), 1, "the EHR's access-log entry is held");
    assert_eq!(records.len(), 1, "the system-log record is held");

    sender
        .store()
        .settle(Chains::BOTH)
        .await
        .expect("a settled chain");
    let uids: Vec<Uuid> = entries.iter().map(|(uid, _)| *uid).collect();
    assert!(
        chained(&pool, &uids).await.is_empty(),
        "a held entry stays out of the chain"
    );

    let first = reconciler::reconcile(sender.store(), &domains(&pool), Duration::ZERO)
        .await
        .expect("a reconciliation");
    assert_eq!(first.released, 1, "{first:?}");
    assert_eq!(first.discarded, 0, "{first:?}");
    sender
        .store()
        .settle(Chains::BOTH)
        .await
        .expect("a settled chain");
    assert_eq!(
        chained(&pool, &uids).await,
        entries,
        "each held entry is chained once, with the content it was written with"
    );
    assert_eq!(
        stored(&pool, &records).await,
        1,
        "the system-log record is stored once"
    );

    let second = reconciler::reconcile(sender.store(), &domains(&pool), Duration::ZERO)
        .await
        .expect("a second reconciliation");
    assert_eq!(second.released, 0, "{second:?}");
    assert_eq!(second.discarded, 0, "{second:?}");
    assert!(
        !reserved(&pool, reservation).await,
        "the spent reservation is removed"
    );
    sender
        .store()
        .settle(Chains::BOTH)
        .await
        .expect("a settled chain");
    assert_eq!(chained(&pool, &uids).await.len(), 1, "never twice");
    assert_eq!(stored(&pool, &records).await, 1, "never twice");
}

/// A write that committed and was handed to the chainer: the chainer's next
/// round releases its records inside its own transaction and chains them,
/// and the write's spent reservation is the reconciler's to remove, so the
/// write itself costs no third commit.
#[tokio::test]
async fn a_confirmed_write_is_released_by_the_chainer_round() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (sender, svc) = audited(&pool).await;
    let recorder = primed();

    with_recorder(recorder.clone(), crate::fixtures::create_ehr(&svc)).await;
    let reservations = recorder.reserved();
    let reservation = reservations[0].id;
    let (entries, records) = held(&pool, reservation).await;
    assert_eq!((entries.len(), records.len()), (1, 1));
    svc.confirm_reserved(&reservations);
    for _ in 0..200 {
        if held(&pool, reservation).await == (Vec::new(), Vec::new()) {
            break;
        }
        sender
            .store()
            .chain_round(512, Chains::default(), Seal::Now)
            .await
            .expect("a chainer round");
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        held(&pool, reservation).await,
        (Vec::new(), Vec::new()),
        "a chainer round released the records"
    );
    sender
        .store()
        .settle(Chains::BOTH)
        .await
        .expect("a settled chain");
    let uids: Vec<Uuid> = entries.iter().map(|(uid, _)| *uid).collect();
    assert_eq!(chained(&pool, &uids).await, entries, "chained as written");
    assert_eq!(stored(&pool, &records).await, 1, "stored once");
    assert!(
        reserved(&pool, reservation).await,
        "the write's commit leaves the spent reservation to the reconciler"
    );

    let reconciled = reconciler::reconcile(sender.store(), &domains(&pool), Duration::ZERO)
        .await
        .expect("a reconciliation");
    assert_eq!(
        reconciled,
        reconciler::Reconciled {
            removed: 1,
            ..reconciler::Reconciled::default()
        }
    );
    assert!(
        !reserved(&pool, reservation).await,
        "the spent reservation is removed"
    );
}

/// A write handed to the chainer whose process stopped before the chainer's
/// round: the queue died with it, the records wait held, and the reconciler
/// of another replica releases them, exactly as written, once.
#[tokio::test]
async fn a_confirmed_write_whose_process_stopped_before_the_round_is_recorded_once() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let config = AuditConfig {
        enabled: true,
        ..AuditConfig::default()
    };
    let (stopping, handle) = sender::start(config, None, pool.clone())
        .await
        .expect("the audit sender of the replica that stops");
    let svc = FerroEhrService::new(&DomainPools::from_shared(&pool)).with_audit(stopping.clone());
    let recorder = primed();

    with_recorder(recorder.clone(), crate::fixtures::create_ehr(&svc)).await;
    handle.shutdown(Duration::from_secs(5)).await;
    let reservations = recorder.reserved();
    svc.confirm_reserved(&reservations);
    let reservation = reservations[0].id;
    let (entries, records) = held(&pool, reservation).await;
    assert_eq!(
        (entries.len(), records.len()),
        (1, 1),
        "no round ran, so the records wait held"
    );

    let (other, _svc) = audited(&pool).await;
    let first = reconciler::reconcile(other.store(), &domains(&pool), Duration::ZERO)
        .await
        .expect("a reconciliation");
    assert_eq!(first.released, 1, "{first:?}");
    other
        .store()
        .settle(Chains::BOTH)
        .await
        .expect("a settled chain");
    let uids: Vec<Uuid> = entries.iter().map(|(uid, _)| *uid).collect();
    assert_eq!(chained(&pool, &uids).await, entries, "chained as written");
    assert_eq!(stored(&pool, &records).await, 1, "stored once");

    let second = reconciler::reconcile(other.store(), &domains(&pool), Duration::ZERO)
        .await
        .expect("a second reconciliation");
    assert_eq!(second.released, 0, "{second:?}");
    assert!(
        !reserved(&pool, reservation).await,
        "the spent reservation is removed"
    );
}

/// Records held under a reservation no domain holds belong to a write that
/// never committed: the reconciler discards them, and no access is logged.
#[tokio::test]
async fn records_of_a_write_that_never_committed_are_discarded() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (sender, _svc) = audited(&pool).await;
    let reservation = Uuid::now_v7();
    let mut event = AuditEvent::new(
        EventActionCode::Create,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    event.ehr_id = Some(Uuid::now_v7().to_string());
    "clinician-7".clone_into(&mut event.user_id);
    sender
        .record(vec![event, system_record()], Some(reservation))
        .await
        .expect("the held write");
    let (entries, records) = held(&pool, reservation).await;
    assert_eq!((entries.len(), records.len()), (1, 1));

    let reconciled = reconciler::reconcile(sender.store(), &domains(&pool), Duration::ZERO)
        .await
        .expect("a reconciliation");
    assert_eq!(reconciled.discarded, 1, "{reconciled:?}");
    assert_eq!(held(&pool, reservation).await, (Vec::new(), Vec::new()));
    sender
        .store()
        .settle(Chains::BOTH)
        .await
        .expect("a settled chain");
    let uids: Vec<Uuid> = entries.iter().map(|(uid, _)| *uid).collect();
    assert!(
        chained(&pool, &uids).await.is_empty(),
        "no access is logged"
    );
    assert_eq!(stored(&pool, &records).await, 0, "no record is stored");
}

/// A held write under a render policy the store has not registered yet, on an
/// audit pool of one connection, registers the policy before it takes the
/// connection it writes on: holding that connection, it asks the pool for no
/// second one, which waited out the acquire timeout and stalled every write
/// of a burst behind it (#5906).
#[tokio::test]
async fn a_held_write_under_a_cold_policy_needs_one_audit_connection() {
    let db = testkit::db().await.expect("testkit database");
    let mut config = ferroehr::db::DbConfig::new(db.url());
    config.max_connections = Some(1);
    config.min_connections = Some(0);
    config.acquire_timeout = ferrohealth_config::units::Duration::from_secs(3);
    let audit = ferroehr::db::connect_domain(&config, ferroehr::db::domain::Domain::Audit)
        .await
        .expect("the audit pool connects");
    let store = ferroehr::system_log::store::AuditStore::new(audit);
    let reservation = Uuid::now_v7();
    let mut event = AuditEvent::new(
        EventActionCode::Create,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    event.ehr_id = Some(Uuid::now_v7().to_string());
    "clinician-7".clone_into(&mut event.user_id);
    let records = [event, system_record()].map(|event| ferroehr::system_log::store::AuditRecord {
        event,
        subject: None,
        entry: None,
    });

    // NOTE: (#5906) a second acquire is refused in a debug build, or times the write out.
    ferroehr::db::acquire::request_scope(store.write_held(records.into(), &[], reservation))
        .await
        .expect("the held write");
    let (entries, records) = held(&db.pool(), reservation).await;
    assert_eq!((entries.len(), records.len()), (1, 1));
}

/// A request's first access record under a render policy the store has not
/// registered yet, on an audit pool of one connection: the registration
/// checks out the connection and the request keeps it, then the entry waits
/// for the gathered writer, which needs a connection of the same pool. The
/// request gives back what it keeps before it waits, so the write lands;
/// keeping it, every such request of a cold burst held the pool until the
/// acquire timeout and its access, and every ITI-81 search, was refused
/// (#6008).
#[tokio::test]
async fn a_cold_entry_gives_back_its_audit_connection_before_the_gathered_write() {
    let db = testkit::db().await.expect("testkit database");
    let mut config = ferroehr::db::DbConfig::new(db.url());
    config.max_connections = Some(1);
    config.min_connections = Some(0);
    config.acquire_timeout = ferrohealth_config::units::Duration::from_secs(3);
    let audit = ferroehr::db::connect_domain(&config, ferroehr::db::domain::Domain::Audit)
        .await
        .expect("the audit pool connects");
    let store = ferroehr::system_log::store::AuditStore::new(audit);
    let mut event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    event.ehr_id = Some(Uuid::now_v7().to_string());
    "clinician-7".clone_into(&mut event.user_id);
    let records = vec![ferroehr::system_log::store::AuditRecord {
        event,
        subject: None,
        entry: None,
    }];

    ferroehr::db::acquire::request_scope(store.write(records, &[]))
        .await
        .expect("the entry lands while the request waits for the gathered writer");
    let landed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM audit.access_log_intake i \
         CROSS JOIN LATERAL unnest(i.entries) AS n(id)",
    )
    .fetch_one(&db.pool())
    .await
    .expect("the intake");
    assert_eq!(landed, 1, "the one entry is in the intake");
}

/// An audited write with the subject lookup on: the subject its records name
/// is read on the write's own transaction, so the record of an EHR creation
/// names the subject the write gave it. A lookup on a connection of its own
/// waited for a second clinical connection while the write held one, and
/// could not see the EHR the write had not committed yet (#5947).
#[tokio::test]
async fn a_write_reads_the_subject_of_its_records_on_its_own_transaction() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let mut config = ferroehr::db::DbConfig::new(db.url());
    config.max_connections = Some(2);
    config.min_connections = Some(0);
    config.acquire_timeout = ferrohealth_config::units::Duration::from_secs(3);
    let clinical = ferroehr::db::connect_domain(&config, ferroehr::db::domain::Domain::Clinical)
        .await
        .expect("the clinical pool connects");
    let lookup_pool = clinical.clone();
    let resolver: sender::SubjectResolver = Arc::new(move |ehr_id: String| {
        let pool = lookup_pool.clone();
        Box::pin(async move {
            let Ok(id) = Uuid::parse_str(&ehr_id) else {
                return sender::SubjectLookup::NoSubject;
            };
            match ferroehr::storage::ehr_repo::ehr_subject_columns(&pool, ferroehr::ids::EhrId(id))
                .await
            {
                Ok(_) => sender::SubjectLookup::NoSubject,
                Err(_) => sender::SubjectLookup::Unavailable,
            }
        })
    });
    let audit = AuditConfig {
        enabled: true,
        ..AuditConfig::default()
    };
    let (sender, handle) = sender::start(audit, Some(resolver), pool.clone())
        .await
        .expect("the audit sender");
    drop(handle);
    let svc = FerroEhrService::new(&DomainPools {
        clinical,
        party: pool.clone(),
        audit: pool.clone(),
    })
    .with_audit(sender);
    let recorder = primed();

    let subject = ferroehr::service::ehr_index::types::SubjectRef {
        id: "subject-5947".to_owned(),
        namespace: "test.ferroehr".to_owned(),
        r#type: "PERSON".to_owned(),
    };

    ferroehr::db::acquire::request_scope(with_recorder(
        recorder.clone(),
        crate::fixtures::create_ehr_subject(&svc, &subject),
    ))
    .await;
    let reservations = recorder.reserved();
    assert_eq!(reservations.len(), 1, "the write took one reservation");
    let named: Vec<Option<String>> = sqlx::query_scalar(
        "SELECT e.patient_id FROM audit.access_log_intake i \
             CROSS JOIN LATERAL unnest(i.entries) AS n(id) \
             JOIN audit.access_log_entry e ON e.id = n.id WHERE i.reserved_by = $1",
    )
    .bind(reservations[0].id)
    .fetch_all(&pool)
    .await
    .expect("the held entries");
    assert_eq!(
        named,
        vec![Some(subject.id.clone())],
        "the entry names the new EHR's subject"
    );
}

/// A reconciliation on an audit pool of one connection resolves what it
/// finds: the pass holds one connection at a time, so it never waits for a
/// second one of a pool while it keeps the first, which waited out the
/// acquire timeout before (#5929).
#[tokio::test]
async fn a_reconciliation_needs_one_audit_connection() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (sender, _svc) = audited(&pool).await;
    let reservation = Uuid::now_v7();
    sender
        .record(vec![system_record()], Some(reservation))
        .await
        .expect("the held write");
    let mut config = ferroehr::db::DbConfig::new(db.url());
    config.max_connections = Some(1);
    config.min_connections = Some(0);
    config.acquire_timeout = ferrohealth_config::units::Duration::from_secs(3);
    let audit = ferroehr::db::connect_domain(&config, ferroehr::db::domain::Domain::Audit)
        .await
        .expect("the audit pool connects");
    let store = ferroehr::system_log::store::AuditStore::new(audit);

    let reconciled = reconciler::reconcile(&store, &domains(&pool), Duration::ZERO)
        .await
        .expect("a reconciliation");
    assert_eq!(reconciled.discarded, 1, "{reconciled:?}");
    assert_eq!(held(&pool, reservation).await, (Vec::new(), Vec::new()));
}

/// A write takes its audit connection before it opens its transaction: with
/// the one audit connection held elsewhere, the write waits holding no
/// clinical connection, so the one clinical connection still serves a read,
/// and the write commits with its records held once the audit connection is
/// back. Taken the other way round, the write sat idle in its transaction
/// while it waited for the audit pool, and the read waited for it (#5964).
#[tokio::test]
async fn a_write_waits_for_its_audit_connection_before_its_transaction() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let mut config = ferroehr::db::DbConfig::new(db.url());
    config.max_connections = Some(1);
    config.min_connections = Some(0);
    config.acquire_timeout = ferrohealth_config::units::Duration::from_secs(3);
    let clinical = ferroehr::db::connect_domain(&config, ferroehr::db::domain::Domain::Clinical)
        .await
        .expect("the clinical pool connects");
    let audit_pool = ferroehr::db::connect_domain(&config, ferroehr::db::domain::Domain::Audit)
        .await
        .expect("the audit pool connects");
    let audit = AuditConfig {
        enabled: true,
        ..AuditConfig::default()
    };
    let (sender, handle) = sender::start(audit, None, audit_pool.clone())
        .await
        .expect("the audit sender");
    drop(handle);
    let svc = FerroEhrService::new(&DomainPools {
        clinical: clinical.clone(),
        party: pool.clone(),
        audit: audit_pool.clone(),
    })
    .with_audit(sender);
    let recorder = primed();
    let busy = ferroehr::db::acquire::acquire(&audit_pool)
        .await
        .expect("the one audit connection");

    let write = tokio::spawn({
        let recorder = recorder.clone();
        async move {
            ferroehr::db::acquire::request_scope(with_recorder(
                recorder,
                crate::fixtures::create_ehr_for_party(&svc, ferroehr::ids::VoId(Uuid::now_v7())),
            ))
            .await
        }
    });
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(!write.is_finished(), "the write waits for the audit pool");
    let one: i32 = sqlx::query_scalar("SELECT 1")
        .fetch_one(
            &mut *ferroehr::db::acquire::acquire(&clinical)
                .await
                .expect("the clinical connection is free while the write waits"),
        )
        .await
        .expect("a read on it");
    assert_eq!(one, 1);
    drop(busy);
    write.await.expect("the write task");
    let reservations = recorder.reserved();
    assert_eq!(reservations.len(), 1, "the write took one reservation");
    let (entries, records) = held(&pool, reservations[0].id).await;
    assert_eq!((entries.len(), records.len()), (1, 1));
}

/// The pools are taken audit first: a request that asks the audit pool for
/// a connection while it holds a clinical one is refused in a debug build,
/// whichever path it takes (#5964).
#[cfg(debug_assertions)]
#[tokio::test]
#[should_panic(expected = "#5964")]
async fn an_audit_connection_after_a_clinical_one_is_refused() {
    let db = testkit::db().await.expect("testkit database");
    let pools = DomainPools::from_shared(&db.pool());
    ferroehr::db::acquire::request_scope(async {
        let _clinical = ferroehr::db::acquire::acquire(&pools.clinical)
            .await
            .expect("a clinical connection");
        drop(ferroehr::db::acquire::acquire(&pools.audit).await);
    })
    .await;
}
