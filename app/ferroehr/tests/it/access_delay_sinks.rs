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
//! change events go to the deployment's broker: none of the three reaches
//! the person, so each carries the records and events about a held
//! composition while the hold is in force. The tests pin that: a hold does
//! not filter a controller feed, and only the person's own ITI-81 read
//! honours it. No openEHR spec governs it: our own design/extension.

#![expect(
    clippy::expect_used,
    reason = "clippy's in-test lint scoping (clippy.toml `allow-*-in-tests`) only \
              reaches `#[test]`-annotated functions, so it misses this integration \
              module's helpers and async bodies; panicking assertions are the \
              intended shape here (the Rust Book ch11)"
)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_trait::async_trait;
use serde_json::Value;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

use crate::fixtures::{composition, uv};
use ferroehr::extensions::events::config::EventsConfig;
use ferroehr::extensions::events::publisher::start_with_publisher;
use ferroehr::service::FerroEhrService;
use ferroehr::service::admin::access_delay::config::{AccessDelayConfig, AccessDelayPolicy};
use ferroehr::system_log::categories::CategoryMapConfig;
use ferroehr::system_log::config::{AuditConfig, FhirFeedConfig, StoreConfig, SyslogConfig};
use ferroehr::system_log::event::{
    AuditEvent, EmitOutcome, EventActionCode, EventOutcome, ObjectClass,
};
use ferroehr::system_log::sender::{AuditHandle, AuditSender, start};
use ferroehr_ext::events::{EventError, EventPublisher};
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
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
            full_verify_interval_seconds: 0,
        },
        ..AuditConfig::default()
    };
    if let Some(listener) = syslog {
        audit.syslog = SyslogConfig {
            enabled: true,
            host: "localhost".to_owned(),
            port: listener.port,
            tls_ca_file: Some(listener.ca_file.display().to_string()),
            ..SyslogConfig::default()
        };
    }
    if let Some(url) = feed {
        audit.fhir_feed = FhirFeedConfig {
            enabled: true,
            url: ferrohealth_secret::secret_url::SecretUrl::new(url),
            poll_interval_ms: 100,
            retry_max_ms: 500,
            oauth: None,
        };
    }
    let (sender, handle) = start(audit, None, Some(pool.clone()))
        .await
        .expect("the audit sender");
    let classifier = CategoryMapConfig::default()
        .compile()
        .expect("the shipped map compiles");
    let section = AccessDelayConfig {
        enabled: true,
        categories: vec!["Patient-Summaries".to_owned()],
        hours: 72,
    };
    let svc = FerroEhrService::new(&ferroehr::db::domain::DomainPools::from_shared(pool))
        .with_audit(sender.clone())
        .with_access_delay(AccessDelayPolicy::compile(&section, &classifier));
    (svc, sender, handle)
}

/// Commits a composition and holds it from the person by a professional's
/// act; returns the EHR id and the held object's id.
async fn held(svc: &FerroEhrService) -> (String, String) {
    let ehr = svc.create_ehr(None).await.expect("create the EHR");
    let committed = svc
        .create_composition(ehr, uv(&composition("held"), "249", None))
        .await
        .expect("commit the composition");
    let (ehr, vo) = (ehr.to_string(), committed.vo_id.to_string());
    svc.delay_person_access(&ehr, &vo, None, None)
        .await
        .expect("hold the composition");
    (ehr, vo)
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
    let mut listener = SyslogTlsListener::bind().await.expect("listener");
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

/// A publisher that records the envelope of every event it is handed.
#[derive(Default)]
struct Recording(Mutex<Vec<Value>>);

#[async_trait]
impl EventPublisher for Recording {
    async fn publish(&self, _routing_key: &str, payload: &[u8]) -> Result<(), EventError> {
        let envelope = serde_json::from_slice(payload).expect("payload json");
        self.0.lock().expect("mutex").push(envelope);
        Ok(())
    }
}

/// The change-event stream publishes the commit of a held composition while
/// the hold is in force: the drain withholds only what a professional may not
/// see (EHDS Art. 8, GDPR Art. 18 and 21), and the hold is the person's.
#[tokio::test]
async fn the_event_stream_carries_the_commit_of_a_held_composition() {
    let db = testkit::db().await.expect("testkit database");
    let pool = db.pool();
    let (svc, _sender, audit) = service(&pool, None, None).await;
    let (_ehr, vo) = held(&svc).await;

    let recording = Arc::new(Recording::default());
    let seam: Arc<dyn EventPublisher> = Arc::<Recording>::clone(&recording);
    let config = EventsConfig {
        enabled: true,
        poll_interval_ms: 50,
        publish_max_retries: 0,
        prune_interval_secs: 3_600,
        ..EventsConfig::default()
    };
    let events = start_with_publisher(
        config,
        pool.clone(),
        ferroehr::db::domain_pool_from(&pool, ferroehr::db::domain::Domain::Party),
        seam,
    );

    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    loop {
        let published = recording.0.lock().expect("mutex").iter().any(|envelope| {
            envelope["versions"]
                .as_array()
                .is_some_and(|versions| versions.iter().any(|v| v["vo_id"] == vo.as_str()))
        });
        if published {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "the commit of the held composition is published"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert_held(&svc, &vo).await;
    events.shutdown(Duration::from_secs(2)).await;
    drop(svc);
    audit.shutdown(Duration::from_secs(2)).await;
}
