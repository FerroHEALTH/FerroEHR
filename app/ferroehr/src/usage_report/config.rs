// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[usage_report]` configuration section.
//!
//! No openEHR spec governs configuration or the usage report — our own design.

use serde::{Deserialize, Serialize};

use crate::config::loader::ConfigError;
use crate::config::secret::SecretUrl;

/// How the instance was deployed, as the report states it.
///
/// Set by the deployment artifact rather than detected: the Helm chart writes
/// `helm` and the compose files write `compose`; anything else reports
/// `unknown` unless the operator says otherwise.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Deployment {
    /// The Helm chart.
    Helm,
    /// A Docker Compose file.
    Compose,
    /// The plain binary.
    Binary,
    /// Not stated.
    #[default]
    Unknown,
}

/// The usage report to `FerroPULSE`: whether it runs, where it goes, and what
/// counts as a slow AQL query.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct UsageReportConfig {
    /// Whether the instance sends the report. On by default; every boot logs
    /// which, with the switch.
    pub enabled: bool,
    /// The collector endpoint. HTTPS, except a loopback host, which may be
    /// plain HTTP because nothing leaves the machine. Credentials are refused
    /// at boot, and any `userinfo` is masked in every rendering.
    pub endpoint: SecretUrl,
    /// The execution time in milliseconds above which an AQL query counts as
    /// slow in the report.
    pub slow_aql_ms: u64,
    /// How the instance was deployed; the deployment artifacts set it.
    pub deployment: Deployment,
}

impl Default for UsageReportConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: SecretUrl::new("https://report.ferropulse.eu/v1/report"),
            slow_aql_ms: 1000,
            deployment: Deployment::Unknown,
        }
    }
}

impl UsageReportConfig {
    /// Returns the endpoint's semantic defects, one error per defect.
    ///
    /// The endpoint must parse as an absolute URL, carry no credentials, and be
    /// `https`, or `http` to a loopback host. The threshold must be positive.
    #[must_use]
    pub fn errors(&self) -> Vec<ConfigError> {
        let mut errors = Vec::new();
        match url::Url::parse(self.endpoint.expose()) {
            Ok(url) => {
                if !url.username().is_empty() || url.password().is_some() {
                    errors.push(ConfigError::semantic(
                        "usage_report.endpoint carries credentials; the report needs none"
                            .to_owned(),
                    ));
                }
                let loopback = is_loopback(&url);
                match url.scheme() {
                    "https" => {}
                    "http" if loopback => {}
                    scheme => errors.push(ConfigError::semantic(format!(
                        "usage_report.endpoint uses `{scheme}`; the report is sent over \
                         https only (plain http is accepted for a loopback host alone)"
                    ))),
                }
            }
            Err(error) => errors.push(ConfigError::semantic(format!(
                "usage_report.endpoint is not an absolute URL: {error}"
            ))),
        }
        if self.slow_aql_ms == 0 {
            errors.push(ConfigError::semantic(
                "usage_report.slow_aql_ms must be at least 1".to_owned(),
            ));
        }
        errors
    }

    /// Returns whether the endpoint is plain HTTP to a loopback host.
    #[must_use]
    pub fn is_loopback_http(&self) -> bool {
        url::Url::parse(self.endpoint.expose())
            .is_ok_and(|url| url.scheme() == "http" && is_loopback(&url))
    }
}

/// Whether `url` names a loopback host (`localhost` or a loopback address).
fn is_loopback(url: &url::Url) -> bool {
    match url.host() {
        Some(url::Host::Domain(domain)) => domain.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(address)) => address.is_loopback(),
        Some(url::Host::Ipv6(address)) => address.is_loopback(),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_endpoint(endpoint: &str) -> UsageReportConfig {
        UsageReportConfig {
            endpoint: SecretUrl::new(endpoint),
            ..UsageReportConfig::default()
        }
    }

    #[test]
    fn the_default_is_on_and_valid() {
        let config = UsageReportConfig::default();
        assert!(config.enabled);
        assert!(config.errors().is_empty());
        assert!(!config.is_loopback_http());
    }

    #[test]
    fn plain_http_is_refused_except_to_loopback() {
        assert_eq!(with_endpoint("http://report.example/v1").errors().len(), 1);
        for loopback in [
            "http://127.0.0.1:9/v1/report",
            "http://localhost:9/v1/report",
            "http://[::1]:9/v1/report",
        ] {
            let config = with_endpoint(loopback);
            assert!(config.errors().is_empty(), "{loopback}");
            assert!(config.is_loopback_http(), "{loopback}");
        }
    }

    #[test]
    fn credentials_other_schemes_and_garbage_are_refused() {
        assert_eq!(
            with_endpoint("https://user:pw@report.example/")
                .errors()
                .len(),
            1
        );
        assert_eq!(with_endpoint("ftp://report.example/").errors().len(), 1);
        assert_eq!(with_endpoint("not a url").errors().len(), 1);
    }

    #[test]
    fn a_zero_slow_threshold_is_refused() {
        let config = UsageReportConfig {
            slow_aql_ms: 0,
            ..UsageReportConfig::default()
        };
        assert_eq!(config.errors().len(), 1);
    }
}
