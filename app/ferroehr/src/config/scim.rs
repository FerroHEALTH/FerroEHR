// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[scim]`: the SCIM 2.0 service provider at `/scim/v2/` (RFC 7644) and the
//! provisioning clients it serves (`[[scim.client]]`).
//!
//! A deployment chooses SCIM: it is off by default. Each client is an OAuth
//! client-credentials client of the deployment's own authorization server
//! (RFC 6749 §4.4), acting for nobody, holding the FerroEHR-defined `scope`
//! (SMART on openEHR `master08-scopes.adoc` defines none for SCIM), and owning
//! the Users whose binding key its `binding_issuer` issues. No openEHR spec
//! governs SCIM or its configuration: our own design.

use std::collections::BTreeSet;

use ferrohealth_smart::config::Resolved;
use ferrohealth_smart::kind::IssuerKind;
use serde::{Deserialize, Serialize};
use url::Url;

/// The `[scim]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScimConfig {
    /// Whether `/scim/v2/` is served.
    pub enabled: bool,
    /// The externally reachable origin the SCIM base URL hangs off, such as
    /// `https://cdr.example.com`; every `meta.location` and discovery document
    /// names `<public_base_url>/scim/v2` (RFC 7643 §3.1: `location` is "The
    /// URI of the resource being returned"). Required when enabled.
    pub public_base_url: Option<String>,
    /// The most resources one list response returns (RFC 7643 §5,
    /// `filter.maxResults`).
    pub max_results: u64,
    /// The provisioning clients.
    pub client: Vec<ScimClient>,
}

impl Default for ScimConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            public_base_url: None,
            max_results: 100,
            client: Vec::new(),
        }
    }
}

/// One `[[scim.client]]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScimClient {
    /// The issuer of the client's access tokens: the deployment's own
    /// authorization server, an `[[auth.issuer]]` of kind `machines`.
    pub issuer: String,
    /// The client the token names (`client_id`, failing that `azp`).
    pub client_id: String,
    /// The scope a token of the client must hold.
    pub scope: String,
    /// The identifier issuer of the binding keys the client provisions, the
    /// `identifier_issuer` of an `[[auth.party]]` table; no two clients share
    /// one.
    pub binding_issuer: String,
}

impl ScimConfig {
    /// The SCIM base URL, `<public_base_url>/scim/v2`, when one is configured.
    #[must_use]
    pub fn base_url(&self) -> Option<String> {
        self.public_base_url
            .as_deref()
            .map(|origin| format!("{}/scim/v2", origin.trim().trim_end_matches('/')))
    }

    /// The boot refusals of `[scim]`, each naming its key; `resolved` is the
    /// resolved `[auth]` section, absent when it does not resolve.
    #[must_use]
    pub fn errors(&self, resolved: Option<&Resolved>) -> Vec<String> {
        if !self.enabled {
            return Vec::new();
        }
        let mut errors = Vec::new();
        let origin = self.public_base_url.as_deref().map(str::trim);
        let parsed = origin.and_then(|origin| Url::parse(origin).ok());
        let absolute = parsed.as_ref().is_some_and(|url| {
            matches!(url.scheme(), "http" | "https")
                && url.has_host()
                && url.username().is_empty()
                && url.password().is_none()
                && url.query().is_none()
                && url.fragment().is_none()
        });
        if !absolute {
            errors.push(
                "scim.public_base_url (an absolute http(s) origin with no userinfo, query or \
                 fragment, e.g. 'https://cdr.example.com') is required when scim.enabled = true: \
                 every meta.location names the resource by its URI (RFC 7643 §3.1)"
                    .to_owned(),
            );
        }
        if self.max_results == 0 {
            errors.push("scim.max_results must be at least 1".to_owned());
        }
        if self.client.is_empty() {
            errors.push(
                "scim.enabled = true needs at least one [[scim.client]] table: the endpoint \
                 serves only declared provisioning clients"
                    .to_owned(),
            );
        }
        let mut clients = BTreeSet::new();
        let mut binding_issuers = BTreeSet::new();
        for client in &self.client {
            let name = format!(
                "the [[scim.client]] table of {:?} and {:?}",
                client.issuer, client.client_id
            );
            for (key, value) in [
                ("issuer", &client.issuer),
                ("client_id", &client.client_id),
                ("scope", &client.scope),
                ("binding_issuer", &client.binding_issuer),
            ] {
                if value.trim().is_empty() {
                    errors.push(format!("{name} needs {key}"));
                }
            }
            if client.scope.split_whitespace().nth(1).is_some() {
                errors.push(format!("{name} has a scope of more than one token"));
            }
            if !clients.insert((client.issuer.as_str(), client.client_id.as_str())) {
                errors.push(format!("{name} is declared twice"));
            }
            if !client.binding_issuer.trim().is_empty()
                && !binding_issuers.insert(client.binding_issuer.as_str())
            {
                errors.push(format!(
                    "{name} shares binding_issuer {:?} with another client: each User belongs \
                     to the one client whose binding_issuer issues its key",
                    client.binding_issuer
                ));
            }
            if let Some(resolved) = resolved {
                errors.extend(issuer_errors(client, &name, resolved));
            }
        }
        errors
    }
}

