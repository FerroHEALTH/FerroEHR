// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[terminology]` section: the extension-API toggle and the one FHIR
//! terminology server the CDR is a client of.
//!
//! BASE `docs/architecture_overview/master12-terminology.adoc` §"Binding
//! Terminology Value-sets to Archetypes" puts the resolver of an archetype
//! constraint binding outside the CDR, in a "terminology query server", so the
//! CDR holds one client to one such server, configured here. It is a field of
//! the one config tree ([`crate::config::FerroEhrConfig`]) with no loader of its
//! own. With [`TerminologyServerConfig::enabled`] `false`, the default, no
//! client is built and terminology stays on the in-process `openehr-term`
//! bundle.
//!
//! NOTE: no openEHR spec governs the transport, the authentication or the
//! configuration of that client — our own design.

use serde::{Deserialize, Serialize};

use ferrohealth_secret::secret_url::SecretUrl;

/// The `[terminology]` section: the extension-API toggle and the terminology
/// server.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminologyConfig {
    /// Mount the terminology extension API. Off
    /// by default — the routes answer `404` unless enabled.
    pub api_enabled: bool,
    /// The FHIR terminology server (`[terminology.server]`).
    pub server: TerminologyServerConfig,
}

/// The one FHIR R4B terminology server (`[terminology.server]`).
///
/// Defaults are the off state: `enabled = false`, `fail_on_error = false`, no
/// URL and no credentials.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminologyServerConfig {
    /// Whether the server is consulted. When `false` (the default), no client
    /// is built.
    pub enabled: bool,
    /// The FHIR R4B base URL, e.g. `http://ferroterm:8080/r4b`. Empty while
    /// [`Self::enabled`] is a boot error. Any `userinfo` in it is masked in
    /// every rendering.
    pub url: SecretUrl,
    /// On a binding the server cannot resolve (unreachable, an error, an
    /// unknown value set): `true` refuses the composition (fail-closed),
    /// `false` accepts it (fail-open, the default). A resolved non-member is
    /// refused under either setting.
    pub fail_on_error: bool,
    /// How long one request may take, connection included (ms).
    pub request_timeout_ms: u64,
    /// The OAuth 2.0 client credentials grant whose bearer token every request
    /// carries (`[terminology.server.oauth]`). Unset = unauthenticated.
    pub oauth: Option<ferrohealth_smart::config::OauthSection>,
}

impl Default for TerminologyServerConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            url: SecretUrl::default(),
            fail_on_error: false,
            request_timeout_ms: 10_000,
            oauth: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        let c = TerminologyConfig::default();
        assert!(!c.api_enabled);
        assert!(!c.server.enabled);
        assert!(!c.server.fail_on_error);
        assert!(c.server.url.is_empty());
        assert!(c.server.oauth.is_none());
        assert_eq!(c.server.request_timeout_ms, 10_000);
    }

    /// The section a release before the one-server shape carried is refused
    /// by name, never ignored: a deployment that still names it would
    /// otherwise lose its terminology server without a word.
    #[test]
    fn the_retired_external_section_is_refused() {
        let err = toml::from_str::<TerminologyConfig>(
            "[external]\nenabled = true\n[external.providers.default]\nurl = \"https://ts/fhir\"\n",
        )
        .expect_err("[terminology.external] is no longer a key");
        assert!(err.to_string().contains("external"), "got {err}");
    }
}
