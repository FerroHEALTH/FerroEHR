// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! The `[terminology]` section — extension-API toggle + external-server
//! validation config.
//!
//! No openEHR spec governs the transport or config mechanics — our own design.
//! `BASE/docs/architecture_overview/master12-terminology.adoc` models the
//! concrete backend as an external "terminology query server", which is why this
//! config lives beside the interface realization in `service/terminology/`. It
//! is a field of the one config tree ([`crate::config::FerroEhrConfig`]) with no
//! loader of its own, grouping the extension-API toggle (`api_enabled`) with
//! [`ExternalTerminologyConfig`] under `[terminology.external]`.
//!
//! Provider selection is bundle-by-default and FHIR opt-in: with
//! [`ExternalTerminologyConfig::enabled`] `false`, the default, no remote
//! provider is built and composition validation stays on the in-process
//! `openehr-term` bundle.
//!
//! Every entry of [`ExternalTerminologyConfig::providers`] is materialised and
//! [`ExternalTerminologyConfig::routes`] maps a terminology id or system URI to
//! the provider that serves it ([`super::router::TerminologyRouter`]), because a
//! deployment binds to several terminologies at once (BASE master12 §Overview
//! names LOINC, `ICDx`, ICPC, SNOMED CT "and the many other terminologies and
//! vocabularies used in healthcare").
//!
//! NOTE: no openEHR spec governs the routing-config mechanics — our own
//! design/extension; only the requirement to serve several terminologies
//! simultaneously is the spec's.

use std::collections::BTreeMap;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::config::secret::Secret;
use crate::service::status::SmError;

use super::fhir::FhirTerminologyProvider;
use super::oauth2::TokenSource;

/// The provider name the router falls back to when no route matches
/// (or the sole configured provider when there is exactly one).
pub(super) const DEFAULT_PROVIDER_NAME: &str = "default";

/// The `[terminology]` section: the extension-API toggle + external-server
/// validation config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminologyConfig {
    /// Mount the terminology extension API (SM `I_TERMINOLOGY_SERVICE`). Off
    /// by default — the routes answer `404` unless enabled.
    pub api_enabled: bool,
    /// External terminology-server validation (`[terminology.external]`).
    pub external: ExternalTerminologyConfig,
}

/// External-terminology validation configuration (`[terminology.external]`).
///
/// Defaults are the off state: `enabled = false`, `fail_on_error = false`, no
/// providers.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ExternalTerminologyConfig {
    /// Whether external-terminology validation is active. When `false` (the
    /// default), no remote provider is built.
    #[serde(default)]
    pub enabled: bool,
    /// On a terminology-server/connectivity error: `true` = reject the
    /// composition (fail-closed); `false` = accept it (fail-open, the
    /// default). Consumed by the composition-validation walker — the raw
    /// provider always surfaces the error; this flag decides how the caller
    /// treats it.
    #[serde(default)]
    pub fail_on_error: bool,
    /// The configured terminology-server providers, keyed by name. **All** of
    /// them are materialised at boot (BASE master12 §Overview — a deployment
    /// binds to several terminologies at the same time).
    #[serde(default)]
    pub providers: BTreeMap<String, FhirProviderConfig>,
    /// Terminology → provider routing: a terminology id (`SNOMED-CT`) or
    /// system URI (`http://snomed.info/sct`) mapped to a provider name from
    /// [`Self::providers`]. Lookups are case-insensitive and exact; an
    /// unmatched key falls back to the `default` provider (or the sole
    /// configured one). No openEHR spec governs the routing map — our own
    /// design/extension.
    #[serde(default)]
    pub routes: BTreeMap<String, String>,
    /// `OAuth2` client-credentials clients a provider authenticates with,
    /// keyed by the name its [`FhirProviderConfig::oauth2_client`] references.
    #[serde(default)]
    pub oauth2_clients: BTreeMap<String, TerminologyOauth2Config>,
}

/// How the `OAuth2` client authenticates at the token endpoint (RFC 6749
/// §2.3.1: `client_secret_basic` — the HTTP Basic form servers MUST support —
/// or the `client_secret_post` form body).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Oauth2AuthMethod {
    /// Credentials in the `Authorization: Basic` header (the RFC 6749 default).
    #[default]
    ClientSecretBasic,
    /// Credentials in the token-request form body.
    ClientSecretPost,
}

