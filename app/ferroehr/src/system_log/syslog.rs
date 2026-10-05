// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Syslog assembly (RFC 5424) + transports (RFC 5426 UDP / RFC 5425 TLS).
//!
//! The DICOM Audit Message XML is carried as the RFC 5424 `MSG` field. IHE ATNA
//! ships records over syslog; we frame per the RFCs:
//!
//! - **RFC 5424** — the `SYSLOG-MSG` header/structure.
//! - **RFC 5426** — one UDP datagram per `SYSLOG-MSG` (no extra framing).
//! - **RFC 5425** — TLS transport with **octet-counting** framing
//!   (`MSG-LEN SP SYSLOG-MSG`), the mandatory TLS framing (RFC 5425 §4.3).
//!
//! Header field choices not fixed by RFC 5424 follow the IHE ATNA convention
//! (recorded inline with the citing RFC section).

use std::io;
use std::sync::Arc;

use jiff::Timestamp;
use tokio::io::AsyncWriteExt;
use tokio::net::{TcpStream, UdpSocket};
use tokio_rustls::TlsConnector;

use crate::system_log::config::SyslogConfig;

// PRI = facility*8 + severity (RFC 5424 §6.2.1). IHE ATNA logs security/audit
// events: facility 10 ("security/authorization messages") and severity 5
// ("Notice"). NOTE: RFC 5424 §6.2.1 does not fix the ATNA severity; 5
// (Notice) is the common IHE choice for a normal audit record — review if the
// deployment expects a different priority.
/// Syslog facility 10 — security/authorization messages (RFC 5424 §6.2.1).
pub const SYSLOG_FACILITY: u8 = 10;
/// Syslog severity 5 — Notice (RFC 5424 §6.2.1).
pub const SYSLOG_SEVERITY: u8 = 5;
/// Computed PRI value (`10*8 + 5 = 85`).
#[expect(
    clippy::as_conversions,
    reason = "u8 → u16 widening is exact; `From` is not usable here because it is not \
              yet stable as a const trait (https://doc.rust-lang.org/reference/const_eval.html)"
)]
pub const SYSLOG_PRI: u16 = (SYSLOG_FACILITY as u16) * 8 + SYSLOG_SEVERITY as u16;
/// RFC 5424 SYSLOG version.
pub const SYSLOG_VERSION: u8 = 1;
/// The IHE ATNA `MSGID` for an audit record.
// NOTE: IHE ITI TF-2 ITI-20 §3.20.4.1.2 — MSGID "shall be set to
// 'IHE+RFC-3881'", uniform for every ITI-20 message even though the MSG
// payload is DICOM PS3.15 §A.5 XML (the token is a back-compat retention).
pub const SYSLOG_MSGID: &str = "IHE+RFC-3881";
/// UTF-8 byte-order mark that RFC 5424 §6.4 recommends prefixing a UTF-8 `MSG`.
pub const UTF8_BOM: &[u8] = &[0xEF, 0xBB, 0xBF];

/// Assemble an RFC 5424 `SYSLOG-MSG` carrying the DICOM Audit Message XML.
///
/// - `HOSTNAME` = `hostname` (or `-` when empty; RFC 5424 §6.2.4 NILVALUE).
/// - `APP-NAME` = `app_name` (the audit source id; RFC 5424 §6.2.5).
/// - `PROCID` = `-`, `STRUCTURED-DATA` = `-` (NILVALUE).
/// - `MSG` = UTF-8 BOM + the XML.
#[must_use]
pub fn assemble_syslog(
    hostname: &str,
    app_name: &str,
    timestamp: &Timestamp,
    msg_xml: &str,
) -> Vec<u8> {
    let host = nilvalue(hostname);
    let app = nilvalue(app_name);
    // RFC 5424 HEADER: PRI VERSION SP TIMESTAMP SP HOSTNAME SP APP-NAME SP
    // PROCID SP MSGID SP STRUCTURED-DATA SP MSG.
    let header =
        format!("<{SYSLOG_PRI}>{SYSLOG_VERSION} {timestamp} {host} {app} - {SYSLOG_MSGID} - ");
    let mut out = Vec::with_capacity(header.len() + UTF8_BOM.len() + msg_xml.len());
    out.extend_from_slice(header.as_bytes());
    out.extend_from_slice(UTF8_BOM);
    out.extend_from_slice(msg_xml.as_bytes());
    out
}

