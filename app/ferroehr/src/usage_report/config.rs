// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[usage_report]` configuration section, and the CI detection that
//! switches the report off on a CI runner.
//!
//! No openEHR spec governs configuration or the usage report — our own design.

use serde::{Deserialize, Serialize};

use crate::config::loader::ConfigError;
use ferrohealth_secret::secret_url::SecretUrl;

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
    /// The CI marker of the environment the configuration was assembled from.
    ///
    /// Never read from a file or a `FERROEHR__` variable: assembly sets it
    /// from [`CiDetected::from_lookup`] over the same environment snapshot.
    #[serde(skip)]
    pub ci: Option<CiDetected>,
}

/// The variables that mark a CI run, in the order they are read.
///
/// GitHub Actions sets both on every runner
/// (<https://docs.github.com/en/actions/reference/workflows-and-actions/variables#default-environment-variables>),
/// and most other CI services set `CI`.
pub const CI_VARIABLES: [&str; 2] = ["GITHUB_ACTIONS", "CI"];

/// A CI run the process takes part in, named by the variable that marks it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CiDetected {
    variable: &'static str,
}

impl CiDetected {
    /// Returns the first of [`CI_VARIABLES`] that `lookup` reports as `true`
    /// or `1`, compared case-insensitively.
    ///
    /// Any other value, `false` included, and an unset variable mark no CI
    /// run.
    #[must_use]
    pub fn from_lookup<'a>(lookup: impl Fn(&str) -> Option<&'a str>) -> Option<Self> {
        CI_VARIABLES
            .into_iter()
            .find(|name| {
                lookup(name).is_some_and(|value| value == "1" || value.eq_ignore_ascii_case("true"))
            })
            .map(|variable| Self { variable })
    }

    /// Returns the name of the variable that marks the CI run.
    #[must_use]
    pub const fn variable(self) -> &'static str {
        self.variable
    }
}

/// Whether a process sends the report, and why not.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Switch {
    /// The report runs.
    On,
    /// `usage_report.enabled = false`.
    OffByConfig,
    /// A CI run was detected; the configuration cannot switch it back on.
    OffInCi(CiDetected),
}

impl Default for UsageReportConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            endpoint: SecretUrl::new("https://report.ferropulse.eu/v1/report"),
            slow_aql_ms: 1000,
            deployment: Deployment::Unknown,
            ci: None,
        }
    }
}

impl UsageReportConfig {
    /// Returns whether this process sends the report: `enabled`, and no CI run
    /// detected.
    ///
    /// The configuration can only switch the report off; in CI it stays off
    /// whatever `enabled` says.
    #[must_use]
    pub fn switch(&self) -> Switch {
        match (self.enabled, self.ci) {
            (false, _) => Switch::OffByConfig,
            (true, Some(ci)) => Switch::OffInCi(ci),
            (true, None) => Switch::On,
        }
    }

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

    /// The switch `config` takes in the environment `vars`.
    fn switch_in(config: &UsageReportConfig, vars: &[(&str, &'static str)]) -> Switch {
        let config = UsageReportConfig {
            ci: CiDetected::from_lookup(|name| {
                vars.iter()
                    .find(|(key, _)| *key == name)
                    .map(|(_, value)| *value)
            }),
            ..config.clone()
        };
        config.switch()
    }

    #[test]
    fn github_actions_true_switches_the_report_off() {
        let on = UsageReportConfig::default();
        for value in ["true", "TRUE", "True", "1"] {
            assert_eq!(
                switch_in(&on, &[("GITHUB_ACTIONS", value)]),
                Switch::OffInCi(CiDetected {
                    variable: "GITHUB_ACTIONS"
                }),
                "GITHUB_ACTIONS={value}"
            );
        }
    }

    #[test]
    fn ci_one_switches_the_report_off() {
        let switch = switch_in(&UsageReportConfig::default(), &[("CI", "1")]);
        assert_eq!(switch, Switch::OffInCi(CiDetected { variable: "CI" }));
        let Switch::OffInCi(ci) = switch else {
            panic!("CI=1 is a CI run");
        };
        assert_eq!(ci.variable(), "CI");
    }

    #[test]
    fn github_actions_is_named_before_ci() {
        assert_eq!(
            switch_in(
                &UsageReportConfig::default(),
                &[("CI", "true"), ("GITHUB_ACTIONS", "true")]
            ),
            Switch::OffInCi(CiDetected {
                variable: "GITHUB_ACTIONS"
            })
        );
    }

    #[test]
    fn ci_false_and_other_values_leave_the_report_on() {
        let on = UsageReportConfig::default();
        for value in ["false", "0", "", "yes", " true"] {
            assert_eq!(
                switch_in(&on, &[("CI", value), ("GITHUB_ACTIONS", value)]),
                Switch::On,
                "CI={value:?}"
            );
        }
    }

    #[test]
    fn no_ci_variable_leaves_the_report_on() {
        assert_eq!(switch_in(&UsageReportConfig::default(), &[]), Switch::On);
    }

    #[test]
    fn enabled_false_is_off_with_or_without_ci() {
        let off = UsageReportConfig {
            enabled: false,
            ..UsageReportConfig::default()
        };
        assert_eq!(switch_in(&off, &[]), Switch::OffByConfig);
        assert_eq!(switch_in(&off, &[("CI", "false")]), Switch::OffByConfig);
        assert_eq!(
            switch_in(&off, &[("GITHUB_ACTIONS", "true")]),
            Switch::OffByConfig
        );
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
