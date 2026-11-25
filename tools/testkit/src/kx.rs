// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! A TLS 1.3 peer that records the key exchange group each client agrees on.
//!
//! It offers only the groups a test names, so the test shows which group a
//! client FerroEHR builds negotiates (RFC 10024, RFC 8446 §4.2.7). [`KxPeer`]
//! listens on a loopback port with a self-signed certificate for `localhost`
//! minted when it starts, answers each completed handshake `204`, and records
//! the group after its first flight, so a client that then refuses the
//! certificate still shows the group it agreed on. No openEHR spec governs
//! test infrastructure: our own design.

use std::io::{Read as _, Write as _};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use rcgen::{CertificateParams, KeyPair};
use rustls::crypto::{CryptoProvider, SupportedKxGroup, aws_lc_rs};
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use rustls::{NamedGroup, ServerConfig, ServerConnection};

/// How long [`KxPeer::agreed`] waits for a connection to be recorded, and
/// how long the peer waits on one read or write.
const RECORD_WAIT: Duration = Duration::from_secs(10);

/// Why the peer could not start.
#[derive(Debug, thiserror::Error)]
pub enum PeerError {
    /// The certificate could not be minted.
    #[error("the peer's certificate could not be minted")]
    Certificate(#[from] rcgen::Error),
    /// rustls refused the configuration.
    #[error("rustls refused the peer's configuration")]
    Config(#[from] rustls::Error),
    /// The loopback port could not be bound.
    #[error("the peer could not bind a loopback port")]
    Io(#[from] std::io::Error),
}

/// A TLS 1.3 peer on a loopback port that offers only the groups it was
/// started with.
#[derive(Debug)]
pub struct KxPeer {
    address: SocketAddr,
    certificate: String,
    agreed: Arc<Mutex<Vec<Option<NamedGroup>>>>,
}

impl KxPeer {
    /// Starts a peer that offers `groups`, in that order of preference.
    ///
    /// # Errors
    /// [`PeerError`] when the certificate, the configuration or the port
    /// cannot be made.
    pub fn start(groups: Vec<&'static dyn SupportedKxGroup>) -> Result<Self, PeerError> {
        let key = KeyPair::generate()?;
        let certificate =
            CertificateParams::new(vec![String::from("localhost")])?.self_signed(&key)?;
        let provider = CryptoProvider {
            kx_groups: groups,
            ..aws_lc_rs::default_provider()
        };
        let config = Arc::new(
            ServerConfig::builder_with_provider(Arc::new(provider))
                .with_protocol_versions(&[&rustls::version::TLS13])?
                .with_no_client_auth()
                .with_single_cert(
                    vec![CertificateDer::from(certificate.der().to_vec())],
                    PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key.serialize_der())),
                )?,
        );
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let agreed = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&agreed);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let group = serve(Arc::clone(&config), stream);
                record
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .push(group);
            }
        });
        Ok(Self {
            address,
            certificate: certificate.pem(),
            agreed,
        })
    }

    /// The peer's address.
    #[must_use]
    pub const fn address(&self) -> SocketAddr {
        self.address
    }

    /// The peer's certificate as PEM, for a client to trust.
    #[must_use]
    pub fn certificate(&self) -> &str {
        &self.certificate
    }

    /// The group of the `index`th connection, `None` when it agreed on none,
    /// waiting until that connection is recorded.
    ///
    /// # Errors
    /// The number of connections recorded when the `index`th was not recorded
    /// in time.
    pub fn agreed(&self, index: usize) -> Result<Option<NamedGroup>, usize> {
        let deadline = Instant::now() + RECORD_WAIT;
        loop {
            let seen = self
                .agreed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            if let Some(group) = seen.get(index) {
                return Ok(*group);
            }
            if Instant::now() >= deadline {
                return Err(seen.len());
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}

/// Drives one connection and returns the group it agreed on; a client that
/// refuses the certificate after the key exchange still yields its group.
fn serve(config: Arc<ServerConfig>, mut stream: TcpStream) -> Option<NamedGroup> {
    let Ok(mut connection) = ServerConnection::new(config) else {
        return None;
    };
    let timeout = Some(RECORD_WAIT);
    if stream.set_read_timeout(timeout).is_err() || stream.set_write_timeout(timeout).is_err() {
        return None;
    }
    let mut handshake = Ok((0, 0));
    while connection.is_handshaking() && handshake.is_ok() {
        handshake = connection.complete_io(&mut stream);
    }
    let group = connection
        .negotiated_key_exchange_group()
        .map(SupportedKxGroup::name);
    if handshake.is_ok() {
        let mut tls = rustls::Stream::new(&mut connection, &mut stream);
        let mut request = [0_u8; 4096];
        if tls.read(&mut request).is_ok() {
            let answered = tls
                .write_all(b"HTTP/1.1 204 No Content\r\nconnection: close\r\n\r\n")
                .and_then(|()| tls.flush());
            if answered.is_ok() {
                tls.conn.send_close_notify();
                let _closed = tls.conn.complete_io(tls.sock);
            }
        }
    }
    group
}
