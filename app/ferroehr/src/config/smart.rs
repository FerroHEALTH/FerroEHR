// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! SMART on openEHR configuration
//! (`docs/specs/openehr/ITS-REST/docs/smart_app_launch/master04-service_discovery.adoc`,
//! `.../master07-*.adoc`, `.../master08-scopes.adoc`).
//!
//! The SMART on openEHR scopes are enforced on every route whenever
//! authentication is on: the route table in `ferroehr_rest::smart` declares a
//! requirement per operation, and the `ferrohealth-smart` crate checks it.
//! This section holds what FerroEHR decides beside that table: whether the
//! `/.well-known/smart-configuration` document is served and where, which
//! clients count as backend clients, which issuers' `patient/` grants count,
//! and which clients reach the DEMOGRAPHIC API.
//!
//! This is the `[smart]` section of the one server configuration tree; it
//! carries no loader of its own.

use serde::{Deserialize, Serialize};
use url::Url;

/// SMART on openEHR resource-server configuration.
///
/// The CDR is the SMART platform's `org.openehr.rest` resource server (master02
/// §Glossary). It never issues tokens, registers clients, or runs the OAuth 2.0
/// endpoints; those are the authorization server's.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SmartConfig {
    /// Serve the `/.well-known/smart-configuration` document (default
    /// `false`). The document is built from the first `[auth]` issuer's
    /// metadata: read from the issuer, or from [`Self::issuer_metadata_file`]
    /// when the issuer's key set is a `jwks_file`.
    pub enabled: bool,

    /// The RFC 8414 §2 authorization-server metadata of the first `[auth]`
    /// issuer when its key set is a `jwks_file`, which carries none: a JSON
    /// document naming that issuer, read at boot. SMART App Launch 2.2.0,
    /// Conformance, §Metadata requires the `token_endpoint` it names.
    pub issuer_metadata_file: Option<String>,

    /// The SMART platform base URL the discovery document hangs off (master04
    /// §Service Discovery: the document is served relative to the platform or
    /// gateway base, not the FHIR base). Unset, the REST root
    /// (`/ferroehr/rest`). A leading path segment is honoured, e.g.
    /// `/gateway/v1` → `/gateway/v1/.well-known/smart-configuration`.
    pub platform_base_url: Option<String>,

    /// The server's externally reachable origin (scheme + host [+ port]), e.g.
    /// `https://cdr.example.com`. master04 §Services makes every `services.*`
    /// entry's `baseUrl` an "Absolute URL to the root of the API (required)",
    /// so the document prefixes this origin onto the served base paths.
    /// Required when [`Self::enabled`] is set.
    pub public_base_url: Option<String>,

    /// Episode context (master09 §Experimental: Episode Context), advertised
    /// only: `episode.enabled = true` adds `context-openehr-episode` to the
    /// document's capabilities and applies no episode filtering (openEHR has
    /// no first-class Episode resource; master09 calls the semantics
    /// "currently implementation-defined").
    pub episode: EpisodeConfig,

    /// Advertise `launch-base64-json` (master09 §Launch Parameter as a Token).
    /// The base64 JSON `launch` object is the application's to consume; the
    /// CDR only advertises the capability. Default off.
    pub launch_base64_json: bool,

    /// The `client_id`s of the backend clients a `system/aql-*` grant counts
    /// for: master08 §Resource Scopes says such a grant "would grant access to
    /// all registered and ad-hoc AQL queries system-wide". Empty, no client.
    pub backend_clients: Vec<String>,

    /// The issuers whose `patient/` grants count, each confined to the `ehrId`
    /// of the token's launch context (master07 §Context Selection: the `ehrId`
    /// names an EHR of the platform that issued it). Empty, a `patient/` grant
    /// grants nothing.
    pub patient_issuers: Vec<String>,

    /// The `client_id`s that reach the DEMOGRAPHIC API. master08 §Resource
    /// Scopes defines no resource family for demographic data, so no SMART
    /// scope covers it: a listed client, or a caller holding the
    /// `[auth] product_scope`, reaches it (no openEHR spec governs this: our
    /// own design).
    pub demographic_clients: Vec<String>,
}

/// Episode-context sub-config (master09 §Experimental: Episode Context).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EpisodeConfig {
    /// Advertise episode context. Advisory only (no filtering).
    pub enabled: bool,
}

impl SmartConfig {
    /// Boot validation.
    ///
    /// No URL this section holds carries `userinfo`, whether or not the
    /// document is served: it is published to every application. When the
    /// document is served, `public_base_url` is an absolute `http(s)` origin
    /// (master04 §Services types `baseUrl` as an "Absolute URL … (required)").
    ///
    /// # Errors
    /// A message naming the offending key; it never quotes a credential.
    pub fn validate(&self) -> Result<(), String> {
        for (key, value) in [
            ("smart.platform_base_url", self.platform_base_url.as_ref()),
            ("smart.public_base_url", self.public_base_url.as_ref()),
        ] {
            let Some(raw) = value else { continue };
            // NOTE: no openEHR spec governs this — our own design; a value that
            // does not parse as an absolute URL has no authority, so no userinfo.
            let Ok(url) = Url::parse(raw.trim()) else {
                continue;
            };
            if !url.username().is_empty() || url.password().is_some() {
                return Err(format!(
                    "{key} must carry no userinfo: the discovery document publishes it to \
                     every application, so a credential in it would be served to anyone"
                ));
            }
        }
        if self.enabled {
            let origin = self.public_base_url.as_deref().unwrap_or("");
            let absolute = Url::parse(origin.trim())
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.has_host());
            if !absolute {
                return Err("smart.public_base_url (an absolute http(s) origin, e.g. \
                     'https://cdr.example.com') is required when smart.enabled = true — \
                     master04 §Services makes every services.*.baseUrl an absolute URL"
                    .to_owned());
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_off_and_list_nobody() {
        let c = SmartConfig::default();
        assert!(!c.enabled);
        assert!(c.platform_base_url.is_none());
        assert!(c.public_base_url.is_none());
        assert!(!c.episode.enabled);
        assert!(!c.launch_base64_json);
        assert!(c.backend_clients.is_empty());
        assert!(c.patient_issuers.is_empty());
        assert!(c.demographic_clients.is_empty());
        assert!(c.validate().is_ok());
    }

    /// A published URL carrying `userinfo` is refused whether or not the
    /// document is served, and the refusal never quotes the credential.
    #[test]
    fn userinfo_in_any_published_url_is_refused() {
        let leaky = "https://as:hunter2@as.example/x".to_owned();
        let setters: [fn(&mut SmartConfig, String); 2] = [
            |c, v| c.platform_base_url = Some(v),
            |c, v| c.public_base_url = Some(v),
        ];
        for set in setters {
            let mut c = SmartConfig::default();
            set(&mut c, leaky.clone());
            let err = c
                .validate()
                .expect_err("userinfo is refused while disabled");
            assert!(err.contains("must carry no userinfo"), "{err}");
            assert!(!err.contains("hunter2"), "{err}");
        }
        // A bare-path platform base holds no authority.
        let c = SmartConfig {
            platform_base_url: Some("/gateway/v1".to_owned()),
            ..SmartConfig::default()
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn a_served_document_requires_an_absolute_origin() {
        let mut c = SmartConfig {
            enabled: true,
            ..SmartConfig::default()
        };
        assert!(c.validate().is_err());
        c.public_base_url = Some("/ferroehr".to_owned());
        assert!(
            c.validate().is_err(),
            "a relative origin is no absolute URL"
        );
        c.public_base_url = Some("https://cdr.example.com".to_owned());
        assert!(c.validate().is_ok());
    }
}
