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

use ferroehr::config::server::ConnectionConfig;
use tokio::io::AsyncReadExt as _;
use tokio::io::AsyncWriteExt as _;

/// Serve a trivial router on an ephemeral port through the crate's real
/// listener path, returning the bound port.
async fn serve(connection: ConnectionConfig) -> u16 {
    let app = axum::Router::new().route("/ok", axum::routing::get(|| async { "OK" }));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    let port = listener.local_addr().expect("addr").port();
    listener.set_nonblocking(true).expect("nonblocking");

    tokio::spawn(async move {
        let mut server = axum_server::from_tcp(listener)
            .expect("from_tcp")
            .handle(axum_server::Handle::new());
        let builder = server.http_builder();
        // The timer is not optional when a timeout is set — hyper panics per
        // connection without it. This mirrors `run_server`'s own wiring so the
        // test exercises the real configuration rather than a simplified one.
        builder
            .http1()
            .timer(hyper_util::rt::TokioTimer::new())
            .header_read_timeout(connection.header_read_timeout());
        builder
            .http2()
            .timer(hyper_util::rt::TokioTimer::new())
            .max_concurrent_streams(connection.stream_cap());
        let _served = server.serve(app.into_make_service()).await;
    });

    for _ in 0..100 {
        if tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .is_ok()
        {
            return port;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("the listener never came up");
}

/// A connection that opens and trickles a partial request head is dropped once
/// the header-read bound elapses, rather than being held indefinitely.
///
/// The bound is set to one second so the test is quick; the shape is what
/// matters, not the number.
#[tokio::test]
async fn a_stalled_connection_is_dropped_rather_than_held() {
    let port = serve(ConnectionConfig {
        header_read_timeout_secs: 1,
        ..ConnectionConfig::default()
    })
    .await;

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
    let port = serve(ConnectionConfig {
        header_read_timeout_secs: 1,
        ..ConnectionConfig::default()
    })
    .await;

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

/// Zero disables the bound, which must remain expressible: an operator behind a
/// proxy that already enforces one should be able to turn this off.
#[test]
fn zero_disables_the_header_bound() {
    let off = ConnectionConfig {
        header_read_timeout_secs: 0,
        ..ConnectionConfig::default()
    };
    assert!(off.header_read_timeout().is_none());
    assert!(ConnectionConfig::default().header_read_timeout().is_some());
}

/// The HTTP/2 bounds are on by default, because HTTP/2's exposure is stream
/// concurrency rather than a trickled head — the rapid-reset amplification
/// class, which `max_concurrent_streams` is what bounds.
#[test]
fn the_http2_bounds_are_on_by_default() {
    let cfg = ConnectionConfig::default();
    assert!(cfg.stream_cap().is_some());
    assert!(cfg.http2_keep_alive_interval().is_some());
    assert!(cfg.http2_keep_alive_timeout() > Duration::ZERO);
}
