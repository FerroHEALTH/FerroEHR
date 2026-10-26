// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RFC 5425 TLS framing round-trip (§4.3): the syslog sink sends an
//! octet-counted syslog record over a real TLS connection to an in-process
//! rustls listener that presents a leaf the test CA signed, the sink trusting
//! that CA, and the exact framed bytes arrive.

use std::time::Duration;

use ferroehr::system_log::config::{AuditConfig, FailMode, StoreConfig, SyslogConfig};
use ferroehr::system_log::event::{
    AuditEvent, EmitOutcome, EventActionCode, EventOutcome, ObjectClass,
};
use ferroehr::system_log::sender::start;
use ferroehr::system_log::syslog::SyslogRoute;
use secrecy::SecretSlice;
use testkit::syslog_tls::{Ending, SyslogTlsListener};

#[tokio::test]
async fn tls_octet_counted_frame_round_trips() {
    let mut listener = SyslogTlsListener::bind().await.expect("listener");
    let config = SyslogConfig {
        enabled: true,
        host: "localhost".to_owned(),
        port: listener.port,
        tls_ca_file: Some(listener.ca_file.display().to_string()),
        ..SyslogConfig::default()
    };
    let route = SyslogRoute::new(&config, "cdr-01", "ferroehr")
        .await
        .expect("route");
    let forwarder = route.forwarder();
    let run = tokio::spawn(std::sync::Arc::clone(&forwarder).run());
    let at = "2026-07-06T12:00:00Z".parse().expect("instant");
    let xml = SecretSlice::from(b"<AuditMessage/>".to_vec());
    route.submit(at, &xml).await.expect("stored");
    // A second record travels on the same connection.
    route.submit(at, &xml).await.expect("second stored");

    for _ in 0..2 {
        let received = listener
            .recv(Duration::from_secs(5))
            .await
            .expect("a frame arrives");
        let text = String::from_utf8(received).expect("UTF-8");
        // ITI TF-2 §3.20.4.1.2: PRI <85>, MSGID IHE+RFC-3881, no STRUCTURED-DATA,
        // and the XML as MSG with no byte order mark.
        let message = "<85>1 2026-07-06T12:00:00.000000Z cdr-01 ferroehr - IHE+RFC-3881 - \
                       <AuditMessage/>";
        assert_eq!(text, format!("{} {message}", message.len()));
    }
    forwarder.stop();
    run.await.expect("the forwarder stops");
}

/// ITI TF-2 §3.20.4.1.1: a record whose first send fails is stored and sent
/// once the repository answers, so a repository that drops the connection
/// loses nothing.
#[tokio::test]
async fn a_record_whose_send_fails_is_delivered_later() {
    let mut listener = SyslogTlsListener::bind_refusing(1).await.expect("listener");
    let config = SyslogConfig {
        enabled: true,
        host: "localhost".to_owned(),
        port: listener.port,
        tls_ca_file: Some(listener.ca_file.display().to_string()),
        retry_max_ms: 500,
        ..SyslogConfig::default()
    };
    let route = SyslogRoute::new(&config, "cdr-01", "ferroehr")
        .await
        .expect("route");
    let forwarder = route.forwarder();
    let run = tokio::spawn(std::sync::Arc::clone(&forwarder).run());
    let at = "2026-07-06T12:00:00Z".parse().expect("instant");
    route
        .submit(at, &SecretSlice::from(b"<AuditMessage/>".to_vec()))
        .await
        .expect("stored");
    assert!(
        listener.recv(Duration::from_secs(15)).await.is_some(),
        "the record arrives after the dropped connection"
    );
    // The listener holds the record before the forwarder counts it, so the
    // count is read once the forwarder has moved it, within a bound.
    let counted = tokio::time::timeout(Duration::from_secs(15), async {
        while forwarder.status().delivered == 0 {
            tokio::task::yield_now().await;
        }
    })
    .await;
    let status = forwarder.status();
    assert!(counted.is_ok(), "the delivery is never counted: {status:?}");
    assert!(status.retries >= 1, "the first attempt failed: {status:?}");
    assert_eq!(status.delivered, 1, "{status:?}");
    assert_eq!(status.depth.waiting(), 0, "{status:?}");
    forwarder.stop();
    run.await.expect("the forwarder stops");
}

/// The drain ends its syslog connection with a TLS `close_notify` when it
/// exits, as RFC 5425 §4.4 asks of a transport sender: a dropped connection is
/// reset when the repository's session tickets sit unread, and the reset can
/// discard the records sent last (#3959).
#[tokio::test]
async fn the_drain_closes_its_connection_with_close_notify_when_it_exits() {
    let mut listener = SyslogTlsListener::bind().await.expect("listener");
    let config = AuditConfig {
        enabled: true,
        store: StoreConfig {
            enabled: false,
            retention_days: 0,
            retention_years: None,
            sgb_v_309_controller: false,
            verify_interval_seconds: 0,
        },
        syslog: SyslogConfig {
            enabled: true,
            host: "localhost".to_owned(),
            port: listener.port,
            tls_ca_file: Some(listener.ca_file.display().to_string()),
            ..SyslogConfig::default()
        },
        fail_mode: FailMode::Closed,
        queue_capacity: 64,
        ..AuditConfig::default()
    };
    let (sender, handle) = start(config, None, None).await.expect("audit start");
    let event = AuditEvent::new(
        EventActionCode::Read,
        ObjectClass::Ehr,
        EventOutcome::Success,
    );
    assert_eq!(sender.emit(event), EmitOutcome::Enqueued);
    assert!(
        listener.recv(Duration::from_secs(10)).await.is_some(),
        "the record arrives"
    );
    drop(sender);
    handle.shutdown(Duration::from_secs(10)).await;

    assert_eq!(
        listener.ending(Duration::from_secs(10)).await,
        Some(Ending::CloseNotify),
        "the drain ended its connection without a close_notify"
    );
}
