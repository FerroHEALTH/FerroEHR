// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The controller feeds while a composition is held from the person (#4521):
//! EHDS Art. 3(3) beside the IHE ATNA sinks and the change-event stream.
//!
//! Regulation (EU) 2025/327 Art. 3(3) delays the person's own access, and
//! Art. 9(2) gives the person the access information "through electronic
//! health data access services" (`docs/law/eu/ehds/text.html`). The syslog
//! and FHIR feeds deliver to an Audit Record Repository, which "receives
//! event audit reports and stores them" (IHE ITI TF-1 §9.1.1.3), and the
//! change events go to the outbox feed's readers: none of the three reaches
//! the person, so each carries the records and events about a held
//! composition while the hold is in force. The tests pin that: a hold does
//! not filter a controller feed, and only the person's own ITI-81 read
//! honours it. No openEHR spec governs it: our own design/extension.

use std::time::Duration;

use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::fixtures::{composition, uv};
use ferroehr::config::access_delay::{AccessDelayConfig, AccessDelayPolicy};
use ferroehr::extensions::outbox::{OutboxDomain, reconcile};
use ferroehr::service::FerroEhrService;
use ferroehr::system_log::categories::CategoryMapConfig;
use ferroehr::system_log::config::{AuditConfig, FhirFeedConfig, StoreConfig, SyslogConfig};
use ferroehr::system_log::event::{
    AuditEvent, EmitOutcome, EventActionCode, EventOutcome, ObjectClass,
};
use ferroehr::system_log::sender::{AuditHandle, AuditSender, start};
use testkit::syslog_tls::SyslogTlsListener;

/// A service that holds patient summaries for 72 hours and audits to the
/// local store, to `syslog` when given and to the FHIR repository at `feed`
/// when given; returns the service, the sender it emits through, and the
/// sender's handle.
async fn service(
    pool: &sqlx::PgPool,
    syslog: Option<&SyslogTlsListener>,
    feed: Option<&str>,
) -> (FerroEhrService, AuditSender, AuditHandle) {
    let mut audit = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: true,
            retention: None,
            sgb_v_309_controller: false,
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            ..StoreConfig::default()
        },
        ..AuditConfig::default()
    };
    if let Some(listener) = syslog {
        audit.syslog = Some(SyslogConfig {
            url: format!("tls://localhost:{}", listener.port),
            tls: ferrohealth_config::tls::TlsClientSection {
                trust_roots_file: Some(listener.ca_file.clone()),
                ..ferrohealth_config::tls::TlsClientSection::default()
            },
            ..SyslogConfig::default()
        });
    }
    if let Some(url) = feed {
        audit.fhir_feed = Some(FhirFeedConfig {
            endpoint: ferrohealth_smart::config::endpoint::EndpointSection {
                url: url.to_owned(),
                retry: ferrohealth_smart::config::endpoint::RetrySection {
                    max_backoff: ferrohealth_config::units::Duration::from_millis(500),
                    ..ferrohealth_smart::config::endpoint::RetrySection::default()
                },
                ..ferrohealth_smart::config::endpoint::EndpointSection::default()
            },
            poll_interval: ferrohealth_config::units::Duration::from_millis(100),
            ..FhirFeedConfig::default()
        });
    }
    let (sender, handle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let classifier = CategoryMapConfig::default()
        .compile()
        .expect("the shipped map compiles");
    let section = AccessDelayConfig {
        categories: vec!["Patient-Summaries".to_owned()],
        max_delay: Some(ferrohealth_config::units::Duration::from_hours(72)),
    };
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_audit(sender.clone())
        .with_access_delay(AccessDelayPolicy::compile(&section, &classifier));
    (svc, sender, handle)
}

/// Commits a composition and holds it from the person by a professional's
/// act; returns the EHR id and the held object's id.
async fn held(svc: &FerroEhrService) -> (String, String) {
    let ehr = crate::fixtures::create_ehr(svc).await;
    let committed = svc
        .create_composition(ehr, uv(&composition("held"), "249", None))
        .await
        .expect("commit the composition");
    svc.delay_person_access(ehr, committed.vo_id, None, None)
        .await
        .expect("hold the composition");
    (ehr.to_string(), committed.vo_id.to_string())
}

