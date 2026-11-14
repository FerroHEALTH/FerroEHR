// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Two replicas of one deployment writing the audit chains at once, against
//! a real `PostgreSQL` 18 (shared testkit harness).
//!
//! Each replica has its own pools, its own audit drain and the same signing
//! key, as two instances behind a load balancer do. Both commit EHRs, record
//! accesses to them in the access log, write records of the system chain and
//! seal both chains concurrently; every writer of a chain serialises on its
//! head row in PostgreSQL, so afterwards the access log, the system chain,
//! every signed link, every checkpoint and every seal verify, whichever
//! replica made them. No openEHR spec governs the process model or audit
//! storage: our own design (#5589).

use std::sync::Arc;
use std::time::Duration;

use ferroehr::access_log::checkpoint::Checkpoints;
use ferroehr::db::domain::DomainPools;
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::sender::{AuditHandle, AuditSender, start};
use ferroehr::system_log::store::AuditStore;
use ferroehr::versioning::signature::signer::Signer;
use ferroehr::versioning::signature::trust::TrustedCertificates;

/// The system both replicas record as.
const SYSTEM: &str = "cdr.example.org";

/// The rounds each replica runs.
const ROUNDS: usize = 24;

/// One replica: its service, its audit drain, its checkpoints and its store.
struct Replica {
    svc: FerroEhrService,
    sender: AuditSender,
    _drain: AuditHandle,
    checkpoints: Checkpoints,
    store: AuditStore,
}

/// A replica over `pool`'s database with pools and an audit drain of its
/// own, signing with `signer`.
async fn replica(pool: &sqlx::PgPool, signer: &Arc<Signer>) -> Replica {
    let pools = DomainPools::from_shared(pool);
    let config = ferroehr::system_log::config::AuditConfig {
        enabled: true,
        store: ferroehr::system_log::config::StoreConfig {
            enabled: true,
            retention: None,
            sgb_v_309_controller: false,
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
        },
        ..ferroehr::system_log::config::AuditConfig::default()
    };
    let (sender, drain) = start(config, None, Some(pools.audit.clone()))
        .await
        .expect("the audit drain starts");
    let checkpoints = Checkpoints::new(
        pools.audit.clone(),
        SYSTEM.to_owned(),
        Arc::clone(signer),
        Arc::new(TrustedCertificates::default()),
    );
    assert!(
        sender.use_access_log(checkpoints.store().clone()),
        "the drain appends through the signing store"
    );
    let svc = FerroEhrService::new(&pools)
        .with_system_id(SYSTEM)
        .with_audit(sender.clone())
        .with_access_log(checkpoints.clone());
    Replica {
        svc,
        sender,
        _drain: drain,
        checkpoints,
        store: AuditStore::new(pools.audit),
    }
}

/// `ROUNDS` rounds of the work of one replica: commit an EHR, record a read
/// of it in the access log, write a record of the system chain, and every
/// third round seal both chains.
async fn work(replica: &Replica) {
    for round in 0..ROUNDS {
        let ehr_id = crate::fixtures::create_ehr(&replica.svc).await;
        let mut read = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Ehr,
            EventOutcome::Success,
        );
        "dr".clone_into(&mut read.user_id);
        read.ehr_id = Some(ehr_id.to_string());
        replica.sender.emit(read);
        let mut outside = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Demographic,
            EventOutcome::Success,
        );
        "dr".clone_into(&mut outside.user_id);
        replica
            .store
            .insert(&outside, None)
            .await
            .expect("a record of the system chain");
        if round % 3 == 2 {
            replica
                .checkpoints
                .seal()
                .await
                .expect("the access log is sealed");
            replica
                .checkpoints
                .system_seals()
                .seal()
                .await
                .expect("the system chain is sealed");
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn two_replicas_writing_at_once_leave_every_chain_verifying() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let signer =
        crate::access_log_checkpoint::keyring(&crate::access_log_checkpoint::generate_key());
    let a = replica(&pool, &signer).await;
    let b = replica(&pool, &signer).await;
    tokio::join!(work(&a), work(&b));
    let expected = i64::try_from(2 * ROUNDS).expect("a count");
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    loop {
        let entries: i64 = sqlx::query_scalar("SELECT count(*) FROM audit.access_log_record")
            .fetch_one(&pool)
            .await
            .expect("the access log is counted");
        if entries == expected {
            break;
        }
        assert!(
            entries < expected && std::time::Instant::now() < deadline,
            "every read of both replicas is one entry: {entries} of {expected}"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    let unsigned: i64 =
        sqlx::query_scalar("SELECT count(*) FROM audit.access_log_record WHERE signature IS NULL")
            .fetch_one(&pool)
            .await
            .expect("the unsigned links are counted");
    assert_eq!(unsigned, 0, "every link is signed");
    let findings = crate::fixtures::verify_chain(&pool)
        .await
        .expect("both chains are verified");
    assert!(
        findings.is_empty(),
        "the access log and the system chain verify: {findings:?}"
    );
    let sealed = a
        .checkpoints
        .seal()
        .await
        .expect("the head is sealed")
        .expect("a checkpoint");
    for (name, checkpoints) in [("a", &a.checkpoints), ("b", &b.checkpoints)] {
        let verified = checkpoints
            .verify(Some(sealed.clone()))
            .await
            .expect("the access log is verified");
        assert!(verified.intact(), "replica {name}: {verified:?}");
        let seals = checkpoints
            .system_seals()
            .verify(None)
            .await
            .expect("the system chain is verified");
        assert!(seals.intact(), "replica {name}: {seals:?}");
    }
}
