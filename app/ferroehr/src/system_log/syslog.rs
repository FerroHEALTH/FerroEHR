// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The ITI-20 syslog route (`[audit.syslog]`), sent through `ihe-atna`.
//!
//! The shared crate frames the DICOM audit message as the RFC 5424 message ITI
//! TF-2 §3.20.4.1.2 fixes (PRI `<85>`, MSGID `IHE+RFC-3881`, the XML as MSG
//! with no byte order mark), counts its octets for RFC 5425 §4.3, and writes
//! it to the Audit Record Repository over TLS 1.2 or later. The crate's
//! forwarder delivers from the local store's syslog outbox (ITI TF-2
//! §3.20.4.1.1), rendering each stored record's message when it reads it, so
//! a record the repository has not taken waits in the store, never in memory.

use std::sync::Arc;
use std::time::Duration;

use ferrohealth_server::tls_client::TlsClient;
use ihe_atna::atna::forwarder::Forwarder;
use ihe_atna::atna::repository::{Repository, Timeouts};
use ihe_atna::atna::syslog::Sender;
use tokio::sync::Notify;

use crate::system_log::AuditError;
use crate::system_log::config::SyslogConfig;
use crate::system_log::feed::StoreOutbox;
use crate::system_log::store::AuditStore;

/// How long opening the connection may take, and each TLS handshake, write
/// and flush after it.
///
/// No specification governs the bound: our own design.
const TIMEOUTS: Timeouts = Timeouts {
    connect: Duration::from_secs(10),
    send: Duration::from_secs(10),
};

/// How long the forwarder waits for the chainer's wake before it reads the
/// outbox again; no specification governs it: our own design.
const IDLE: Duration = Duration::from_secs(2);

/// The RFC 5424 `PROCID`, the NILVALUE: the process id names no audited fact.
const PROCID: &str = "-";

/// The forwarder of the syslog route `config` names, writing as `hostname`
/// (the server's `NetworkAccessPointID`, `-` when unknown) and `app_name`
/// (the audit source id), delivering from `store`'s syslog outbox and woken
/// by `notify`.
///
/// # Errors
/// [`AuditError::Transport`] when a header field is not printable US-ASCII
/// of the length RFC 5424 §6.2 allows, the `[audit.syslog.tls]` material
/// cannot be read or is refused, or `ihe-atna` refuses the address.
pub(super) async fn forwarder(
    config: &SyslogConfig,
    hostname: &str,
    app_name: &str,
    store: AuditStore,
    notify: Arc<Notify>,
) -> Result<Arc<Forwarder<StoreOutbox>>, AuditError> {
    let hostname = if hostname.is_empty() { "-" } else { hostname };
    let framer =
        Sender::new(hostname, app_name, PROCID).map_err(|e| AuditError::Transport(Box::new(e)))?;
    let repository = repository(config).await?;
    let outbox = StoreOutbox::syslog(store, framer, notify, IDLE);
    Ok(Forwarder::syslog_from(
        outbox,
        repository,
        config.retry.max_backoff.get(),
    ))
}

/// The Audit Record Repository `config` names, reached over TLS with the
/// `[audit.syslog.tls]` material.
///
/// # Errors
/// [`AuditError::Transport`] when the address is no `tls://host:port` URL or
/// the TLS material cannot be read or is refused.
async fn repository(config: &SyslogConfig) -> Result<Repository, AuditError> {
    let address = config
        .address()
        .map_err(|e| AuditError::Transport(e.into()))?;
    let section = config.tls.clone();
    let tls = tokio::task::spawn_blocking(move || {
        TlsClient::read("audit.syslog.tls", &section).and_then(|client| client.platform_config())
    })
    .await
    .map_err(|e| AuditError::Transport(Box::new(e)))?
    .map_err(|e| AuditError::Transport(Box::new(e)))?;
    Repository::tls(&address, Arc::new(tls), TIMEOUTS)
        .map_err(|e| AuditError::Transport(Box::new(e)))
}

#[cfg(test)]
mod tests {
    use secrecy::ExposeSecret as _;

    use super::*;
    use crate::system_log::access_binding::tests::{dicom_details, system};
    use crate::system_log::event::{AuditEvent, EventActionCode, EventOutcome, ObjectClass};
    use crate::system_log::message::{AuditContext, to_xml};
    use jiff::Timestamp;

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

    /// The group the repository connection agreed on with `peer`, whose
    /// certificate no root trusts, so the connection ends after the key exchange.
    async fn agreed_by_the_repository(peer: &testkit::kx::KxPeer) -> Option<rustls::NamedGroup> {
        let config = SyslogConfig {
            url: ferrohealth_secret::secret_url::SecretUrl::new(format!(
                "tls://{}",
                peer.address()
            )),
            ..SyslogConfig::default()
        };
        let repository = repository(&config).await.expect("the repository builds");
        assert!(
            repository.connect().await.is_err(),
            "the peer's certificate is trusted by no root"
        );
        peer.agreed(0).expect("the connection is recorded")
    }

    /// The ARR connection prefers X25519MLKEM768 (RFC 10024) and agrees on
    /// X25519 with a repository that lacks it.
    #[tokio::test]
    async fn the_repository_connection_prefers_x25519mlkem768_and_falls_back_to_x25519() {
        use rustls::crypto::aws_lc_rs::kx_group;
        let hybrid = testkit::kx::KxPeer::start(vec![kx_group::X25519, kx_group::X25519MLKEM768])
            .expect("the peer starts");
        assert_eq!(
            Some(rustls::NamedGroup::X25519MLKEM768),
            agreed_by_the_repository(&hybrid).await
        );
        let classical =
            testkit::kx::KxPeer::start(vec![kx_group::X25519]).expect("the peer starts");
        assert_eq!(
            Some(rustls::NamedGroup::X25519),
            agreed_by_the_repository(&classical).await
        );
    }
}
