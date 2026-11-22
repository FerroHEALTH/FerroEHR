// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RFC 5425 TLS framing round-trip (§4.3): the syslog route delivers each
//! stored record from the store's syslog outbox as an octet-counted syslog
//! record over a real TLS connection to an in-process rustls listener that
//! presents a leaf the test CA signed, the route trusting that CA, and the
//! framed bytes arrive.

use std::time::Duration;

use ferroehr::system_log::config::{AuditConfig, StoreConfig, SyslogConfig};
use ferroehr::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
use ferroehr::system_log::sender::{AuditHandle, AuditSender, start};
use testkit::syslog_tls::{Ending, SyslogTlsListener};

/// An audit subsystem over `pool` whose syslog route sends to `listener` as
/// `cdr-01`, retrying after at most `max_backoff` milliseconds.
async fn audited(
    pool: &sqlx::PgPool,
    listener: &SyslogTlsListener,
    max_backoff: u64,
) -> (AuditSender, AuditHandle) {
    let config = AuditConfig {
        enabled: true,
        server_host: Some("cdr-01".to_owned()),
        store: StoreConfig {
            verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            full_verify_interval: ferrohealth_config::units::Duration::from_secs(0),
            max_verify_lag: ferrohealth_config::units::Duration::from_secs(0),
            ..StoreConfig::default()
        },
        syslog: Some(SyslogConfig {
            url: ferrohealth_secret::secret_url::SecretUrl::new(format!(
                "tls://localhost:{}",
                listener.port
            )),
            tls: ferrohealth_config::tls::TlsClientSection {
                trust_roots_file: Some(listener.ca_file.clone()),
                ..ferrohealth_config::tls::TlsClientSection::default()
            },
            retry: ferroehr::system_log::config::SinkRetry {
                max_backoff: ferrohealth_config::units::Duration::from_millis(max_backoff),
            },
        }),
        ..AuditConfig::default()
    };
    start(config, None, pool.clone())
        .await
        .expect("the audit subsystem starts")
}

/// A record of no access to EHR data, stamped at a fixed instant.
fn event() -> AuditEvent {
    let mut event = AuditEvent::new(
        EventActionCode::Execute,
        ObjectClass::Authentication,
        EventOutcome::Success,
    );
    event.timestamp = "2026-07-06T12:00:00Z".parse().expect("instant");
    event
}

/// The ITI-20 frame: PRI `<85>`, the record's instant, the host and the
/// source, MSGID `IHE+RFC-3881`, no STRUCTURED-DATA, and the DICOM audit
/// message as MSG, octet-counted (ITI TF-2 §3.20.4.1.2, RFC 5425 §4.3).
fn assert_framed(frame: Vec<u8>) {
    let text = String::from_utf8(frame).expect("UTF-8");
    let (count, record) = text.split_once(' ').expect("an octet count");
    assert_eq!(count.parse::<usize>().ok(), Some(record.len()), "{text}");
    let message = record
        .strip_prefix(&format!(
            "<85>1 2026-07-06T12:00:00.000000Z cdr-01 {} - IHE+RFC-3881 - ",
            ferroehr::service::DEFAULT_SYSTEM_ID
        ))
        .unwrap_or_else(|| panic!("the RFC 5424 header: {text}"));
    assert!(
        message.starts_with("<?xml") && message.contains("<AuditMessage>"),
        "the DICOM audit message with no byte order mark: {text}"
    );
}

/// Two stored records travel as two frames on one connection.
#[tokio::test]
async fn tls_octet_counted_frame_round_trips() {
    let db = testkit::db().await.expect("testkit database");
    let mut listener = SyslogTlsListener::bind(0).await.expect("listener");
    let (sender, handle) = audited(&db.pool(), &listener, 1000).await;
    sender
        .record(vec![event(), event()], None)
        .await
        .expect("the records are written");
    for _ in 0..2 {
        let received = listener
            .recv(Duration::from_secs(10))
            .await
            .expect("a frame arrives");
        assert_framed(received);
    }
    handle.shutdown(Duration::from_secs(5)).await;
}

/// ITI TF-2 §3.20.4.1.1: a record whose first send fails waits in the store
/// and is sent once the repository answers, so a repository that drops the
/// connection loses nothing.
#[tokio::test]
async fn a_record_whose_send_fails_is_delivered_later() {
    let db = testkit::db().await.expect("testkit database");
    let mut listener = SyslogTlsListener::bind(1).await.expect("listener");
    let (sender, handle) = audited(&db.pool(), &listener, 500).await;
    sender
        .record(vec![event()], None)
        .await
        .expect("the record is written");
    let received = listener
        .recv(Duration::from_secs(15))
        .await
        .expect("the record arrives after the dropped connection");
    assert_framed(received);
    let delivery = handle.delivery();
    let counted = tokio::time::timeout(Duration::from_secs(15), async {
        loop {
            let statuses = delivery.statuses();
            if statuses.iter().any(|(_, status)| status.delivered >= 1) {
                return statuses;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("the delivery is counted");
    let (sink, status) = counted.first().expect("the syslog route");
    assert_eq!(*sink, "syslog");
    assert!(status.retries >= 1, "the first attempt failed: {status:?}");
    assert_eq!(status.delivered, 1, "{status:?}");
    handle.shutdown(Duration::from_secs(5)).await;
}

/// The route ends its connection with a TLS `close_notify` when it stops, as
/// RFC 5425 §4.4 asks of a transport sender: a dropped connection is reset
/// when the repository's session tickets sit unread, and the reset can
/// discard the records sent last (#3959).
#[tokio::test]
async fn the_route_closes_its_connection_with_close_notify_when_it_stops() {
    let db = testkit::db().await.expect("testkit database");
    let mut listener = SyslogTlsListener::bind(0).await.expect("listener");
    let (sender, handle) = audited(&db.pool(), &listener, 1000).await;
    sender
        .record(vec![event()], None)
        .await
        .expect("the record is written");
    assert!(
        listener.recv(Duration::from_secs(10)).await.is_some(),
        "the record arrives"
    );
    drop(sender);
    handle.shutdown(Duration::from_secs(10)).await;
    assert_eq!(
        listener.ending(Duration::from_secs(10)).await,
        Some(Ending::CloseNotify),
        "the route ended its connection without a close_notify"
    );
}
