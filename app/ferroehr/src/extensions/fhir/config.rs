// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[fhir]` section — the FHIR connector (inbound façade + outbound
//! emitter).
//!
//! **No openEHR spec governs this — our own design/extension.** Fields of the
//! one config tree ([`crate::config::FerroEhrConfig`]); no loader of its own.
//! The inbound API façade and the outbound emitter are **independent switches**
//! ([`FhirConfig::api_enabled`] vs [`FhirOutboundConfig::enabled`]).
//!
//! PHI NOTE: unlike the PHI-free event envelopes, the outbound emitter's
//! payload IS the mapped FHIR **resource**, so it carries clinical content by
//! design. It is off by default behind its own explicit flag, and publishes to
//! a SEPARATE [`exchange`](FhirOutboundConfig::exchange) (default
//! `ferroehr.fhir`, distinct from the events exchange) so broker-level access
//! control can restrict the PHI-bearing stream independently. Turning it on is
//! an explicit, audited deployment decision.

use std::path::PathBuf;

use crate::config::secret::SecretUrl;
use serde::{Deserialize, Serialize};

/// The `[fhir]` section: the inbound API façade toggle + the outbound emitter.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FhirConfig {
    /// Mount the `/fhir/r4/*` inbound routes + the `/admin/fhir_mapping` CRUD.
    /// Off by default — the routes answer `404` unless enabled.
    pub api_enabled: bool,
    /// The outbound (PHI-bearing) FHIR emitter.
    pub outbound: FhirOutboundConfig,
}

/// FHIR outbound emitter configuration (`[fhir.outbound]`) — our own extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FhirOutboundConfig {
    /// Master switch. Off by default — this stream carries PHI (the mapped FHIR
    /// resource), so enabling it is an explicit deployment decision.
    pub enabled: bool,
    /// AMQP broker URL (credentials redacted from every rendering).
    pub url: SecretUrl,
    /// Path to a file holding the broker URL, read at boot in place of
    /// [`Self::url`]. Setting both this and a non-default `url` is a boot error.
    pub url_file: Option<PathBuf>,
    /// Topic exchange to publish FHIR resources to; default `ferroehr.fhir`,
    /// distinct from the events exchange for PHI isolation.
    pub exchange: String,
    /// Use TLS: upgrades an `amqp://` URL to `amqps://`.
    pub tls: bool,
    /// Outbox rows scanned per poll.
    pub batch_size: i64,
    /// Poll interval when idle (ms).
    pub poll_interval_ms: u64,
    /// Per-message publish retry count before backing off.
    pub publish_max_retries: usize,
}

impl Default for FhirOutboundConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: SecretUrl::new("amqp://guest:guest@localhost:5672/%2f"),
            url_file: None,
            exchange: "ferroehr.fhir".to_owned(),
            tls: false,
            batch_size: 128,
            poll_interval_ms: 1_000,
            publish_max_retries: 3,
        }
    }
}

impl FhirOutboundConfig {
    /// The effective broker URL, upgraded to `amqps://` when [`Self::tls`] is set
    /// and the URL is a plain `amqp://`.
    #[must_use]
    pub fn effective_url(&self) -> String {
        let url = self.url.expose();
        if self.tls && url.starts_with("amqp://") {
            url.replacen("amqp://", "amqps://", 1)
        } else {
            url.to_owned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_and_use_a_separate_exchange() {
        let c = FhirConfig::default();
        assert!(!c.api_enabled);
        assert!(!c.outbound.enabled, "off by default (PHI stream)");
        assert_eq!(c.outbound.exchange, "ferroehr.fhir");
        assert_eq!(c.outbound.batch_size, 128);
        assert_eq!(c.outbound.poll_interval_ms, 1_000);
        assert_eq!(c.outbound.publish_max_retries, 3);
        assert!(c.outbound.url.expose().starts_with("amqp://"));
    }

    #[test]
    fn tls_upgrades_amqp_scheme() {
        let c = FhirOutboundConfig {
            tls: true,
            url: SecretUrl::new("amqp://guest:guest@host:5672/%2f"),
            ..FhirOutboundConfig::default()
        };
        assert_eq!(c.effective_url(), "amqps://guest:guest@host:5672/%2f");
    }
}
