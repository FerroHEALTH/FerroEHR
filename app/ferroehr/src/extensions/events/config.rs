// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[events]` section — contribution-outbox eventing configuration.
//!
//! **No openEHR spec governs this — our own design/extension.** A field of the
//! one config tree ([`crate::config::FerroEhrConfig`]); no loader of its own. Publishing is
//! **off by default**: with [`EventsConfig::enabled`] `false` the binary never
//! spawns the publisher.
//!
//! The commit path only writes `event_outbox` rows when an outbox consumer is
//! configured on (this publisher OR the FHIR outbound emitter), gated in the
//! binary from `events.enabled || fhir.outbound.enabled`.

use std::path::PathBuf;

use crate::config::secret::SecretUrl;
use serde::{Deserialize, Serialize};

/// Contribution-outbox eventing configuration (`[events]`) — our own extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EventsConfig {
    /// Master switch. Off by default; also (with `fhir.outbound.enabled`) gates
    /// the per-commit outbox INSERT.
    pub enabled: bool,
    /// AMQP broker URL (credentials redacted from every rendering).
    pub url: SecretUrl,
    /// Path to a file holding the broker URL, read at boot in place of
    /// [`Self::url`]. Setting both this and a non-default `url` is a boot error.
    pub url_file: Option<PathBuf>,
    /// Topic exchange to publish to (the PHI-free envelope stream).
    pub exchange: String,
    /// Use TLS: when `true` an `amqp://` URL is upgraded to `amqps://` (an
    /// already-`amqps://` URL is TLS regardless).
    pub tls: bool,
    /// Rows drained per poll.
    pub batch_size: i64,
    /// Poll interval when idle (ms).
    pub poll_interval_ms: u64,
    /// Published-row retention window in days.
    pub retention_days: i64,
    /// Retention-prune cadence in seconds.
    pub prune_interval_secs: u64,
    /// Per-row publish retry count before backing off.
    pub publish_max_retries: usize,
    /// Mount the `/admin/event_subscription` CRUD routes.
    pub admin_api: bool,
}

impl Default for EventsConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: SecretUrl::new("amqp://guest:guest@localhost:5672/%2f"),
            url_file: None,
            exchange: "ferroehr.events".to_owned(),
            tls: false,
            batch_size: 128,
            poll_interval_ms: 1_000,
            retention_days: 7,
            prune_interval_secs: 3_600,
            publish_max_retries: 3,
            admin_api: false,
        }
    }
}

impl EventsConfig {
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
    fn defaults_are_off_and_sane() {
        let c = EventsConfig::default();
        assert!(!c.enabled);
        assert!(!c.admin_api);
        assert_eq!(c.exchange, "ferroehr.events");
        assert_eq!(c.batch_size, 128);
        assert_eq!(c.poll_interval_ms, 1_000);
        assert_eq!(c.retention_days, 7);
        assert_eq!(c.prune_interval_secs, 3_600);
        assert_eq!(c.publish_max_retries, 3);
        assert!(c.url.expose().starts_with("amqp://"));
    }

    #[test]
    fn tls_upgrades_amqp_scheme() {
        let c = EventsConfig {
            tls: true,
            url: SecretUrl::new("amqp://guest:guest@host:5672/%2f"),
            ..EventsConfig::default()
        };
        assert_eq!(c.effective_url(), "amqps://guest:guest@host:5672/%2f");
    }

    #[test]
    fn tls_leaves_amqps_untouched() {
        let c = EventsConfig {
            tls: true,
            url: SecretUrl::new("amqps://host:5671/%2f"),
            ..EventsConfig::default()
        };
        assert_eq!(c.effective_url(), "amqps://host:5671/%2f");
    }
}
