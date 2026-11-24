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
//! `/.well-known/smart-configuration` document is served and where. Whether
//! an issuer's `patient/` grants count is its profile's `patient_context`
//! (`[auth.issuer.<name>.profile]`). Which clients count as backend
//! clients and which reach the DEMOGRAPHIC API is policy the CDR holds: the
//! `query_system_wide` and `access_demographic` grants of a client's `ACTOR`
//! (`crate::party_binding::grants`).

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

    /// Episode context (master09 §Experimental: Episode Context, whose
    /// semantics are "currently implementation-defined"): `episode_launch =
    /// true` adds `context-openehr-episode` to the document's capabilities and
    /// narrows every request of a launch to the token's `episodeId`, read as
    /// `issuer|id`: the session reads and queries that episode's data, the
    /// data placed in no episode and the data `ACCESS_POLICY.episode_shared`
    /// names (draft access control, ITS-REST, Queries and the Caseload).
    pub episode_launch: bool,
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
        assert!(c.validate().is_ok());
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
    fn an_unknown_key_is_refused() {
        let refused =
            toml::from_str::<SmartConfig>("colour = \"blue\"").expect_err("the key is unknown");
        assert!(
            refused.to_string().contains("unknown field `colour`"),
            "{refused}"
        );
    }
}
