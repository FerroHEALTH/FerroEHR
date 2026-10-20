// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! RFC 5425 TLS framing round-trip (§4.3): the syslog sink sends an
//! octet-counted syslog record over a real TLS connection to an in-process
//! rustls listener that presents a leaf the test CA signed, the sink trusting
//! that CA, and the exact framed bytes arrive.

use std::time::Duration;

use ferroehr::system_log::config::SyslogConfig;
use ferroehr::system_log::syslog::SyslogSink;
use secrecy::SecretSlice;
use testkit::syslog_tls::SyslogTlsListener;

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
