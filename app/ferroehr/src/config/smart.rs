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
//! `/.well-known/smart-configuration` document is served and where, and
//! which issuers' `patient/` grants count. Which clients count as backend
//! clients and which reach the DEMOGRAPHIC API is policy the CDR holds: the
//! `query_system_wide` and `access_demographic` grants of a client's `ACTOR`
//! (`crate::party_binding::grants`).
//!
//! This is the `[auth.smart]` table of the one server configuration tree; it
//! carries no loader of its own. `patient_issuers` applies whether or not the
//! discovery document is served, so the table is always read and `discovery` alone
//! turns the document on.

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
    /// launch: `[auth] described_issuer`, else the one issuer that is not of
    /// kind `machines` and whose metadata is available. Several such issuers
    /// and no `described_issuer`, or none, refuse the start. master04
    /// §Services makes every `services.*.baseUrl` an "Absolute URL to the
    /// root of the API (required)", so the document needs `[server]
    /// public_base_url`.
    pub discovery: bool,

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

    /// Episode context (master09 §Experimental: Episode Context), advertised
    /// only: `episode_launch = true` adds `context-openehr-episode` to the
    /// document's capabilities and applies no episode filtering (openEHR has
    /// no first-class Episode resource; master09 calls the semantics
    /// "currently implementation-defined").
    pub episode_launch: bool,

    /// The issuers whose `patient/` grants count, each confined to the `ehrId`
    /// of the token's launch context (master07 §Context Selection: the `ehrId`
    /// names an EHR of the platform that issued it). Empty, a `patient/` grant
    /// grants nothing. Each renders with any userinfo masked.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
    pub patient_issuers: Vec<SecretUrl>,
}

impl SmartConfig {
    /// Boot validation.
    ///
    /// `platform_base_url` carries no `userinfo`, whether or not the document
    /// is served: it is published to every application.
    ///
    /// # Errors
    /// A message naming the offending key; it never quotes a credential.
    pub fn validate(&self) -> Result<(), String> {
        let Some(raw) = &self.platform_base_url else {
            return Ok(());
        };
        // NOTE: no openEHR spec governs this — our own design; a value that
        // does not parse as an absolute URL has no authority, so no userinfo.
        let Ok(url) = Url::parse(raw.expose().trim()) else {
            return Ok(());
        };
        if !url.username().is_empty() || url.password().is_some() {
            return Err(
                "auth.smart.platform_base_url must carry no userinfo: the discovery document \
                 publishes it to every application, so a credential in it would be served to \
                 anyone"
                    .to_owned(),
            );
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
        assert!(!c.episode_launch);
        assert_eq!(c.patient_issuers, Vec::<SecretUrl>::new());
        assert!(c.validate().is_ok());
    }

    /// The client lists are grants the CDR holds (#5437), so the keys that
    /// carried them are unknown keys.
    #[test]
    fn the_client_lists_are_unknown_keys() {
        for key in ["backend_clients", "demographic_clients"] {
            let err = toml::from_str::<SmartConfig>(&format!("{key} = [\"etl\"]"))
                .expect_err("a removed key is refused");
            assert!(err.to_string().contains("unknown field"), "{key}: {err}");
        }
    }

    /// A published URL carrying `userinfo` is refused whether or not the
    /// document is served, and the refusal never quotes the credential.
    #[test]
    fn userinfo_in_the_published_platform_url_is_refused() {
        let c = SmartConfig {
            platform_base_url: Some(SecretUrl::new("https://as:hunter2@as.example/x")),
            ..SmartConfig::default()
        };
        let err = c
            .validate()
            .expect_err("userinfo is refused while disabled");
        assert!(err.contains("must carry no userinfo"), "{err}");
        assert!(!err.contains("hunter2"), "{err}");
        // A bare-path platform base holds no authority.
        let c = SmartConfig {
            platform_base_url: Some(SecretUrl::new("/gateway/v1")),
            ..SmartConfig::default()
        };
        assert!(c.validate().is_ok());
    }

    #[test]
    fn the_table_takes_no_public_base_url() {
        let refused =
            toml::from_str::<SmartConfig>("public_base_url = \"https://cdr.example.com\"")
                .expect_err("the key is unknown");
        assert!(
            refused
                .to_string()
                .contains("unknown field `public_base_url`"),
            "{refused}"
        );
    }
}
