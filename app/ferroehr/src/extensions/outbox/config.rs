// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[record.change_feed]` section: how long the commit-event outbox
//! keeps a row.
//!
//! **No openEHR spec governs this — our own design/extension.** A field of the
//! one config tree ([`crate::config::FerroEhrConfig`]); no loader of its own.
//! Who reads the outbox is policy, read per request from the change-feed
//! grants ([`crate::extensions::outbox`]); with no reader granted, no commit
//! writes an outbox row and the feed answers `404` for every name.

use ferrohealth_config::units::Duration;
use serde::{Deserialize, Serialize};

/// The commit-event outbox configuration (`[record.change_feed]`), our own
/// extension.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
pub struct OutboxConfig {
    /// How long a row is kept once every active reader has acknowledged it,
    /// counted from its commit instant.
    pub retention: Duration,
    /// How often the retention prune runs.
    pub prune_interval: Duration,
}

impl Default for OutboxConfig {
    fn default() -> Self {
        Self {
            retention: Duration::from_days(7),
            prune_interval: Duration::from_hours(1),
        }
    }
}

impl OutboxConfig {
    /// The semantic refusals of this section: a retention or interval that
    /// keeps nothing.
    #[must_use]
    pub fn errors(&self) -> Vec<String> {
        let mut errors = Vec::new();
        if self.retention.as_millis() < 1_000 {
            errors.push(format!(
                "record.change_feed.retention = \"{}\" keeps no acknowledged row for a reader \
                 to read again; set a second or more, such as \"7d\"",
                self.retention
            ));
        }
        if self.prune_interval.is_zero() {
            errors.push(
                "record.change_feed.prune_interval = \"0s\" would prune without pause; set a \
                 duration such as \"1h\""
                    .to_owned(),
            );
        }
        errors
    }
}

/// Whether `name` matches `^[a-z][a-z0-9-]{0,62}$`, the
/// `ck_event_outbox_reader_name` check of the reader registry.
#[must_use]
pub fn is_reader_name(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|first| first.is_ascii_lowercase())
        && name.len() <= 63
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_keeps_a_week_and_prunes_hourly() {
        let config = OutboxConfig::default();
        assert_eq!(config.errors(), Vec::<String>::new());
        assert_eq!(config.retention, Duration::from_days(7));
        assert_eq!(config.prune_interval, Duration::from_hours(1));
    }

    #[test]
    fn a_reader_name_follows_the_registry_check() {
        for good in ["ferrobridge", "a", "omop-etl-2"] {
            assert!(is_reader_name(good), "{good}");
        }
        let long = "a".repeat(64);
        for bad in [
            "",
            "Ferrobridge",
            "2etl",
            "-etl",
            "etl_2",
            "etl.2",
            long.as_str(),
        ] {
            assert!(!is_reader_name(bad), "{bad}");
        }
        assert!(is_reader_name(&"a".repeat(63)));
    }

    #[test]
    fn an_empty_window_is_refused() {
        let config = OutboxConfig {
            retention: Duration::ZERO,
            prune_interval: Duration::ZERO,
        };
        let errors = config.errors();
        assert_eq!(errors.len(), 2, "{errors:?}");
        assert!(errors.iter().any(|e| e.contains("retention")));
        assert!(errors.iter().any(|e| e.contains("prune_interval")));
    }
}
