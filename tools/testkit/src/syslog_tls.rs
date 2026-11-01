// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! An in-process syslog-over-TLS Audit Record Repository for the audit tests:
//! a rustls listener that presents a test leaf its test CA signed, and hands
//! over every RFC 5425 §4.3 frame it reads.
//!
//! Both certificates are minted when the listener binds (`rcgen`): a CA
//! (`CN=ferroehr-test-ca`) and a leaf (`CN=localhost`, SAN `localhost` and
//! `127.0.0.1`, `serverAuth`) it signed, each valid from yesterday to
//! 30 days from today. A platform verifier refuses a TLS server
//! certificate valid for years (macOS caps it at 825 days), and nothing
//! committed can expire. No openEHR spec governs test infrastructure: our own
//! design/extension.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use tokio::io::AsyncReadExt as _;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_rustls::TlsAcceptor;

/// How many days past today the minted certificates stay valid.
const VALIDITY_DAYS: i64 = 30;

/// Why the listener could not start.
#[derive(Debug, thiserror::Error)]
pub enum ListenerError {
    /// The test PKI did not read as PEM, or rustls refused it.
    #[error("the test PKI is unusable")]
    Pki(#[source] Box<dyn std::error::Error + Send + Sync>),
    /// The listener could not bind, or the CA file could not be written.
    #[error("the listener could not start")]
    Io(#[from] std::io::Error),
}

/// A running listener on `127.0.0.1`.
#[derive(Debug)]
pub struct SyslogTlsListener {
    /// The port it listens on.
    pub port: u16,
    /// A file holding the minted CA certificate, PEM, for
    /// `[audit.syslog] tls_ca_file`.
    pub ca_file: PathBuf,
    frames: mpsc::UnboundedReceiver<Vec<u8>>,
    endings: mpsc::UnboundedReceiver<Ending>,
}

impl SyslogTlsListener {
    /// Binds a listener on a free port and starts accepting connections. It
    /// drops its first `refused` connections before any TLS handshake, as a
    /// repository that is going down does, and accepts every one after them.
    ///
    /// # Errors
    /// [`ListenerError`] when the PKI is unusable, the port cannot be bound or
    /// the CA file cannot be written.
    pub async fn bind(refused: usize) -> Result<Self, ListenerError> {
        let pki = Pki::mint()?;
        let acceptor = TlsAcceptor::from(pki.server_config()?);
        let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
        let port = listener.local_addr()?.port();
        // NOTE: the bound port and the process id make the file one listener's own.
        let ca_file = std::env::temp_dir().join(format!(
            "ferroehr-test-ca-{}-{port}.pem",
            std::process::id()
        ));
        tokio::fs::write(&ca_file, pki.ca_pem.as_bytes()).await?;
        let (tx, frames) = mpsc::unbounded_channel();
        let (ended, endings) = mpsc::unbounded_channel();
        drop(tokio::spawn(async move {
            let mut to_refuse = refused;
            while let Ok((tcp, _peer)) = listener.accept().await {
                if to_refuse > 0 {
                    to_refuse -= 1;
                    drop(tcp);
                    continue;
                }
                let acceptor = acceptor.clone();
                let tx = tx.clone();
                let ended = ended.clone();
                drop(tokio::spawn(async move {
                    match acceptor.accept(tcp).await {
                        Ok(tls) => {
                            if let Some(ending) = read_frames(tls, &tx).await
                                && ended.send(ending).is_err()
                            {
                                tracing::debug!("syslog test listener: no test reads the endings");
                            }
                        }
                        Err(e) => {
                            tracing::warn!("syslog test listener: TLS handshake failed: {e}");
                        }
                    }
                }));
            }
        }));
        Ok(Self {
            port,
            ca_file,
            frames,
            endings,
        })
    }

    /// How the next connection to end ended, or `None` when none ends within
    /// `wait`.
    pub async fn ending(&mut self, wait: Duration) -> Option<Ending> {
        tokio::time::timeout(wait, self.endings.recv())
            .await
            .ok()
            .flatten()
    }

    /// The next whole frame, `MSG-LEN SP SYSLOG-MSG`, or `None` when none
    /// arrives within `wait`.
    pub async fn recv(&mut self, wait: Duration) -> Option<Vec<u8>> {
        tokio::time::timeout(wait, self.frames.recv())
            .await
            .ok()
            .flatten()
    }
}

/// How a sender ended its connection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ending {
    /// With a TLS `close_notify`, as RFC 5425 §4.4 asks of a transport sender.
    CloseNotify,
    /// Without one: the connection was dropped, reset, or failed.
    Abrupt,
}

/// Reads RFC 5425 §4.3 frames off `stream` until it ends, sending each, and
/// returns how it ended; `None` when the test stopped listening first.
async fn read_frames<S: tokio::io::AsyncRead + Unpin>(
    mut stream: S,
    tx: &mpsc::UnboundedSender<Vec<u8>>,
) -> Option<Ending> {
    let mut buffer = Vec::new();
    let mut chunk = [0_u8; 4096];
    loop {
        while let Some(frame) = take_frame(&mut buffer) {
            if tx.send(frame).is_err() {
                return None;
            }
        }
        // NOTE: tokio-rustls reads a close_notify as the end of the stream, and
        // a stream that ends without one as an error.
        match stream.read(&mut chunk).await {
            Ok(0) => return Some(Ending::CloseNotify),
            Err(_) => return Some(Ending::Abrupt),
            Ok(read) => buffer.extend(chunk.iter().take(read)),
        }
    }
}

/// The first whole frame in `buffer`, removed from it.
fn take_frame(buffer: &mut Vec<u8>) -> Option<Vec<u8>> {
    let space = buffer.iter().position(|byte| *byte == b' ')?;
    let length: usize = std::str::from_utf8(buffer.get(..space)?)
        .ok()?
        .parse()
        .ok()?;
    let end = space.checked_add(1)?.checked_add(length)?;
    if buffer.len() < end {
        return None;
    }
    Some(buffer.drain(..end).collect())
}

/// The CA certificate and the leaf it signed, minted for one listener.
pub(crate) struct Pki {
    /// The CA certificate, PEM.
    pub(crate) ca_pem: String,
    /// The leaf certificate, DER.
    leaf: rustls::pki_types::CertificateDer<'static>,
    /// The leaf's private key, PKCS #8 DER.
    leaf_key: Vec<u8>,
}

impl Pki {
    /// Mints the CA and the leaf, valid from yesterday to [`VALIDITY_DAYS`]
    /// from today.
    pub(crate) fn mint() -> Result<Self, ListenerError> {
        use rcgen::{
            BasicConstraints, CertificateParams, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer,
            KeyPair, KeyUsagePurpose,
        };
        let pki = |e: rcgen::Error| ListenerError::Pki(Box::new(e));
        let stamp = |date: jiff::civil::Date| -> Result<_, ListenerError> {
            let part = |e: std::num::TryFromIntError| ListenerError::Pki(Box::new(e));
            Ok(rcgen::date_time_ymd(
                i32::from(date.year()),
                u8::try_from(date.month()).map_err(part)?,
                u8::try_from(date.day()).map_err(part)?,
            ))
        };
        let (first, last) = validity()?;
        let (not_before, not_after) = (stamp(first)?, stamp(last)?);

        let mut ca = CertificateParams::new(Vec::<String>::new()).map_err(pki)?;
        ca.distinguished_name
            .push(DnType::CommonName, "ferroehr-test-ca");
        ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
        ca.not_before = not_before;
        ca.not_after = not_after;
        let ca_key = KeyPair::generate().map_err(pki)?;
        let ca_pem = ca.self_signed(&ca_key).map_err(pki)?.pem();

        let mut leaf = CertificateParams::new(vec!["localhost".to_owned(), "127.0.0.1".to_owned()])
            .map_err(pki)?;
        leaf.distinguished_name
            .push(DnType::CommonName, "localhost");
        leaf.key_usages = vec![
            KeyUsagePurpose::DigitalSignature,
            KeyUsagePurpose::KeyEncipherment,
        ];
        leaf.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
        leaf.not_before = not_before;
        leaf.not_after = not_after;
        let leaf_key = KeyPair::generate().map_err(pki)?;
        let issuer = Issuer::new(ca, ca_key);
        let leaf = leaf.signed_by(&leaf_key, &issuer).map_err(pki)?;
        Ok(Self {
            ca_pem,
            leaf: leaf.der().clone(),
            leaf_key: leaf_key.serialize_der(),
        })
    }

    /// The server configuration presenting the leaf.
    pub(crate) fn server_config(&self) -> Result<Arc<rustls::ServerConfig>, ListenerError> {
        let pki = |e: rustls::Error| ListenerError::Pki(Box::new(e));
        let key = rustls::pki_types::PrivateKeyDer::Pkcs8(self.leaf_key.clone().into());
        let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .map_err(pki)?
            .with_no_client_auth()
            .with_single_cert(vec![self.leaf.clone()], key)
            .map_err(pki)?;
        Ok(Arc::new(config))
    }
}

/// Yesterday and [`VALIDITY_DAYS`] from today, in UTC.
fn validity() -> Result<(jiff::civil::Date, jiff::civil::Date), ListenerError> {
    let date = |e: jiff::Error| ListenerError::Pki(Box::new(e));
    let today = jiff::Timestamp::now()
        .to_zoned(jiff::tz::TimeZone::UTC)
        .date();
    let first = today.yesterday().map_err(date)?;
    let last = today
        .checked_add(jiff::Span::new().days(VALIDITY_DAYS))
        .map_err(date)?;
    Ok((first, last))
}
