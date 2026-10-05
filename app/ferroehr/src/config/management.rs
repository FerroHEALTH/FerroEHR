// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! Management-surface configuration.
//!
//! The `[management]` section of the one server configuration tree; it carries **no loader of its own** — the
//! whole tree is assembled once by `ferroehr::config` and this struct is
//! deserialized as a field of it. **Everything is off by default**:
//! `management.enabled = false`, and every individual endpoint defaults to
//! [`AccessLevel::Off`] (→ `404`). The surface only exists once a deployment
//! opts each piece in explicitly.
//!
//! The surface is **ops introspection only** — build info, Prometheus, the
//! metric views, the redacted effective config, and the live log-filter
//! control. Health probes are NOT configured here: the `/health`,
//! `/health/liveness`, and `/health/readiness` endpoints are always-on and
//! public, so an orchestrator can probe a server that has this whole section
//! switched off.
//!
//! No openEHR spec governs configuration or the management surface — our own
//! design.

use serde::{Deserialize, Serialize};

/// The access level of a single management endpoint. Maps onto the existing
/// authentication layer (the same authenticator the API surface uses).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum AccessLevel {
    /// Not mounted at all — the route returns `404` (the default for every
    /// endpoint).
    #[default]
    Off,
    /// Requires an authenticated principal carrying the admin scope (the
    /// admin-scope gate). `401` unauthenticated, `403` authenticated-but-not-admin.
    AdminOnly,
    /// Requires any authenticated principal. `401` unauthenticated.
    Private,
    /// No authentication required.
    Public,
}

impl AccessLevel {
    /// Whether the endpoint is mounted (anything other than [`AccessLevel::Off`]).
    #[must_use]
    pub const fn is_mounted(self) -> bool {
        !matches!(self, AccessLevel::Off)
    }

    /// The level's configuration spelling, as it appears in `ferroehr.toml`.
    ///
    /// Used by the boot log, so what an operator reads back matches what they
    /// wrote.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            AccessLevel::Off => "off",
            AccessLevel::AdminOnly => "admin_only",
            AccessLevel::Private => "private",
            AccessLevel::Public => "public",
        }
    }
}

/// Per-endpoint access levels. Each defaults to [`AccessLevel::Off`].
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EndpointLevels {
    /// `/management/info`.
    #[serde(default)]
    pub info: AccessLevel,
    /// `/management/metrics` (+ `/{name}`) — the JSON registry view.
    #[serde(default)]
    pub metrics: AccessLevel,
    /// `/management/prometheus` — the text exposition scraped by Prometheus.
    #[serde(default)]
    pub prometheus: AccessLevel,
    /// `/management/env` — the redacted effective configuration.
    #[serde(default)]
    pub env: AccessLevel,
    /// `/management/loggers` — the runtime `EnvFilter` control.
    #[serde(default)]
    pub loggers: AccessLevel,
    /// `/management/flamegraph` — the on-demand CPU flamegraph (pprof sampling).
    #[serde(default)]
    pub flamegraph: AccessLevel,
}

/// Limits for the on-demand CPU profiler behind `/management/flamegraph`.
///
/// Sampling is cheap but not free (a `SIGPROF`-driven stack sample at the
/// requested frequency, per the `pprof` crate —
/// <https://docs.rs/pprof/latest/pprof/>), so the caps below bound what a
/// request may ask for; a request outside them is refused with `400`, never
/// silently clamped.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ProfilingConfig {
    /// The longest sample window a single request may ask for, in seconds.
    pub max_seconds: u16,
    /// The highest sampling frequency a request may ask for, in Hz.
    pub max_frequency: i32,
}

impl Default for ProfilingConfig {
    fn default() -> Self {
        Self {
            max_seconds: 30,
            max_frequency: 999,
        }
    }
}

/// The management surface configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ManagementConfig {
    /// Master switch. When `false`, no management routes are mounted at all.
    #[serde(default)]
    pub enabled: bool,
    /// The base path the management endpoints hang off.
    pub base_path: String,
    /// When set, the management surface is served from its **own** listener on
    /// this port (a separate axum server task in the binary) instead of the main
    /// API listener — so production can keep it off the public listener entirely.
    #[serde(default)]
    pub port: Option<u16>,
    /// Per-endpoint access levels — the SINGLE authority for this surface.
    ///
    /// NOTE: there is deliberately no global default beside this. An endpoint
    /// that names no level is `off`, so it is never mounted and answers 404;
    /// a surface this privileged opens one endpoint at a time, by name.
    #[serde(default)]
    pub endpoints: EndpointLevels,
    /// Limits for the on-demand CPU profiler (`endpoints.flamegraph`).
    #[serde(default)]
    pub profiling: ProfilingConfig,
}

impl Default for ManagementConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            base_path: "/management".to_owned(),
            port: None,
            endpoints: EndpointLevels::default(),
            profiling: ProfilingConfig::default(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off() {
        let c = ManagementConfig::default();
        assert!(!c.enabled);
        assert_eq!(c.base_path, "/management");
        assert!(c.port.is_none());
        assert_eq!(c.endpoints.prometheus, AccessLevel::Off);
        assert_eq!(c.endpoints.info, AccessLevel::Off);
        assert_eq!(c.endpoints.flamegraph, AccessLevel::Off);
        assert_eq!(c.profiling.max_seconds, 30);
        assert_eq!(c.profiling.max_frequency, 999);
    }

    /// The health probes are not part of this section any more (they are the
    /// always-on public `/health` family), so a config still carrying the
    /// removed keys must fail loudly at boot rather than be silently ignored
    /// (`deny_unknown_fields`).
    #[test]
    fn removed_probe_keys_are_rejected() {
        let err = toml::from_str::<ManagementConfig>("probes_enabled = true")
            .expect_err("probes_enabled must be an unknown field");
        assert!(
            err.to_string().contains("probes_enabled"),
            "the error must name the offending key: {err}"
        );
        let err = toml::from_str::<EndpointLevels>("health = \"public\"")
            .expect_err("endpoints.health must be an unknown field");
        assert!(
            err.to_string().contains("health"),
            "the error must name the offending key: {err}"
        );
    }
}
