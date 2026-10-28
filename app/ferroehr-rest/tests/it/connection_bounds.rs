// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The connection-level bounds, driven over a real socket.
//!
//! These have to be tested at the transport, not through a `Router`, because
//! that is the whole point of them: every other limit this server has engages
//! once a request head has been parsed and dispatched — the body limit, the
//! request timeout, both rate-limit tiers, the in-flight shed. A client that
//! opens a connection and never finishes writing a request head reaches none of
//! them, so a test that goes through the router cannot observe the bound at all.
//!
//! No openEHR spec governs connection handling — our own design; the control is
//! the OWASP Denial of Service Cheat Sheet's "minimum ingress rate threshold".

#![allow(
    clippy::panic_in_result_fn,
    clippy::panic,
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test assertions and fixtures"
)]

use std::time::Duration;

use ferrohealth_health::lifecycle::Lifecycle;
use ferrohealth_server::section::ServerSection;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;

/// Serve a trivial router on an ephemeral port through the family listener
/// FerroEHR serves on, with a header-read bound of one second, returning the
/// bound port.
async fn serve() -> u16 {
    let app = axum::Router::new().route("/ok", axum::routing::get(|| async { "OK" }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let section = ServerSection {
        header_read_timeout_ms: std::num::NonZeroU64::new(1_000).expect("non-zero"),
        ..ServerSection::default()
    };
    tokio::spawn(async move {
        let _served = ferrohealth_server::serve::run(
            listener,
            app,
            &section,
            Lifecycle::default(),
            std::future::pending(),
        )
        .await;
    });
    port
}

/// A connection that opens and trickles a partial request head is dropped once
/// the header-read bound elapses, rather than being held indefinitely.
///
/// The bound is set to one second so the test is quick; the shape is what
/// matters, not the number.
#[tokio::test]
async fn a_stalled_connection_is_dropped_rather_than_held() {
    let port = serve().await;

    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    // A request line and one header, deliberately never terminated — the
    // Slowloris shape. No further bytes are ever sent.
    socket
        .write_all(b"GET /ok HTTP/1.1\r\nHost: localhost\r\n")
        .await
        .expect("partial write");

    // The server must close the connection of its own accord. A read on a
    // half-closed socket returns 0 bytes; if the bound were absent this would
    // block until the test's own timeout instead.
    let mut buffer = [0_u8; 64];
    let outcome = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut buffer)).await;
    let read = outcome.expect("the connection must be dropped, not held open");
    let bytes = read.unwrap_or(0);
    if bytes > 0 {
        // hyper may answer `408 Request Timeout` before closing, which is the
        // same outcome expressed politely — what must not happen is the
        // connection being held.
        let head = String::from_utf8_lossy(&buffer[..bytes]);
        assert!(
            head.contains("408") || head.contains("400"),
            "an unfinished request head must not be answered normally: {head}"
        );
    }
}

/// A complete request on the same configuration is served normally, so the bound
/// is a bound on stalling rather than a shorter deadline for everyone.
#[tokio::test]
async fn a_complete_request_is_unaffected_by_the_bound() {
    let port = serve().await;

    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    socket
        .write_all(b"GET /ok HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n")
        .await
        .expect("write");

    let mut response = Vec::new();
    tokio::time::timeout(Duration::from_secs(5), socket.read_to_end(&mut response))
        .await
        .expect("a complete request must be answered")
        .expect("read");
    let text = String::from_utf8_lossy(&response);
    assert!(
        text.starts_with("HTTP/1.1 200"),
        "a complete request must be served: {text}"
    );
}

/// A connection that opens and never sends a byte is closed once the
/// header-read bound passes, counted from accept, so silent connections
/// cannot hold the process's file descriptors (GHSA-2mc2-h3jq-c5wg).
#[tokio::test]
async fn a_silent_connection_is_closed_at_the_header_read_bound() {
    let port = serve().await;

    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");

    let mut buffer = [0_u8; 64];
    let outcome = tokio::time::timeout(Duration::from_secs(10), socket.read(&mut buffer)).await;
    let read = outcome.expect("a silent connection must be closed, not held open");
    assert_eq!(
        0,
        read.unwrap_or(0),
        "a connection that sent nothing must be closed without an answer"
    );
}

/// Serves the assembled FerroEHR router over a real service through the family
/// listener, with the header-read bound of [`serve`], returning the bound port
/// and the database guard the service reads.
async fn serve_ferroehr() -> (testkit::TestDb, u16) {
    let (db, app) = crate::common::test_router().await;
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind");
    let port = listener.local_addr().expect("addr").port();
    let section = ServerSection {
        header_read_timeout_ms: std::num::NonZeroU64::new(1_000).expect("non-zero"),
        ..crate::common::server()
    };
    tokio::spawn(async move {
        let _served = ferrohealth_server::serve::run(
            listener,
            app,
            &section,
            Lifecycle::default(),
            std::future::pending(),
        )
        .await;
    });
    (db, port)
}

/// A `POST` of a large body to a path FerroEHR does not serve is answered `404`
/// with its body unread, and the connection is then either reused for the
/// client's next request or announced closed with `Connection: close`
/// (RFC 9112 §9.6), so a client never sends its next request into a socket the
/// server already closed.
#[tokio::test]
async fn a_connection_after_an_unmatched_large_post_is_reused_or_announced_closed() {
    const SIZE: usize = 300_000;
    let (_db, port) = serve_ferroehr().await;

    let stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .expect("connect");
    let (mut reader, mut writer) = stream.into_split();
    let head = format!(
        "POST {}/no-such-resource HTTP/1.1\r\nHost: localhost\r\n\
         Content-Type: application/xml\r\nContent-Length: {SIZE}\r\n\r\n",
        crate::common::BASE
    );
    let sent = tokio::spawn(async move {
        writer.write_all(head.as_bytes()).await?;
        writer.write_all(&vec![b'x'; SIZE]).await?;
        writer
            .write_all(
                b"GET /health/liveness HTTP/1.1\r\nHost: localhost\r\nConnection: close\r\n\r\n",
            )
            .await?;
        Ok::<_, std::io::Error>(writer)
    });

    let mut answer = Vec::new();
    // The server may reset the socket once it closes with bytes unread,
    // so a read error ends the answer; what it wrote before is what counts.
    let _read = tokio::time::timeout(Duration::from_secs(10), reader.read_to_end(&mut answer))
        .await
        .expect("the server answers or closes");
    let _sent = sent.await;

    let answer = String::from_utf8_lossy(&answer).to_ascii_lowercase();
    let first = answer.split("\r\n\r\n").next().unwrap_or_default();
    assert!(
        first.starts_with("http/1.1 404"),
        "an unserved path is answered 404: {answer}"
    );
    let second_answered = answer.matches("http/1.1 ").count() > 1;
    assert!(
        second_answered || first.contains("\r\nconnection: close"),
        "the next request is answered or the 404 says the connection closes: {answer}"
    );
}
