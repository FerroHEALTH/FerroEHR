// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The boot rules of `[authz.abac]` and the two engines' own tables.
//!
//! The policy kinds, the parameters each may carry, what each engine needs,
//! and `[authz.abac.cedar]` and `[authz.abac.remote]`. No openEHR spec governs RBAC or ABAC:
//! our own design.

use std::path::PathBuf;

use ferrohealth_secret::secret_url::RedactedUrl;
use ferrohealth_smart::config::endpoint::EndpointSection;
use serde::{Deserialize, Serialize};

use crate::config::authz::{AbacConfig, AbacEngineKind, AbacParam, AuthzConfigError};

/// The canonical resource-kind keys accepted in the `abac.policy` map, and the
/// two on which a `template` parameter is illegal.
const POLICY_KINDS: [&str; 6] = [
    "ehr",
    "ehr_status",
    "composition",
    "contribution",
    "query",
    "directory",
];

/// The resource kinds a remote PDP must have a policy for.
///
/// `directory` is absent deliberately: DIRECTORY gating is the separate
/// [`AbacConfig::check_directory`] opt-in, so its policy is required only when
/// that switch is on.
pub const REQUIRED_REMOTE_POLICY_KINDS: [&str; 5] =
    ["ehr", "ehr_status", "composition", "contribution", "query"];

impl AbacConfig {
    /// Validate the ABAC section at boot (hard errors). Only called when
    /// `enabled`.
    pub(crate) fn validate(&self) -> Result<(), AuthzConfigError> {
        self.validate_policy_rules()?;
        match self.engine {
            AbacEngineKind::Remote => self.validate_remote_engine(),
            AbacEngineKind::Cedar => match self.cedar.policy_dir {
                Some(_) => Ok(()),
                None => Err(AuthzConfigError::CedarDirMissing),
            },
        }
    }

    /// Every configured policy rule names a known resource kind, and no rule
    /// asks for a parameter its kind cannot carry.
    ///
    /// The `template` parameter is a property of committed content, so an
    /// EHR-level or EHR_STATUS-level rule can never be given one.
    fn validate_policy_rules(&self) -> Result<(), AuthzConfigError> {
        for (kind, rule) in &self.policy {
            if !POLICY_KINDS.contains(&kind.as_str()) {
                return Err(AuthzConfigError::UnknownPolicyKind(kind.clone()));
            }
            if !rule.parameters.contains(&AbacParam::Template) {
                continue;
            }
            match kind.as_str() {
                "ehr" => return Err(AuthzConfigError::TemplateParamIllegal("ehr")),
                "ehr_status" => return Err(AuthzConfigError::TemplateParamIllegal("ehr_status")),
                _ => {}
            }
        }
        Ok(())
    }

    /// The remote-PDP engine's own requirements: a base server URL with a
    /// trailing slash, and a policy for every resource kind the PEP consults
    /// (an unconfigured kind denies, and the missing rule is a boot error
    /// rather than a silent refusal at request time).
    fn validate_remote_engine(&self) -> Result<(), AuthzConfigError> {
        let server = Some(self.remote.endpoint.url.expose())
            .filter(|s| !s.trim().is_empty())
            .ok_or(AuthzConfigError::RemoteServerMissing)?;
        if !server.ends_with('/') {
            return Err(AuthzConfigError::RemoteServerTrailingSlash(
                RedactedUrl(server).to_string(),
            ));
        }
        for kind in REQUIRED_REMOTE_POLICY_KINDS {
            if !self.policy.contains_key(kind) {
                return Err(AuthzConfigError::RemotePolicyMissing(kind));
            }
        }
        if self.check_directory && !self.policy.contains_key("directory") {
            return Err(AuthzConfigError::RemotePolicyMissing("directory"));
        }
        Ok(())
    }
}

/// The embedded-Cedar engine settings.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CedarConfig {
    /// Directory of `*.cedar` policy files (required when `engine = cedar`).
    pub policy_dir: Option<PathBuf>,
    /// Optional periodic hot-reload interval; off when unset.
    pub reload_interval: Option<ferrohealth_config::units::Duration>,
    /// Retired in v5.7.0: refused at load, naming `reload_interval`.
    #[serde(skip_serializing)]
    pub reload_secs: ferrohealth_config::removed::Removed,
}

/// The remote-PDP client settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct RemoteConfig {
    /// `[authz.abac.remote.endpoint]`, the family's outbound endpoint: the PDP
    /// base URL (`url`; the policy name is appended, so it must end with `/`;
    /// required when `engine = remote`), the timeouts (a connect timeout keeps
    /// a PDP that blackholes packets from parking the request until the OS TCP
    /// timeout), the `retry` of a call that got no answer or a `5xx`, the TLS
    /// and the credential, such as the OAuth 2.0 client credentials grant.
    /// Any `userinfo` in the URL is masked in every rendering.
    pub endpoint: EndpointSection,
    /// Retired in v5.7.0: refused at load, naming
    /// `endpoint.credentials.oauth`.
    #[serde(skip_serializing)]
    pub oauth: ferrohealth_config::removed::Removed,
    /// Retired in v5.7.0: refused at load, naming `url`.
    #[serde(skip_serializing)]
    pub server: ferrohealth_config::removed::Removed,
    /// Retired in v5.7.0: refused at load, naming `connect_timeout`.
    #[serde(skip_serializing)]
    pub connect_timeout_ms: ferrohealth_config::removed::Removed,
    /// Retired in v5.7.0: refused at load, naming `timeout`.
    #[serde(skip_serializing)]
    pub request_timeout_ms: ferrohealth_config::removed::Removed,
}

impl Default for RemoteConfig {
    fn default() -> Self {
        Self {
            endpoint: EndpointSection {
                timeout: ferrohealth_config::units::Duration::from_secs(5),
                connect_timeout: Some(ferrohealth_config::units::Duration::from_secs(2)),
                ..EndpointSection::default()
            },
            oauth: ferrohealth_config::removed::Removed::default(),
            server: ferrohealth_config::removed::Removed::default(),
            connect_timeout_ms: ferrohealth_config::removed::Removed::default(),
            request_timeout_ms: ferrohealth_config::removed::Removed::default(),
        }
    }
}
