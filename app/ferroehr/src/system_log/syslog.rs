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

use ferrohealth_server::tls_client::TlsClient;
use ihe_atna::atna::forwarder::Forwarder;
use ihe_atna::atna::repository::{Repository, Timeouts};
use ihe_atna::atna::spool::{Bounds, Spool};
use ihe_atna::atna::syslog::Sender;
use jiff::Timestamp;
use secrecy::SecretSlice;

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
    /// of the length RFC 5424 §6.2 allows, the `[audit.syslog.tls]` material
    /// cannot be read or is refused, `ihe-atna` refuses the address, or the
    /// spool directory cannot be opened.
    pub async fn new(
        config: &SyslogConfig,
        hostname: &str,
        app_name: &str,
    ) -> Result<Self, AuditError> {
        let hostname = if hostname.is_empty() { "-" } else { hostname };
        let sender = Sender::new(hostname, app_name, PROCID)
            .map_err(|e| AuditError::Transport(Box::new(e)))?;
        let address = config
            .address()
            .map_err(|e| AuditError::Transport(e.into()))?;
        let section = config.tls.clone();
        let tls = tokio::task::spawn_blocking(move || {
            TlsClient::read("audit.syslog.tls", &section)
                .and_then(|client| client.platform_config())
        })
        .await
        .map_err(|e| AuditError::Transport(Box::new(e)))?
        .map_err(|e| AuditError::Transport(Box::new(e)))?;
        let repository = Repository::tls(&address, Arc::new(tls), TIMEOUTS)
            .map_err(|e| AuditError::Transport(Box::new(e)))?;
        let bounds = Bounds {
            max_messages: config.spool_max_events,
            max_bytes: config.spool_max_size.bytes(),
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
        let forwarder = Forwarder::new(spool, repository, config.retry.max_backoff.get());
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

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret as _;

    use super::*;
    use crate::system_log::access_binding::tests::{dicom_details, system};
    use crate::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
    use crate::system_log::message::{AuditContext, to_xml};

    /// The ITI-20 syslog frame carries the DICOM message with its binding
    /// objects (ITI TF-2 §3.20.4.1.2: the message is the MSG).
    #[test]
    fn the_syslog_frame_carries_the_binding_details() {
        let mut event = AuditEvent::new(
            EventActionCode::Read,
            ObjectClass::Composition,
            EventOutcome::Success,
        );
        event.user_id = "client-1".to_owned();
        event.binding = Some(system());
        let ctx = AuditContext {
            source_id: "ferroehr".to_owned(),
            value_if_missing: "UNKNOWN".to_owned(),
            ..AuditContext::default()
        };
        let xml = to_xml(&event, &ctx, Some("patient-42")).expect("xml");
        let sender = Sender::new("ferroehr.example.org", "ferroehr", PROCID).expect("a sender");
        let frame = sender.frame(Timestamp::UNIX_EPOCH, &xml);
        let text = String::from_utf8(frame.expose_secret().to_vec()).expect("UTF-8");
        assert!(text.contains("IHE+RFC-3881"), "{text}");
        assert!(
            dicom_details(&text, "ehds-accessor").contains(&(
                "acts-for".to_owned(),
                "urn:oid:2.999.7|provider-1".to_owned()
            )),
            "{text}"
        );
    }
}
