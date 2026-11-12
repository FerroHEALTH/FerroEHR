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
//! This is the `[auth.smart]` table of the one server configuration tree; it
//! carries no loader of its own. Its lists apply whether or not the discovery
//! document is served, so the table is always read and `discovery` alone
//! turns the document on.

use ferrohealth_config::removed::Removed;
use ferrohealth_secret::secret_url::SecretUrl;
use serde::{Deserialize, Serialize};
use url::Url;

/// SMART on openEHR resource-server configuration.
///
/// The CDR is the SMART platform's `org.openehr.rest` resource server (master02
/// §Glossary). It never issues tokens, registers clients, or runs the OAuth 2.0
/// endpoints; those are the authorization server's.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct SmartConfig {
    /// Serve the `/.well-known/smart-configuration` discovery document
    /// (SMART App Launch 2.x §Conformance; default `false`). The document is
    /// built from the metadata of one `[auth]` issuer that serves a user
    /// launch: [`Self::described_issuer`], else the one issuer that is not of
    /// kind `machines` and whose metadata is available. Several such issuers
    /// and no `described_issuer`, or none, refuse the start.
    pub discovery: bool,

    /// The `[auth]` issuer the discovery document describes: an issuer of kind
    /// `persons`, `patients` or `federation` whose metadata is read from it,
    /// introspected, or supplied by [`Self::issuer_metadata_file`]. A
    /// `machines` issuer signs client credentials tokens only, so it serves no
    /// user launch. Required when more than one issuer could be described. It
    /// renders with any userinfo masked.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub described_issuer: Option<SecretUrl>,

    /// The RFC 8414 §2 authorization-server metadata of the `[auth]` issuer
    /// it names, whose key set is a `jwks_file` and carries none: a JSON
    /// document read at boot. SMART App Launch 2.2.0,
    /// Conformance, §Metadata requires the `token_endpoint` it names.
    pub issuer_metadata_file: Option<String>,

    /// The SMART platform base URL the discovery document hangs off (master04
    /// §Service Discovery: the document is served relative to the platform or
    /// gateway base, not the FHIR base). Unset, the REST root
    /// (`/ferroehr/rest`). A leading path segment is honoured, e.g.
    /// `/gateway/v1` → `/gateway/v1/.well-known/smart-configuration`.
    pub platform_base_url: Option<SecretUrl>,

    /// The server's externally reachable origin (scheme + host [+ port]), e.g.
    /// `https://cdr.example.com`. master04 §Services makes every `services.*`
    /// entry's `baseUrl` an "Absolute URL to the root of the API (required)",
    /// so the document prefixes this origin onto the served base paths.
    /// Required when [`Self::discovery`] is set.
    pub public_base_url: Option<SecretUrl>,

    /// Episode context (master09 §Experimental: Episode Context), advertised
    /// only: `episode_launch = true` adds `context-openehr-episode` to the
    /// document's capabilities and applies no episode filtering (openEHR has
    /// no first-class Episode resource; master09 calls the semantics
    /// "currently implementation-defined").
    pub episode_launch: bool,

    /// The `client_id`s of the backend clients a `system/aql-*` grant counts
    /// for: master08 §Resource Scopes says such a grant "would grant access to
    /// all registered and ad-hoc AQL queries system-wide". Empty, no client.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub backend_clients: Vec<String>,

    /// The issuers whose `patient/` grants count, each confined to the `ehrId`
    /// of the token's launch context (master07 §Context Selection: the `ehrId`
    /// names an EHR of the platform that issued it). Empty, a `patient/` grant
    /// grants nothing. Each renders with any userinfo masked.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub patient_issuers: Vec<SecretUrl>,

    /// The `client_id`s that reach the DEMOGRAPHIC API. master08 §Resource
    /// Scopes defines no resource family for demographic data, so no SMART
    /// scope covers it: a listed client, or a caller holding the
    /// `[auth] product_scope`, reaches it (no openEHR spec governs this: our
    /// own design).
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub demographic_clients: Vec<String>,

    /// Retired in v5.7.0: refused at load, naming `discovery`.
    #[serde(skip_serializing)]
    pub enabled: Removed,
    /// Retired in v5.7.0: refused at load, naming `episode_launch`.
    #[serde(skip_serializing)]
    pub episode: Removed,
    /// Retired in v5.7.0: refused at load, naming `launch-base64-json` in
    /// `auth.capabilities`.
    #[serde(skip_serializing)]
    pub launch_base64_json: Removed,
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
            (
                "auth.smart.platform_base_url",
                self.platform_base_url.as_ref(),
            ),
            ("auth.smart.public_base_url", self.public_base_url.as_ref()),
        ] {
            let Some(raw) = value else { continue };
            // NOTE: no openEHR spec governs this — our own design; a value that
            // does not parse as an absolute URL has no authority, so no userinfo.
            let Ok(url) = Url::parse(raw.expose().trim()) else {
                continue;
            };
            if !url.username().is_empty() || url.password().is_some() {
                return Err(format!(
                    "{key} must carry no userinfo: the discovery document publishes it to \
                     every application, so a credential in it would be served to anyone"
                ));
            }
        }
        if self.discovery {
            let origin = self.public_base_url.as_ref().map_or("", SecretUrl::expose);
            let absolute = Url::parse(origin.trim())
                .is_ok_and(|url| matches!(url.scheme(), "http" | "https") && url.has_host());
            if !absolute {
                return Err(
                    "auth.smart.public_base_url (an absolute http(s) origin, e.g. \
                     'https://cdr.example.com') is required when auth.smart.discovery = true — \
                     master04 §Services makes every services.*.baseUrl an absolute URL"
                        .to_owned(),
                );
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
        assert!(!c.discovery);
        assert!(c.platform_base_url.is_none());
        assert!(c.public_base_url.is_none());
        assert!(!c.episode_launch);
        assert_eq!(c.backend_clients, Vec::<String>::new());
        assert_eq!(c.patient_issuers, Vec::<SecretUrl>::new());
        assert_eq!(c.demographic_clients, Vec::<String>::new());
        assert!(c.validate().is_ok());
    }

    /// A published URL carrying `userinfo` is refused whether or not the
    /// document is served, and the refusal never quotes the credential.
    #[test]
    fn userinfo_in_any_published_url_is_refused() {
        let leaky = SecretUrl::new("https://as:hunter2@as.example/x");
        let setters: [fn(&mut SmartConfig, SecretUrl); 2] = [
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
            platform_base_url: Some(SecretUrl::new("/gateway/v1")),
            ..SmartConfig::default()
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn a_served_document_requires_an_absolute_origin() {
        let mut c = SmartConfig {
            discovery: true,
            ..SmartConfig::default()
        };
        assert!(c.validate().is_err());
        c.public_base_url = Some(SecretUrl::new("/ferroehr"));
        assert!(
            c.validate().is_err(),
            "a relative origin is no absolute URL"
        );
        c.public_base_url = Some(SecretUrl::new("https://cdr.example.com"));
        assert!(c.validate().is_ok());
    }
}
