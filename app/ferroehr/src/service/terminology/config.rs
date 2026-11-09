// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[terminology]` table: the one FHIR terminology server the CDR is a
//! client of.
//!
//! BASE `docs/architecture_overview/master12-terminology.adoc` §"Binding
//! Terminology Value-sets to Archetypes" puts the resolver of an archetype
//! constraint binding outside the CDR, in a "terminology query server", so the
//! CDR holds one client to one such server, configured here. It is a field of
//! the one config tree ([`crate::config::FerroEhrConfig`]) with no loader of its
//! own. Absent, the default, no client is built and terminology stays on the
//! in-process `openehr-term` bundle. The terminology extension API is
//! `api.terminology`.
//!
//! NOTE: no openEHR spec governs the transport, the authentication or the
//! configuration of that client — our own design.

use ferrohealth_config::removed::Removed;
use ferrohealth_config::units::Duration;
use serde::{Deserialize, Serialize};

use ferrohealth_secret::secret_url::SecretUrl;

/// What a composition gets when the terminology server cannot resolve one of
/// its bindings (unreachable, an error, an unknown value set).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OnError {
    /// The composition is accepted (fail-open), the default.
    #[default]
    Accept,
    /// The composition is refused (fail-closed).
    Refuse,
}

/// The one FHIR R4B terminology server (`[terminology]`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminologyServerConfig {
    /// The FHIR R4B base URL, e.g. `http://ferroterm:8080/r4b`; required. Any
    /// `userinfo` in it is masked in every rendering.
    pub url: SecretUrl,
    /// On a binding the server cannot resolve: `accept` (the default) or
    /// `refuse`. A resolved non-member is refused under either setting.
    pub on_error: OnError,
    /// How long one request may take, connection included.
    pub timeout: Duration,
    /// The OAuth 2.0 client credentials grant whose bearer token every request
    /// carries (`[terminology.oauth]`). Unset = unauthenticated.
    pub oauth: Option<ferrohealth_smart::config::credentials::OauthSection>,
    /// Retired: refused at load, naming `api.terminology`.
    #[serde(skip_serializing)]
    pub api_enabled: Removed,
    /// Retired: refused at load, naming the keys of `[terminology]`.
    #[serde(skip_serializing)]
    pub server: Removed,
}

impl Default for TerminologyServerConfig {
    fn default() -> Self {
        Self {
            url: SecretUrl::default(),
            on_error: OnError::Accept,
            timeout: Duration::from_secs(10),
            oauth: None,
            api_enabled: Removed::default(),
            server: Removed::default(),
        }
    }
}

impl TerminologyServerConfig {
    /// Whether an unresolvable binding refuses the composition.
    #[must_use]
    pub fn refuses_on_error(&self) -> bool {
        self.on_error == OnError::Refuse
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_defaults_accept_on_error() {
        let c = TerminologyServerConfig::default();
        assert!(!c.refuses_on_error());
        assert!(c.url.is_empty());
        assert!(c.oauth.is_none());
        assert_eq!(c.timeout, Duration::from_secs(10));
    }

    /// The section a release before the one-server shape carried is refused
    /// by name, never ignored: a deployment that still names it would
    /// otherwise lose its terminology server without a word.
    #[test]
    fn the_retired_external_section_is_refused() {
        let err = toml::from_str::<TerminologyServerConfig>(
            "[external]\nenabled = true\n[external.providers.default]\nurl = \"https://ts/fhir\"\n",
        )
        .expect_err("[terminology.external] is no longer a key");
        assert!(err.to_string().contains("external"), "got {err}");
    }

    #[test]
    fn on_error_reads_its_two_words() {
        let refuse: TerminologyServerConfig =
            toml::from_str("on_error = \"refuse\"").expect("refuse");
        assert!(refuse.refuses_on_error());
        assert!(toml::from_str::<TerminologyServerConfig>("on_error = true").is_err());
    }
}
