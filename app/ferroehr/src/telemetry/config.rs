// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Telemetry configuration: the `[log]` ([`LogConfig`]) and `[telemetry]`
//! ([`OtelConfig`]) sections of the one config tree.
//!
//! No loaders here — both are fields of [`crate::config::FerroEhrConfig`],
//! assembled once at boot. Everything is off by default: no OTLP endpoint means
//! the `OTel` layer is **not installed at all** (zero overhead); logging always
//! runs (stdout). [`TelemetryConfig`] is the runtime pair the binary hands to
//! [`super::init`]; the config tree stores `log`/`telemetry` as siblings.
//!
//! No openEHR spec governs telemetry — our own design.

use serde::{Deserialize, Serialize};

/// The stdout log rendering profile.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum LogFormat {
    /// Pick `json` when stdout is not a TTY, `pretty` when it is.
    #[default]
    Auto,
    /// One JSON object per line (containers / log collectors).
    Json,
    /// Human-friendly multi-line (interactive dev).
    Pretty,
}

impl LogFormat {
    /// Resolves this profile against the stdout terminal state, yielding the
    /// rendering that is actually installed.
    ///
    /// The terminal state is a parameter rather than a probe so the `auto` rule
    /// lives in ONE testable place: the log layer and the boot banner both key
    /// off the result, and a banner printed ahead of JSON output would make the
    /// first bytes of stdout unparseable.
    #[must_use]
    pub fn resolve(self, stdout_is_terminal: bool) -> ResolvedLogFormat {
        match self {
            Self::Pretty => ResolvedLogFormat::Pretty,
            Self::Auto if stdout_is_terminal => ResolvedLogFormat::Pretty,
            Self::Json | Self::Auto => ResolvedLogFormat::Json,
        }
    }
}

/// The stdout rendering actually installed, with [`LogFormat::Auto`] decided.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolvedLogFormat {
    /// One JSON object per line.
    Json,
    /// Human-friendly multi-line text.
    Pretty,
}

/// Logging configuration (`[log]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct LogConfig {
    /// The stdout rendering profile.
    pub format: LogFormat,
    /// The boot `EnvFilter` directives (also the `/management/loggers` reset
    /// target). `RUST_LOG` is a recognized lower-priority alias.
    pub filter: String,
}

impl Default for LogConfig {
    fn default() -> Self {
        Self {
            format: LogFormat::Auto,
            filter: defaults::filter(),
        }
    }
}

/// OpenTelemetry export configuration (`[telemetry]`; traces always, metrics
/// push opt-in).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct OtelConfig {
    /// The OTLP/gRPC collector endpoint. **Unset ⇒ the `OTel` layer is not
    /// installed** (traces are not exported; zero overhead).
    pub otlp_endpoint: Option<String>,
    /// The `service.name` resource attribute.
    pub service_name: String,
    /// The `deployment.environment` resource attribute.
    pub environment: String,
    /// Head-sampling ratio for `parentbased_traceidratio` (1.0 = sample all;
    /// 0.1 is the documented prod starting point).
    pub traces_sample_ratio: f64,
    /// Whether to also **push** metrics over OTLP (a periodic `OTel` meter
    /// provider alongside the Prometheus pull surface). Off by default.
    pub metrics_push: bool,
    /// Span-timing flamegraph capture (the `tracing-flame` layer,
    /// <https://docs.rs/tracing-flame/latest/tracing_flame/>): write folded
    /// stack samples of every `tracing` span to this file, for offline
    /// rendering with inferno. **Unset ⇒ the layer is not installed at all**
    /// (zero overhead) — set it for a diagnostic session, not as a standing
    /// production posture (the file grows with span traffic).
    pub flame_file: Option<std::path::PathBuf>,
}

impl Default for OtelConfig {
    fn default() -> Self {
        Self {
            otlp_endpoint: None,
            service_name: defaults::service_name(),
            environment: defaults::environment(),
            traces_sample_ratio: defaults::sample_ratio(),
            metrics_push: false,
            flame_file: None,
        }
    }
}

impl OtelConfig {
    /// Whether the `OTel` export layer should be installed at all.
    #[must_use]
    pub fn export_enabled(&self) -> bool {
        self.otlp_endpoint
            .as_ref()
            .is_some_and(|e| !e.trim().is_empty())
    }
}

/// The runtime telemetry pair the binary hands to [`super::init`]: the `[log]`
/// and `[telemetry]` sections together.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TelemetryConfig {
    /// Logging configuration.
    pub log: LogConfig,
    /// OTLP export configuration.
    pub otel: OtelConfig,
}

mod defaults {
    pub(super) fn filter() -> String {
        "info,ferroehr=info".to_owned()
    }
    pub(super) fn service_name() -> String {
        "ferroehr".to_owned()
    }
    pub(super) fn environment() -> String {
        "dev".to_owned()
    }
    pub(super) const fn sample_ratio() -> f64 {
        1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_disable_export() {
        let c = TelemetryConfig::default();
        assert!(!c.otel.export_enabled());
        assert_eq!(c.log.filter, "info,ferroehr=info");
        assert_eq!(c.otel.service_name, "ferroehr");
        assert_eq!(c.log.format, LogFormat::Auto);
        assert!(!c.otel.metrics_push);
        assert!(c.otel.flame_file.is_none());
    }

    /// `auto` follows the terminal state; the explicit profiles ignore it. This
    /// is the ONE place the rule lives, so the log layer and the boot banner
    /// cannot disagree about what stdout carries.
    #[test]
    fn auto_resolves_off_the_terminal_state() {
        assert_eq!(LogFormat::Auto.resolve(false), ResolvedLogFormat::Json);
        assert_eq!(LogFormat::Auto.resolve(true), ResolvedLogFormat::Pretty);
        for is_terminal in [false, true] {
            assert_eq!(
                LogFormat::Json.resolve(is_terminal),
                ResolvedLogFormat::Json
            );
            assert_eq!(
                LogFormat::Pretty.resolve(is_terminal),
                ResolvedLogFormat::Pretty
            );
        }
    }
}
