// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[audit] fail_mode = "closed"` reaches the DOMAIN-level access records
//! (#3235): the linkage resolution, the subject-to-EHR lookup, the
//! national-identifier resolution and the EHR-Extract export each emit their
//! own record, and each must withhold its result when the sender rejects it.
//!
//! The rejecting sender is a real one whose store is unreachable: the drain's
//! first write fails, the store is marked unhealthy, and from then on a
//! fail-closed sender answers `Rejected` (`fail_closed_rejects_while_store_unhealthy`
//! in the sender's own tests pins that transition). No test hook in production
//! code; the posture is the one an operator meets when the audit database is
//! down.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use sqlx::PgPool;
use sqlx::postgres::PgPoolOptions;
use uuid::Uuid;

use ferroehr::ids::{EhrId, VoId};
use ferroehr::service::FerroEhrService;
use ferroehr::service::demographic::identifier::config::IdentifierProtectionConfig;
use ferroehr::service::demographic::identifier::engine::IdentifierProtection;
use ferroehr::service::ehr_index::types::SubjectRef;
use ferroehr::service::linkage::LinkageError;
use ferroehr::service::status::CallStatusType;
use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig};
use ferroehr::system_log::event::{
    AuditEvent, EmitOutcome, EventActionCode, EventOutcome, ObjectClass,
};
use ferroehr::system_log::sender::{AuditHandle, start};

use crate::admin_fixture::seed_full_ehr;

const TEST_ROOT_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";
const SYNTHETIC_BSN: &str = "111222333"; // privacy-allow: synthetic

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
            retention_days: 0,
            sgb_v_309_controller: false,
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

/// A resolution through the linkage map is withheld when its record is rejected.
#[tokio::test]
async fn a_linkage_resolution_is_withheld_when_its_record_is_rejected() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    // The mapping is opened under a healthy, fail-open sender so the fixture is
    // not what is under test.
    let open = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool));
    let party = VoId(Uuid::now_v7());
    let ehr = EhrId(Uuid::now_v7());
    open.link(party, ehr).await.expect("the mapping opens");

    let (sender, _handle) = rejecting_sender().await;
    let closed = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_audit(sender);

    let refused = closed.resolve_ehr_for_party(party).await;
    assert!(
        matches!(refused, Err(LinkageError::Unrecorded(_))),
        "the mapping resolved but the result is withheld: {refused:?}"
    );
    let refused = closed.merge(party, VoId(Uuid::now_v7())).await;
    assert!(
        matches!(refused, Err(LinkageError::Unrecorded(_))),
        "a merge whose record is rejected reports so: {refused:?}"
    );
}

/// The subject-to-EHR lookup is withheld when its record is rejected, and the
/// SM route reports the `service_overloaded` status the wire maps to `503`.
#[tokio::test]
async fn a_subject_lookup_is_withheld_when_its_record_is_rejected() {
    let db = testkit::db().await.expect("testkit database");
    let (sender, _handle) = rejecting_sender().await;
    let closed = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&db.pool()))
        .with_audit(sender);

    let refused = closed
        .has_ehr_for_subject(SubjectRef {
            id: Uuid::now_v7().to_string(),
            namespace: "urn:example:pseudonym".to_owned(),
            r#type: "PERSON".to_owned(),
        })
        .await;
    let error = refused.expect_err("a miss is still a lookup, and its record was rejected");
    assert_eq!(error.status, CallStatusType::ServiceOverloaded, "{error}");
}

/// The national-identifier resolution is withheld when its record is rejected.
#[tokio::test]
async fn an_identifier_resolution_is_withheld_when_its_record_is_rejected() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let engine = IdentifierProtection::from_config(
        &IdentifierProtectionConfig {
            enabled: true,
            schemes: vec!["nl-bsn".to_owned()],
            key: Some(ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
            key_file: None,
        },
        Some(&ferroehr::config::secret::Secret::new(TEST_ROOT_KEY)),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
    )
    .expect("the engine builds")
    .expect("protection is enabled");
    let (sender, _handle) = rejecting_sender().await;
    let closed = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(&pool))
        .with_identifier_protection(Arc::new(engine))
        .with_audit(sender);

    let refused = closed
        .resolve_party_by_identifier("nl-bsn", SYNTHETIC_BSN)
        .await;
    let error = refused.expect_err("the resolution ran, and its record was rejected");
    assert_eq!(error.status, CallStatusType::ServiceOverloaded, "{error}");
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
    assert_eq!(error.status, CallStatusType::ServiceOverloaded, "{error}");
}
