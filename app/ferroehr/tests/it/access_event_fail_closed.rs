// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[audit] fail_mode = "closed"` reaches the DOMAIN-level access records
//! (#3235): the subject-to-EHR lookup and the EHR-Extract export each emit
//! their own record, and each must withhold its result when the sender rejects
//! it.
//!
//! The rejecting sender is a real one whose store is unreachable: the drain's
//! first write fails, the store is marked unhealthy, and from then on a
//! fail-closed sender answers `Rejected` (`fail_closed_rejects_while_store_unhealthy`
//! in the sender's own tests pins that transition). No test hook in production
//! code; the posture is the one an operator meets when the audit database is
//! down.

use std::time::{Duration, Instant};

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use ferroehr::access_log::store::AccessLogStore;
use ferroehr::error::ServiceError;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig};
use ferroehr::system_log::event::{
    AuditEvent, EmitOutcome, EventActionCode, EventOutcome, ObjectClass,
};
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::admin_fixture::seed_full_ehr;

/// A fail-closed sender whose store cannot be reached, driven to the rejecting
/// state: one probe record is enqueued, the drain fails to write it, and the
/// sender rejects everything after.
async fn rejecting_sender() -> (ferroehr::system_log::sender::AuditSender, AuditHandle) {
    let unreachable: PgPool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(200))
        .connect_lazy("postgres://nobody:nothing@127.0.0.1:9/nowhere")
        .expect("a lazy pool builds without connecting");
    let config = AuditConfig {
        enabled: true,
        fail_mode: FailMode::Closed,
        store: StoreConfig {
            enabled: true,
            retention: None,
            sgb_v_309_controller: false,
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
        },
        ..AuditConfig::default()
    };
    let (sender, handle): (_, AuditHandle) = start(config, None, Some(unreachable))
        .await
        .expect("the audit sender");

    let probe = || {
        AuditEvent::new(
            EventActionCode::Execute,
            ObjectClass::Ehr,
            EventOutcome::Success,
        )
    };
    assert_eq!(
        sender.emit(probe()),
        EmitOutcome::Enqueued,
        "the store is healthy until a write fails"
    );
    let deadline = Instant::now() + Duration::from_secs(30);
    while sender.emit(probe()) != EmitOutcome::Rejected {
        assert!(
            Instant::now() < deadline,
            "the drain never marked the store unhealthy"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    (sender, handle)
}

/// The subject-to-EHR lookup is withheld when its record is rejected, and the
/// status route reports the `service_overloaded` status the wire maps to `503`.
#[tokio::test]
async fn a_subject_lookup_is_withheld_when_its_record_is_rejected() {
    let db = testkit::db().await.expect("testkit database");
    let (sender, _handle) = rejecting_sender().await;
    let closed = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_audit(sender);

    let refused = closed
        .ehr_read_for_subject(&Uuid::now_v7().to_string(), "urn:example:pseudonym")
        .await;
    let error = refused.expect_err("a miss is still a lookup, and its record was rejected");
    assert!(matches!(&error, ServiceError::Unrecorded(_)), "{error}");
}

/// An EHR-Extract export is withheld when its non-repudiation record is
/// rejected: the extract was assembled, and nothing is returned.
#[tokio::test]
async fn an_extract_export_is_withheld_when_its_record_is_rejected() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let seeded = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let ehr = seed_full_ehr(&seeded).await;

    let (sender, _handle) = rejecting_sender().await;
    let closed = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender);

    let refused = closed.extract_ehrs(ehr).await;
    let error = refused.expect_err("the export ran, and its record was rejected");
    assert!(matches!(&error, ServiceError::Unrecorded(_)), "{error}");
}

/// One stored access-log entry, from the shared example fixture.
async fn stored_entry(pool: &PgPool) -> String {
    let mut value = crate::access_log_store::example();
    let uid = Uuid::now_v7().to_string();
    value["uid"]["value"] = serde_json::json!(uid);
    let entry: AccessLogEntry = serde_json::from_value(value).expect("the example entry reads");
    AccessLogStore::new(pool.clone())
        .append(vec![entry])
        .await
        .expect("the entry is stored");
    uid
}

/// The erasure grounds of a law that orders it.
fn grounds() -> NonEmptyVec<DvCodedText> {
    let ground: DvCodedText = serde_json::from_value(serde_json::json!({
        "_type": "DV_CODED_TEXT", "value": "erasure ordered by law",
        "defining_code": { "terminology_id": { "value": "local" }, "code_string": "court-order" }
    }))
    .expect("the ground reads");
    NonEmptyVec::of(ground)
}

/// An access-log erasure whose record a fail-closed sender would refuse is
/// refused before it erases: the entry stays an entry.
#[tokio::test]
async fn an_erasure_whose_record_is_refused_leaves_the_entry() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let uid = stored_entry(&pool).await;
    let (sender, _handle) = rejecting_sender().await;
    let closed = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender)
        .with_audit_store(AuditStore::new(pool.clone()));

    let error = closed
        .erase_access_log_entry(&uid, grounds())
        .await
        .expect_err("the erasure's record would be refused");
    assert!(matches!(&error, ServiceError::Unrecorded(_)), "{error}");
    let record = AccessLogStore::new(pool.clone())
        .read(&uid)
        .await
        .expect("read the record");
    assert!(
        matches!(record, Some(AccessLogRecord::AccessLogEntry(_))),
        "the entry is not erased: {record:?}"
    );
}

/// An erasure writes its tombstone and the access-log entry that records it,
/// naming the erased entry, together.
#[tokio::test]
async fn an_erasure_writes_its_tombstone_and_its_entry_together() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let uid = stored_entry(&pool).await;
    let config = AuditConfig {
        enabled: true,
        fail_mode: FailMode::Closed,
        store: StoreConfig {
            enabled: true,
            retention: None,
            sgb_v_309_controller: false,
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
        },
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(config, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender)
        .with_audit_store(AuditStore::new(pool.clone()));

    service
        .erase_access_log_entry(&uid, grounds())
        .await
        .expect("the erasure");
    let recorded: Vec<(String, Option<String>)> = sqlx::query_as(
        "SELECT content, action FROM audit.access_log_record \
         WHERE record_type = 'ACCESS_LOG_ENTRY' AND content LIKE '%admin_access_log_erase%'",
    )
    .fetch_all(&pool)
    .await
    .expect("read the log");
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0].1.as_deref(), Some("delete"));
    assert!(
        recorded[0].0.contains(&uid),
        "the entry names the erased entry"
    );
    let erased = AccessLogStore::new(pool.clone())
        .read(&uid)
        .await
        .expect("read the record");
    assert!(matches!(
        erased,
        Some(AccessLogRecord::AccessLogTombstone(_))
    ));
}