/// RFC 5425 §4.3 octet-counting framing: `MSG-LEN SP SYSLOG-MSG`.
#[must_use]
pub fn frame_octet_counting(syslog_msg: &[u8]) -> Vec<u8> {
    let prefix = format!("{} ", syslog_msg.len());
    let mut out = Vec::with_capacity(prefix.len() + syslog_msg.len());
    out.extend_from_slice(prefix.as_bytes());
    out.extend_from_slice(syslog_msg);
    out
}

fn nilvalue(s: &str) -> &str {
    if s.is_empty() { "-" } else { s }
}

/// A live syslog transport to the ARR, owned by the drain task.
#[derive(Debug)]
pub enum Transport {
    /// RFC 5426 UDP (a connected datagram socket).
    Udp(UdpTransport),
    /// RFC 5425 TLS (a lazily (re)connected octet-counted stream).
    Tls(Box<TlsTransport>),
}

impl Transport {
    /// Build the transport the [`SyslogConfig`] selects.
    ///
    /// # Errors
    /// [`io::Error`] on a UDP bind/connect failure or an invalid TLS config
    /// (missing CA, unreadable identity, bad server name).
    pub async fn connect(config: &SyslogConfig) -> io::Result<Self> {
        match config.transport {
            crate::system_log::config::Transport::Udp => Ok(Transport::Udp(
                UdpTransport::connect(&config.host, config.port).await?,
            )),
            crate::system_log::config::Transport::Tls => {
                Ok(Transport::Tls(Box::new(TlsTransport::from_config(config)?)))
            }
        }
    }

    /// Frame + send one assembled `SYSLOG-MSG`.
    ///
    /// # Errors
    /// [`io::Error`] on a socket write / TLS handshake failure.
    pub async fn send(&mut self, syslog_msg: &[u8]) -> io::Result<()> {
        match self {
            // RFC 5426: the datagram IS the SYSLOG-MSG (no octet counting).
            Transport::Udp(u) => u.send(syslog_msg).await,
            // RFC 5425: octet-counted framing over the TLS stream.
            Transport::Tls(t) => t.send(&frame_octet_counting(syslog_msg)).await,
        }
    }
}

/// A connected UDP datagram transport (RFC 5426).
#[derive(Debug)]
pub struct UdpTransport {
    socket: UdpSocket,
}

impl UdpTransport {
    /// Bind an ephemeral local socket and connect it to the ARR.
    ///
    /// # Errors
    /// [`io::Error`] if the socket cannot bind or the address cannot resolve.
    pub async fn connect(host: &str, port: u16) -> io::Result<Self> {
        let socket = UdpSocket::bind(("0.0.0.0", 0)).await?;
        socket.connect((host, port)).await?;
        Ok(Self { socket })
    }

    /// Send one datagram.
    ///
    /// # Errors
    /// [`io::Error`] on a send failure.
    pub async fn send(&self, datagram: &[u8]) -> io::Result<()> {
        self.socket.send(datagram).await?;
        Ok(())
    }
}

/// A TLS octet-counted stream transport (RFC 5425), reconnected on demand.
pub struct TlsTransport {
    connector: TlsConnector,
    server_name: rustls::pki_types::ServerName<'static>,
    host: String,
    port: u16,
    stream: Option<tokio_rustls::client::TlsStream<TcpStream>>,
}

impl std::fmt::Debug for TlsTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TlsTransport")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("connected", &self.stream.is_some())
            .finish_non_exhaustive()
    }
}

impl TlsTransport {
    /// Build from configuration (loads the CA + optional client identity PEM).
    ///
    /// # Errors
    /// [`io::Error`] if the TLS client config is invalid or `tls_ca_file` is
    /// unset/unreadable.
    pub fn from_config(config: &SyslogConfig) -> io::Result<Self> {
        let client_config = tls_client_config(config)?;
        Self::new(Arc::new(client_config), &config.host, config.port)
    }

    /// Build from a prebuilt rustls [`rustls::ClientConfig`] (used by tests).
    ///
    /// # Errors
    /// [`io::Error`] if the host is not a valid TLS server name.
    pub fn new(
        client_config: Arc<rustls::ClientConfig>,
        host: &str,
        port: u16,
    ) -> io::Result<Self> {
        let server_name = rustls::pki_types::ServerName::try_from(host.to_owned())
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
        Ok(Self {
            connector: TlsConnector::from(client_config),
            server_name,
            host: host.to_owned(),
            port,
            stream: None,
        })
    }

    async fn ensure_connected(&mut self) -> io::Result<()> {
        if self.stream.is_none() {
            let tcp = TcpStream::connect((self.host.as_str(), self.port)).await?;
            let tls = self
                .connector
                .connect(self.server_name.clone(), tcp)
                .await?;
            self.stream = Some(tls);
        }
        Ok(())
    }

