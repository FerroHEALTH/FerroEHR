// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-20 syslog route (`[audit.syslog]`), sent through `ihe-atna`.
//!
//! The shared crate frames the DICOM audit message as the RFC 5424 message ITI
//! TF-2 §3.20.4.1.2 fixes (PRI `<85>`, MSGID `IHE+RFC-3881`, the XML as MSG
//! with no byte order mark), counts its octets for RFC 5425 §4.3, and writes
//! it to the Audit Record Repository over TLS 1.2 or later. Every frame is
//! stored in the crate's spool first and delivered from it by the crate's
//! forwarder (ITI TF-2 §3.20.4.1.1), which keeps one connection, retries
//! with backoff and closes with `close_notify`. The local audit store keeps
//! only the FHIR document, so the spool, not the store, holds what syslog has
//! still to send.

use std::sync::Arc;
use std::time::Duration;

use ihe_atna::atna::forwarder::Forwarder;
use ihe_atna::atna::repository::{Repository, Timeouts, TlsSettings};
use ihe_atna::atna::spool::{Bounds, Spool};
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

/// How long storing one frame in the spool may take; no specification
/// governs it: our own design.
const SPOOL_WRITE_TIMEOUT: Duration = Duration::from_secs(2);

/// The RFC 5424 `PROCID`, the NILVALUE: the process id names no audited fact.
const PROCID: &str = "-";

/// The syslog route to the Audit Record Repository: the frame builder and
/// the forwarder that delivers from the spool.
#[derive(Debug)]
pub struct SyslogRoute {
    sender: Sender,
    forwarder: Arc<Forwarder>,
}

impl SyslogRoute {
    /// The route `config` names, writing as `hostname` (the server's
    /// `NetworkAccessPointID`, `-` when unknown) and `app_name` (the audit
    /// source id), its spool in `config.spool_dir` or in memory.
    ///
    /// # Errors
    /// [`AuditError::Transport`] when a header field is not printable US-ASCII
    /// of the length RFC 5424 §6.2 allows, the trust or identity files cannot
    /// be read, `ihe-atna` refuses the address or the PEM material, or the
    /// spool directory cannot be opened.
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
        let bounds = Bounds {
            max_messages: config.spool_max_messages,
            max_bytes: config.spool_max_bytes,
            write_timeout: SPOOL_WRITE_TIMEOUT,
        };
        let spool = if let Some(directory) = &config.spool_dir {
            Spool::open(directory, bounds).map_err(|e| AuditError::Transport(Box::new(e)))?
        } else {
            tracing::warn!(
                "ATNA audit syslog spool is held in memory: a restart loses the records \
                 not yet delivered (set [audit.syslog] spool_dir)"
            );
            Spool::in_memory(bounds)
        };
        let forwarder = Forwarder::new(
            spool,
            repository,
            Duration::from_millis(config.retry_max_ms),
        );
        Ok(Self { sender, forwarder })
    }

    /// The forwarder delivering this route's spool, for the caller to run,
    /// stop and read.
    #[must_use]
    pub fn forwarder(&self) -> Arc<Forwarder> {
        Arc::clone(&self.forwarder)
    }

    /// Stores the audit message `xml`, written at `at`, in the spool; the
    /// forwarder delivers it.
    ///
    /// # Errors
    /// [`AuditError::Transport`] when the spool is full or cannot be written:
    /// the frame is then not stored and syslog never receives it.
    pub async fn submit(&self, at: Timestamp, xml: &SecretSlice<u8>) -> Result<(), AuditError> {
        let frame = self.sender.frame(at, xml);
        self.forwarder
            .submit(frame)
            .await
            .map_err(|e| AuditError::Transport(Box::new(e)))
    }
}

/// The bytes of the PEM file at `path`.
async fn read(path: &str) -> Result<Vec<u8>, AuditError> {
    tokio::fs::read(path)
        .await
        .map_err(|e| AuditError::Transport(Box::new(e)))
}
