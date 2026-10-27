// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An in-process HTTPS server for the outbound TLS tests: a rustls listener
//! that presents a leaf its own test CA signed and answers every request with
//! one fixed JSON body.
//!
//! The PKI is the one [`crate::syslog_tls`] mints (`CN=ferroehr-test-ca`, a
//! `localhost` leaf valid from yesterday to 30 days from today), written to a
//! CA file a client names as its `ca_file`. A client that trusts only the
//! platform roots fails the handshake, which is the other half of every test
//! that uses it. No openEHR spec governs test infrastructure: our own
//! design/extension.

use std::path::PathBuf;

use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::TcpListener;
use tokio_rustls::TlsAcceptor;

use crate::syslog_tls::{ListenerError, Pki};

/// The most request bytes read before the answer is sent.
const REQUEST_LIMIT: usize = 64 * 1024;

/// A running HTTPS server on `127.0.0.1`.
#[derive(Debug)]
pub struct HttpsServer {
    /// The base URL, `https://localhost:<port>`.
    pub base_url: String,
    /// A file holding the minted CA certificate, PEM.
    pub ca_file: PathBuf,
}

impl HttpsServer {
    /// Binds a server on a free port that answers every request with `200`
    /// and `body` as `content_type`.
    ///
    /// # Errors
    /// [`ListenerError`] when the PKI is unusable, the port cannot be bound or
    /// the CA file cannot be written.
    pub async fn answering(
        content_type: &'static str,
        body: String,
    ) -> Result<Self, ListenerError> {
        Self::answering_with(content_type, |_base_url| body).await
    }

    /// Binds a server like [`Self::answering`] whose body `body` builds from
    /// the server's own base URL, for a document that names its origin (an
    /// authorization server's metadata names its `issuer`).
    ///
    /// # Errors
    /// [`ListenerError`] when the PKI is unusable, the port cannot be bound or
    /// the CA file cannot be written.
    pub async fn answering_with(
        content_type: &'static str,
        body: impl FnOnce(&str) -> String,
    ) -> Result<Self, ListenerError> {
        let pki = Pki::mint()?;
        let acceptor = TlsAcceptor::from(pki.server_config()?);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        // NOTE: the bound port and the process id make the file one server's own.
        let ca_file = std::env::temp_dir().join(format!(
            "ferroehr-test-https-ca-{}-{port}.pem",
            std::process::id()
        ));
        tokio::fs::write(&ca_file, pki.ca_pem.as_bytes()).await?;
        let base_url = format!("https://localhost:{port}");
        let body = body(&base_url);
        let response = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: {content_type}\r\ncontent-length: {}\r\n\
             connection: close\r\n\r\n{body}",
            body.len()
        );
        drop(tokio::spawn(async move {
            while let Ok((tcp, _peer)) = listener.accept().await {
                let acceptor = acceptor.clone();
                let response = response.clone();
                drop(tokio::spawn(async move {
                    let Ok(mut tls) = acceptor.accept(tcp).await else {
                        return;
                    };
                    if read_head(&mut tls).await
                        && tls.write_all(response.as_bytes()).await.is_ok()
                        && tls.shutdown().await.is_err()
                    {
                        tracing::debug!("https test server: the client left before close_notify");
                    }
                }));
            }
        }));
        Ok(Self { base_url, ca_file })
    }
}

/// Reads until the end of the request head, `\r\n\r\n`; `false` when the
/// stream ends or errs first, or the head exceeds [`REQUEST_LIMIT`].
async fn read_head<S: tokio::io::AsyncRead + Unpin>(stream: &mut S) -> bool {
    let mut seen: Vec<u8> = Vec::new();
    let mut chunk = [0_u8; 4096];
    while seen.len() < REQUEST_LIMIT {
        match stream.read(&mut chunk).await {
            Ok(0) | Err(_) => return false,
            Ok(read) => seen.extend(chunk.iter().take(read)),
        }
        if seen.windows(4).any(|window| window == b"\r\n\r\n") {
            return true;
        }
    }
    false
}
