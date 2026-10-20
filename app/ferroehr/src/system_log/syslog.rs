// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-20 syslog sink (`[audit.syslog]`), sent through `ihe-atna`.
//!
//! The shared crate frames the DICOM audit message as the RFC 5424 message ITI
//! TF-2 §3.20.4.1.2 fixes (PRI `<85>`, MSGID `IHE+RFC-3881`, the XML as MSG
//! with no byte order mark), counts its octets for RFC 5425 §4.3, and writes
//! it to the Audit Record Repository over TLS 1.2 or later. FerroEHR keeps one
//! connection open, opens a new one when it fails, and says which repository
//! to reach with which trust and identity.

use std::time::Duration;

use ihe_atna::atna::repository::{Connection, Repository, Timeouts, TlsSettings};
use ihe_atna::atna::syslog::Sender;
use jiff::Timestamp;
use secrecy::{SecretSlice, SecretString};

use crate::system_log::AuditError;
use crate::system_log::config::SyslogConfig;

/// How long opening the connection may take, and each TLS handshake, write
/// and flush after it.
///
/// No specification governs the bound: our own design.
const TIMEOUTS: Timeouts = Timeouts {
    connect: Duration::from_secs(10),
    send: Duration::from_secs(10),
};

/// The RFC 5424 `PROCID`, the NILVALUE: the process id names no audited fact.
const PROCID: &str = "-";

/// The open syslog route to the Audit Record Repository.
#[derive(Debug)]
pub struct SyslogSink {
    sender: Sender,
    repository: Repository,
    // NOTE: `ihe_atna` 0.1.0's `Connection` is `Send` but not `Sync`; the lock lets the drain share the sink.
    connection: tokio::sync::Mutex<Option<Connection>>,
}

impl SyslogSink {
    /// The sink `config` names, writing as `hostname` (the server's
    /// `NetworkAccessPointID`, `-` when unknown) and `app_name` (the audit
    /// source id).
    ///
    /// # Errors
    /// [`AuditError::Transport`] when a header field is not printable US-ASCII
    /// of the length RFC 5424 §6.2 allows, the trust or identity files cannot
    /// be read, or `ihe-atna` refuses the address or the PEM material.
    pub async fn new(
        config: &SyslogConfig,
        hostname: &str,
        app_name: &str,
    ) -> Result<Self, AuditError> {
        let hostname = if hostname.is_empty() { "-" } else { hostname };
        let sender = Sender::new(hostname, app_name, PROCID)
            .map_err(|e| AuditError::Transport(Box::new(e)))?;
        let address = url::Url::parse(&format!("tls://{}:{}", config.host, config.port))
            .map_err(|e| AuditError::Transport(Box::new(e)))?;
        let roots = match config.tls_ca_file.as_deref() {
            Some(path) => Some(read(path).await?),
            None => None,
        };
        let identity = match (
            config.tls_identity_cert_file.as_deref(),
            config.tls_identity_key_file.as_deref(),
        ) {
            (Some(cert), Some(key)) => {
                let mut pem = read(cert).await?;
                pem.push(b'\n');
                pem.extend(read(key).await?);
                let pem = String::from_utf8(pem).map_err(|e| AuditError::Transport(Box::new(e)))?;
                Some(SecretString::from(pem))
            }
            _ => None,
        };
        let repository = Repository::tls(&address, &TlsSettings { roots, identity }, TIMEOUTS)
            .map_err(|e| AuditError::Transport(Box::new(e)))?;
        Ok(Self {
            sender,
            repository,
            connection: tokio::sync::Mutex::new(None),
        })
    }

    /// Sends the audit message `xml`, written at `at`, over the held
    /// connection, opening one first when none is held or it has closed.
    ///
    /// # Errors
    /// [`AuditError::Transport`] when the repository cannot be reached or the
    /// frame cannot be written; the connection is dropped, and the next send
    /// opens a new one.
    pub async fn send(&self, at: Timestamp, xml: &SecretSlice<u8>) -> Result<(), AuditError> {
        let frame = self.sender.frame(at, xml);
        let mut slot = self.connection.lock().await;
        let held = match slot.take() {
            Some(mut held) => (!held.is_closed().await).then_some(held),
            None => None,
        };
        let mut connection = match held {
            Some(open) => open,
            None => self
                .repository
                .connect()
                .await
                .map_err(|e| AuditError::Transport(Box::new(e)))?,
        };
        connection
            .send(&frame)
            .await
            .map_err(|e| AuditError::Transport(Box::new(e)))?;
        *slot = Some(connection);
        Ok(())
    }
}

/// The bytes of the PEM file at `path`.
async fn read(path: &str) -> Result<Vec<u8>, AuditError> {
    tokio::fs::read(path)
        .await
        .map_err(|e| AuditError::Transport(Box::new(e)))
}