/// Asserts the hold on `vo` is still in force.
async fn assert_held(svc: &FerroEhrService, vo: &str) {
    let vo_id = ferroehr::ids::VoId(vo.parse().expect("a UUID"));
    assert!(
        svc.is_object_withheld(vo_id).await.expect("read the hold"),
        "the hold on {vo} is in force"
    );
}

/// The syslog feed (ITI-20) sends the record of placing the hold and a
/// professional's read of the held composition while the hold is in force;
/// both name the composition, the hold's own record as `access-delay:<vo>`,
/// the object its FHIR form names.
#[tokio::test]
async fn the_syslog_feed_carries_a_record_about_a_held_composition() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let mut listener = SyslogTlsListener::bind(0).await.expect("listener");
    let (svc, sender, handle) = service(&pool, Some(&listener), None).await;
    let (ehr, vo) = held(&svc).await;
    let mut read = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Composition,
        EventOutcome::Success,
    );
    "dr".clone_into(&mut read.user_id);
    read.ehr_id = Some(ehr);
    read.object_id = Some(format!("{vo}::ferroehr::1"));
    assert_eq!(
        sender.emit(read),
        EmitOutcome::Enqueued,
        "the read is recorded"
    );

    let mut marks = vec![
        format!("ParticipantObjectID=\"access-delay:{vo}\""),
        format!("ParticipantObjectID=\"{vo}::ferroehr::1\""),
    ];
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    let mut frames = Vec::new();
    while !marks.is_empty() {
        let wait = deadline.saturating_duration_since(std::time::Instant::now());
        let Some(frame) = listener.recv(wait).await else {
            panic!("the records naming {marks:?} reach the syslog repository: {frames:#?}");
        };
        let frame = String::from_utf8_lossy(&frame).into_owned();
        marks.retain(|mark| !frame.contains(mark.as_str()));
        frames.push(frame);
    }
    assert_held(&svc, &vo).await;
    drop(svc);
    drop(sender);
    handle.shutdown(Duration::from_secs(2)).await;
}

/// The FHIR feed (ITI-20 ATX: FHIR Feed) `POST`s the record of placing the
/// hold, which names the held composition, while the hold is in force.
#[tokio::test]
async fn the_fhir_feed_carries_a_record_about_a_held_composition() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let arr = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/fhir/AuditEvent"))
        .respond_with(ResponseTemplate::new(201))
        .mount(&arr)
        .await;
    let (svc, _sender, handle) = service(&pool, None, Some(&format!("{}/fhir", arr.uri()))).await;
    let (_ehr, vo) = held(&svc).await;

    let mark = format!("access-delay:{vo}");
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let fed = arr
            .received_requests()
            .await
            .unwrap_or_default()
            .iter()
            .any(|request| String::from_utf8_lossy(&request.body).contains(&mark));
        if fed {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the record of the hold reaches the FHIR repository"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_held(&svc, &vo).await;
    drop(svc);
    handle.shutdown(Duration::from_secs(2)).await;
}

/// The outbox feed carries the commit of a held composition while the hold
/// is in force: a commit withholds its event only for what a professional may
/// not see (EHDS Art. 8, GDPR Art. 18 and 21), and the hold is the person's.
#[tokio::test]
async fn the_event_stream_carries_the_commit_of_a_held_composition() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    reconcile(&pool, &["ferrobridge".to_owned()])
        .await
        .expect("register the reader");
    let (svc, _sender, audit) = service(&pool, None, None).await;
    let (_ehr, vo) = held(&svc).await;

    let events = svc
        .outbox_events(OutboxDomain::Clinical, "ferrobridge", 100)
        .await
        .expect("read the feed");
    let carried = events.iter().any(|event| {
        event.envelope["versions"]
            .as_array()
            .is_some_and(|versions| versions.iter().any(|v| v["vo_id"] == vo.as_str()))
    });
    assert!(carried, "the commit of the held composition is in the feed");
    assert_held(&svc, &vo).await;
    drop(svc);
    audit.shutdown(Duration::from_secs(2)).await;
}
