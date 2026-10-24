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
use ferroehr::system_log::syslog::SyslogSink;
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
    let sink = SyslogSink::new(&config, "cdr-01", "ferroehr")
        .await
        .expect("sink");
    let at = "2026-07-06T12:00:00Z".parse().expect("instant");
    let xml = SecretSlice::from(b"<AuditMessage/>".to_vec());
    sink.send(at, &xml).await.expect("tls send");
    // A second record travels on the same connection.
    sink.send(at, &xml).await.expect("second send");

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