/// An `OAuth2` client-credentials client used to authenticate to a terminology
/// server (`[terminology.external.oauth2_clients.<name>]`).
///
/// No openEHR spec governs terminology-server authentication — our own
/// design/extension; `BASE/docs/architecture_overview/master12-terminology.adoc`
/// only models the backend as an external "terminology query server".
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct TerminologyOauth2Config {
    /// The `OAuth2` token endpoint (RFC 6749 §3.2). Empty is a boot error.
    pub token_url: String,
    /// The registered client identifier. Empty is a boot error.
    pub client_id: String,
    /// The client secret (redacted in every rendering); or set
    /// [`Self::client_secret_file`].
    pub client_secret: Option<Secret>,
    /// A file whose contents are the client secret, resolved by the config
    /// loader into [`Self::client_secret`].
    pub client_secret_file: Option<PathBuf>,
    /// Scopes requested with the client-credentials grant (RFC 6749 §4.4.2).
    pub scopes: Vec<String>,
    /// How long before a token's stated expiry it is refreshed, in seconds.
    pub refresh_leeway_secs: u64,
    /// Client authentication method at the token endpoint.
    pub auth_method: Oauth2AuthMethod,
}

impl Default for TerminologyOauth2Config {
    fn default() -> Self {
        Self {
            token_url: String::new(),
            client_id: String::new(),
            client_secret: None,
            client_secret_file: None,
            scopes: Vec::new(),
            refresh_leeway_secs: 30,
            auth_method: Oauth2AuthMethod::ClientSecretBasic,
        }
    }
}

/// The kind of terminology server. Only FHIR R4B is supported.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderKind {
    /// A FHIR R4B terminology server (`$validate-code`/`$expand`/`$subsumes`/
    /// `$lookup`).
    #[default]
    Fhir,
}

/// The FHIR operation used for value-set membership (`value_set_validate`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FhirOperation {
    /// FHIR `ValueSet/$validate-code` — a direct yes/no (least payload).
    #[default]
    ValidateCode,
    /// FHIR `ValueSet/$expand` + a membership test — the fallback for
    /// servers that lack `$validate-code`.
    Expand,
}

/// Configuration for a single FHIR R4B terminology-server provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FhirProviderConfig {
    /// Server kind (`type = "fhir"`).
    #[serde(rename = "type")]
    pub kind: ProviderKind,
    /// FHIR R4B base URL, e.g. `https://r4.ontoserver.csiro.au/fhir`.
    /// Empty is a boot error.
    pub url: String,
    /// The membership operation for `value_set_validate` (default
    /// `validate_code`).
    pub operation: FhirOperation,
    /// TCP connect timeout (ms).
    pub connect_timeout_ms: u64,
    /// Overall request timeout (ms).
    pub request_timeout_ms: u64,
    /// Name of the `OAuth2` client-credentials client
    /// ([`ExternalTerminologyConfig::oauth2_clients`]) whose bearer token is
    /// attached to every request to this provider. Unset = unauthenticated.
    pub oauth2_client: Option<String>,
    /// PEM file holding the client certificate (optionally a chain) this
    /// provider presents to its terminology server for **mutual TLS**. Set
    /// together with [`Self::client_key_path`]; unset = no client identity.
    ///
    /// The identity is per provider because a client certificate is issued by
    /// the peer's PKI — see the [`super::tls`] module NOTE.
    pub client_cert_path: Option<PathBuf>,
    /// PEM file holding the private key of [`Self::client_cert_path`]. Both
    /// keys are set together; one without the other is a boot error.
    pub client_key_path: Option<PathBuf>,
    /// PEM bundle of the trust anchors this provider's terminology-server
    /// certificate is verified against. When set it **replaces** the default
    /// anchors for this provider (a privately-issued server is pinned to its
    /// own CA); unset = the platform's default trust. Server-certificate and
    /// hostname verification are always on — there is no way to disable them.
    pub ca_bundle_path: Option<PathBuf>,
    /// Result-cache TTL in seconds for this provider's FHIR operations
    /// (`$validate-code`/`$expand`/`$subsumes`/`$lookup`). `0` disables the
    /// cache. Bounded staleness against the remote server is the trade for
    /// not paying one HTTPS round trip per validated code — no openEHR spec
    /// governs terminology-server caching; our own design.
    pub cache_ttl_secs: u64,
    /// Maximum cached responses per provider.
    pub cache_capacity: u64,
}