/// The refusals of `client`'s issuers against the resolved `[auth]` section.
fn issuer_errors(client: &ScimClient, name: &str, resolved: &Resolved) -> Vec<String> {
    let mut errors = Vec::new();
    match resolved.kinds.get(&client.issuer) {
        Some(IssuerKind::Machines) => {}
        Some(kind) => errors.push(format!(
            "{name} names a {kind} issuer: a SCIM client acts for nobody, so its issuer is of \
             kind machines"
        )),
        None => errors.push(format!("{name} names no [[auth.issuer]]")),
    }
    let binds = resolved
        .party
        .iter()
        .any(|party| party.identifier_issuer == client.binding_issuer);
    if !client.binding_issuer.trim().is_empty() && !binds {
        errors.push(format!(
            "{name} has binding_issuer {:?}, which is the identifier_issuer of no [[auth.party]] \
             table",
            client.binding_issuer
        ));
    }
    errors
}

#[cfg(test)]
mod tests {
    use super::*;

    fn client(issuer: &str, client_id: &str, binding_issuer: &str) -> ScimClient {
        ScimClient {
            issuer: issuer.to_owned(),
            client_id: client_id.to_owned(),
            scope: "scim".to_owned(),
            binding_issuer: binding_issuer.to_owned(),
        }
    }

    fn enabled(clients: Vec<ScimClient>) -> ScimConfig {
        ScimConfig {
            enabled: true,
            public_base_url: Some("https://cdr.example.com".to_owned()),
            client: clients,
            ..ScimConfig::default()
        }
    }

    #[test]
    fn off_by_default_and_judged_only_when_enabled() {
        let config = ScimConfig::default();
        assert!(!config.enabled);
        assert_eq!(config.errors(None), Vec::<String>::new());
    }

    #[test]
    fn the_base_url_hangs_off_the_origin() {
        let config = ScimConfig {
            public_base_url: Some("https://cdr.example.com/".to_owned()),
            ..ScimConfig::default()
        };
        assert_eq!(
            config.base_url().as_deref(),
            Some("https://cdr.example.com/scim/v2")
        );
    }

    #[test]
    fn a_duplicate_binding_issuer_fails_boot() {
        let config = enabled(vec![
            client("https://as.example", "entra", "urn:login"),
            client("https://as.example", "okta", "urn:login"),
        ]);
        let errors = config.errors(None);
        assert!(
            errors
                .iter()
                .any(|e| e.contains("shares binding_issuer \"urn:login\"")),
            "{errors:?}"
        );
    }

    #[test]
    fn each_missing_key_and_a_bad_origin_are_named() {
        let config = ScimConfig {
            public_base_url: Some("https://user:pw@cdr.example.com".to_owned()),
            client: vec![ScimClient::default()],
            ..enabled(Vec::new())
        };
        let errors = config.errors(None);
        for needed in [
            "scim.public_base_url",
            "needs issuer",
            "needs client_id",
            "needs scope",
            "needs binding_issuer",
        ] {
            assert!(
                errors.iter().any(|e| e.contains(needed)),
                "{needed}: {errors:?}"
            );
        }
        let none = ScimConfig {
            client: Vec::new(),
            ..enabled(Vec::new())
        };
        assert!(
            none.errors(None)
                .iter()
                .any(|e| e.contains("at least one [[scim.client]]"))
        );
    }

    #[test]
    fn a_client_declared_twice_is_refused() {
        let config = enabled(vec![
            client("https://as.example", "entra", "urn:a"),
            client("https://as.example", "entra", "urn:b"),
        ]);
        assert!(
            config
                .errors(None)
                .iter()
                .any(|e| e.contains("declared twice"))
        );
    }
}