    /// Send a pre-framed record, connecting/reconnecting as needed.
    ///
    /// # Errors
    /// [`io::Error`] on handshake/write failure (the stream is dropped so the
    /// next send reconnects).
    pub async fn send(&mut self, framed: &[u8]) -> io::Result<()> {
        self.ensure_connected().await?;
        let Some(stream) = self.stream.as_mut() else {
            return Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "TLS stream unavailable after connect",
            ));
        };
        if let Err(e) = write_all_flush(stream, framed).await {
            // Drop the broken stream so the next attempt reconnects.
            self.stream = None;
            return Err(e);
        }
        Ok(())
    }
}

async fn write_all_flush(
    stream: &mut tokio_rustls::client::TlsStream<TcpStream>,
    bytes: &[u8],
) -> io::Result<()> {
    stream.write_all(bytes).await?;
    stream.flush().await?;
    Ok(())
}

/// Build a rustls [`rustls::ClientConfig`] from the audit TLS settings.
///
/// Requires `tls_ca_file` (IHE nodes are mutually authenticated against an
/// explicit trust anchor, not the public web PKI). Adds a client certificate
/// when `tls_identity_*` are set.
///
/// # Errors
/// [`io::Error`] if the CA path is unset/unreadable, contains no certificates,
/// or the client identity is invalid.
pub fn tls_client_config(config: &SyslogConfig) -> io::Result<rustls::ClientConfig> {
    let ca_path = config.tls_ca_file.as_ref().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "audit.syslog.tls_ca_file is required for TLS transport",
        )
    })?;
    let ca_pem = std::fs::read(ca_path)?;
    let mut roots = rustls::RootCertStore::empty();
    add_roots(&mut roots, &ca_pem)?;

    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let builder = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
        .with_root_certificates(roots);

    let config = match (
        &config.tls_identity_cert_file,
        &config.tls_identity_key_file,
    ) {
        (Some(cert_path), Some(key_path)) => {
            let certs = load_certs(&std::fs::read(cert_path)?)?;
            let key = load_key(&std::fs::read(key_path)?)?;
            builder
                .with_client_auth_cert(certs, key)
                .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?
        }
        _ => builder.with_no_client_auth(),
    };
    Ok(config)
}

/// Add every PEM certificate in `pem` to the root store.
///
/// # Errors
/// [`io::Error`] if the PEM contains no certificate or a certificate is rejected.
pub fn add_roots(roots: &mut rustls::RootCertStore, pem: &[u8]) -> io::Result<()> {
    let certs = load_certs(pem)?;
    for cert in certs {
        roots
            .add(cert)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    }
    Ok(())
}

fn load_certs(pem: &[u8]) -> io::Result<Vec<rustls::pki_types::CertificateDer<'static>>> {
    use rustls::pki_types::pem::PemObject;
    let certs: Vec<_> = rustls::pki_types::CertificateDer::pem_slice_iter(pem)
        .collect::<Result<_, _>>()
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
    if certs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "no certificates found in PEM",
        ));
    }
    Ok(certs)
}

fn load_key(pem: &[u8]) -> io::Result<rustls::pki_types::PrivateKeyDer<'static>> {
    use rustls::pki_types::pem::PemObject;
    rustls::pki_types::PrivateKeyDer::from_pem_slice(pem)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pri_is_85() {
        assert_eq!(SYSLOG_PRI, 85);
    }

    #[test]
    fn assembles_rfc5424_header() {
        let ts: Timestamp = "2026-07-06T12:00:00Z".parse().unwrap();
        let msg = assemble_syslog("cdr-01", "ferroehr", &ts, "<AuditMessage/>");
        let text = String::from_utf8_lossy(&msg);
        assert!(text.starts_with("<85>1 2026-07-06T12:00:00Z cdr-01 ferroehr - IHE+RFC-3881 -"));
        // BOM precedes the XML.
        assert!(msg.windows(3).any(|w| w == UTF8_BOM));
        assert!(text.contains("<AuditMessage/>"));
    }

    #[test]
    fn nilvalue_for_empty_hostname() {
        let ts: Timestamp = "2026-07-06T12:00:00Z".parse().unwrap();
        let msg = assemble_syslog("", "ferroehr", &ts, "<x/>");
        let text = String::from_utf8_lossy(&msg);
        assert!(text.starts_with("<85>1 2026-07-06T12:00:00Z - ferroehr - IHE+RFC-3881 -"));
    }

    #[test]
    fn octet_counting_frames_length() {
        let payload = b"hello";
        let framed = frame_octet_counting(payload);
        assert_eq!(framed, b"5 hello");
    }
}