impl Default for FhirProviderConfig {
    fn default() -> Self {
        Self {
            kind: ProviderKind::Fhir,
            url: String::new(),
            operation: FhirOperation::ValidateCode,
            connect_timeout_ms: 2_000,
            request_timeout_ms: 10_000,
            oauth2_client: None,
            client_cert_path: None,
            client_key_path: None,
            ca_bundle_path: None,
            cache_ttl_secs: 300,
            cache_capacity: 10_000,
        }
    }
}

impl ExternalTerminologyConfig {
    /// Build the named provider, with its `OAuth2` token source attached when
    /// one is configured. `None` when external terminology is disabled or no
    /// provider carries that name.
    ///
    /// # Errors
    ///
    /// [`SmError`] when the provider's configuration is invalid (empty URL, an
    /// un-buildable HTTP client) or its `oauth2_client` names a client that is
    /// missing or itself invalid.
    #[must_use]
    pub fn provider(&self, name: &str) -> Option<Result<FhirTerminologyProvider, SmError>> {
        if !self.enabled {
            return None;
        }
        let cfg = self.providers.get(name)?;
        Some(self.build_provider(name, cfg))
    }

    /// Build one provider from its config: the FHIR client plus, when
    /// `oauth2_client` is set, the client-credentials [`TokenSource`] whose
    /// bearer token every request carries.
    pub(super) fn build_provider(
        &self,
        name: &str,
        cfg: &FhirProviderConfig,
    ) -> Result<FhirTerminologyProvider, SmError> {
        let provider = FhirTerminologyProvider::new(name, cfg)?;
        let Some(client_name) = cfg.oauth2_client.as_deref() else {
            return Ok(provider);
        };
        let client_cfg = self.oauth2_clients.get(client_name).ok_or_else(|| {
            SmError::exception(format!(
                "terminology provider '{name}' names oauth2_client '{client_name}', which is not \
                 configured under [terminology.external.oauth2_clients]"
            ))
        })?;
        let token = TokenSource::new(client_name, client_cfg)?;
        Ok(provider.with_token(std::sync::Arc::new(token)))
    }
}

/// A provider config pointing at `url`, with defaults everywhere else and no
/// authentication — the shared fixture for this module's and
/// [`super::router`]'s tests.
#[cfg(test)]
pub(super) fn test_provider_config(url: &str) -> FhirProviderConfig {
    FhirProviderConfig {
        url: url.to_owned(),
        cache_capacity: 1024,
        ..FhirProviderConfig::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disabled_by_default() {
        let c = ExternalTerminologyConfig::default();
        assert!(!c.enabled);
        assert!(!c.fail_on_error);
        assert!(c.providers.is_empty());
        assert!(c.routes.is_empty());
        assert!(c.oauth2_clients.is_empty());
        assert!(c.provider(DEFAULT_PROVIDER_NAME).is_none());
    }

    #[test]
    fn a_configured_provider_materialises() {
        let providers = BTreeMap::from([(
            "default".to_owned(),
            test_provider_config("http://terminology:8090/fhir"),
        )]);
        let enabled = ExternalTerminologyConfig {
            enabled: true,
            providers: providers.clone(),
            ..ExternalTerminologyConfig::default()
        };
        assert!(enabled.provider("default").expect("selected").is_ok());
        // Disabled selects nothing even with a provider present.
        let disabled = ExternalTerminologyConfig {
            enabled: false,
            providers,
            ..ExternalTerminologyConfig::default()
        };
        assert!(disabled.provider("default").is_none());
    }

    #[test]
    fn empty_url_is_rejected_at_build() {
        let c = ExternalTerminologyConfig {
            enabled: true,
            providers: BTreeMap::from([("default".to_owned(), test_provider_config("   "))]),
            ..ExternalTerminologyConfig::default()
        };
        assert!(c.provider("default").expect("present").is_err());
    }

    /// A provider naming an `oauth2_client` that is not configured is a build
    /// error — never a silent unauthenticated request to the terminology
    /// server.
    #[test]
    fn unknown_oauth2_client_is_rejected_at_build() {
        let mut provider = test_provider_config("http://terminology:8090/fhir");
        provider.oauth2_client = Some("ts-client".to_owned());
        let c = ExternalTerminologyConfig {
            enabled: true,
            providers: BTreeMap::from([("default".to_owned(), provider)]),
            ..ExternalTerminologyConfig::default()
        };
        let err = c
            .provider("default")
            .expect("present")
            .expect_err("unknown oauth2_client must fail the build");
        assert!(err.message.contains("ts-client"), "got {}", err.message);
    }
}
