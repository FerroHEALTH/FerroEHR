// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An access record that cannot be written refuses the access it reports
//! (#3235, #5747): the subject-to-EHR lookup, the EHR-Extract export and the
//! access-log erasure each write their own record, and each withholds its
//! result when the record cannot be written.

use std::time::Duration;

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use ferroehr::access_log::store::AccessLogStore;
use ferroehr::error::ServiceError;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::config::{AuditConfig, StoreConfig};
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::sender::{AuditHandle, start};
use ferroehr::system_log::store::AuditStore;
use openehr_base::containers::NonEmptyVec;
use openehr_rm::draft::access_log::access_log::access_log_entry::AccessLogEntry;
use openehr_rm::draft::access_log::access_log::access_log_record::AccessLogRecord;
use openehr_rm::v1_2::data_types::text::dv_coded_text::DvCodedText;

use crate::admin_fixture::seed_full_ehr;

/// A sender whose store cannot be reached, so no record it is given is
/// written.
async fn rejecting_sender() -> (ferroehr::system_log::sender::AuditSender, AuditHandle) {
    let unreachable: PgPool = PgPoolOptions::new()
        .acquire_timeout(Duration::from_millis(200))
        .connect_lazy("postgres://nobody:nothing@127.0.0.1:9/nowhere")
        .expect("a lazy pool builds without connecting");
    let config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            max_verify_lag: ferrohealth_config::units::Duration::from_secs(0),
            ..StoreConfig::default()
        },
        ..AuditConfig::default()
    };
    let (sender, handle): (_, AuditHandle) = start(config, None, unreachable)
        .await
        .expect("the audit sender");
    let probe = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    sender
        .record(vec![probe], None)
        .await
        .expect_err("a record the store cannot take is an error, never a drop");
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

/// An access-log erasure whose record cannot be written is refused, and the
/// entry stays an entry.
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
        store: StoreConfig {
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            max_verify_lag: ferrohealth_config::units::Duration::from_secs(0),
            ..StoreConfig::default()
        },
        ..AuditConfig::default()
    };
    let (sender, _handle): (_, AuditHandle) = start(config, None, pool.clone())
        .await
        .expect("the audit sender");
    let service = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender)
        .with_audit_store(AuditStore::new(pool.clone()));

    service
        .erase_access_log_entry(&uid, grounds())
        .await
        .expect("the erasure");
    let recorded: Vec<(String, Option<String>)> = sqlx::query_as::<_, (String, Option<String>)>(
        "SELECT audit.access_log_document(r.content, sh.content, r.record_type, r.uid, \
         r.time_recorded, r.ehr_id, r.request_id, r.resource_id)::text, r.action_code FROM audit.access_log_record r \
         LEFT JOIN audit.access_log_shared sh ON sh.digest = r.shared \
         WHERE r.record_type = 'ACCESS_LOG_ENTRY'",
    )
    .fetch_all(&pool)
    .await
    .map(|rows| {
        rows.into_iter()
            .map(|(document, action)| (crate::fixtures::canonical_entry(&document), action))
            .filter(|(content, _)| content.contains("admin_access_log_erase"))
            .collect()
    })
    .expect("read the log");
    assert_eq!(recorded.len(), 1, "{recorded:?}");
    assert_eq!(recorded[0].1.as_deref(), Some("D"));
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
