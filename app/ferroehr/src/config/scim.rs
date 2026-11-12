// SPDX-FileCopyrightText: Cadasto B.V.
// SPDX-License-Identifier: BUSL-1.1

//! `[auth.scim]`: the SCIM 2.0 service provider at `/scim/v2/` (RFC 7644) and
//! the provisioning clients it serves (`[[auth.scim.client]]`).
//!
//! A deployment chooses SCIM: it is off by default, and the table's presence
//! turns it on. Each client is an OAuth
//! client-credentials client of the deployment's own authorization server
//! (RFC 6749 §4.4), acting for nobody, holding the FerroEHR-defined `scope`
//! (SMART on openEHR `master08-scopes.adoc` defines none for SCIM), and owning
//! the Users whose binding key its `binding_issuer` issues. No openEHR spec
//! governs SCIM or its configuration: our own design.

use std::collections::BTreeSet;

use ferrohealth_config::units::Duration;
use ferrohealth_secret::secret_url::SecretUrl;
use ferrohealth_smart::config::Resolved;
use ferrohealth_smart::kind::IssuerKind;
use ferrohealth_smart::party::config::PartySection;
use serde::{Deserialize, Serialize};
use url::Url;

/// The longest `contact_erasure_delay` a deployment may configure: 30 days.
const MAX_CONTACT_ERASURE_DELAY: Duration = Duration::from_days(30);

/// The `[auth.scim]` section.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "infrastructure")))]
pub struct ScimConfig {
    /// The externally reachable origin the SCIM base URL hangs off, such as
    /// `https://cdr.example.com`; every `meta.location` and discovery document
    /// names `<public_base_url>/scim/v2` (RFC 7643 §3.1: `location` is "The
    /// URI of the resource being returned"). Required.
    pub public_base_url: Option<SecretUrl>,
    /// The most resources one list response returns (RFC 7643 §5, the
    /// `ServiceProviderConfig` `filter.maxResults`).
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
    pub filter_max_results: u64,
    /// The most Users one client deactivates or deletes in one window; past
    /// it, a deactivation or a deletion is refused with `429`.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
    pub max_deactivations: u32,
    /// The length of that window.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
    pub deactivation_window: Duration,
    /// How long after a User's deletion the contacts of every version of its
    /// `PERSON` are erased (GDPR Art. 17), at most 30 days; `"0s"` erases them
    /// in the deletion itself. The name, the binding key and the identifiers
    /// stay.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
    pub contact_erasure_delay: Duration,
    /// The most members one Group holds; an add past it is refused with
    /// `400`. Each membership change stores every membership of the Group
    /// again (RM demographic master02 §Versioning Semantics), so the cap bounds
    /// a Group's version storage.
    #[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "optional")))]
    pub max_group_members: u32,
    /// The provisioning clients.
    pub client: Vec<ScimClient>,
}

impl Default for ScimConfig {
    fn default() -> Self {
        Self {
            public_base_url: None,
            filter_max_results: 100,
            max_deactivations: 100,
            deactivation_window: Duration::from_hours(1),
            contact_erasure_delay: Duration::ZERO,
            max_group_members: 5_000,
            client: Vec::new(),
        }
    }
}

/// One `[[auth.scim.client]]` table.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schema", derive(schemars::JsonSchema))]
#[serde(default, deny_unknown_fields)]
#[cfg_attr(feature = "schema", schemars(extend("x-ferroehr-class" = "trust")))]
pub struct ScimClient {
    /// The issuer of the client's access tokens: the deployment's own
    /// authorization server, an `[[auth.issuer]]` of kind `machines`. It
    /// renders with any userinfo masked.
    pub issuer: SecretUrl,
    /// The client the token names (`client_id`, failing that `azp`).
    pub client_id: String,
    /// The scope a token of the client must hold.
    pub scope: String,
    /// The identifier issuer of the binding keys the client provisions, the
    /// `identifier_issuer` of an `[[auth.party]]` table; no two clients share
    /// one.
    pub binding_issuer: String,
}

impl ScimClient {
    /// The issuer the client's Groups' `externalId`s are written under: the
    /// `group_identifier_issuer` of the `[[auth.party]]` table whose
    /// `identifier_issuer` is the client's `binding_issuer`, and when that
    /// table sets none, the `binding_issuer` with `/groups` appended.
    #[must_use]
    pub fn group_issuer(&self, party: &[PartySection]) -> String {
        party
            .iter()
            .find(|table| table.identifier_issuer == self.binding_issuer)
            .map(|table| table.group_identifier_issuer.trim())
            .filter(|issuer| !issuer.is_empty())
            .map_or_else(
                || format!("{}/groups", self.binding_issuer.trim_end_matches('/')),
                str::to_owned,
            )
    }
}

impl ScimConfig {
    /// The SCIM base URL, `<public_base_url>/scim/v2`, when one is configured.
    #[must_use]
    pub fn base_url(&self) -> Option<String> {
        self.public_base_url
            .as_ref()
            .map(|origin| format!("{}/scim/v2", origin.expose().trim().trim_end_matches('/')))
    }

    /// The deactivation window in whole minutes, a part minute rounded down.
    #[must_use]
    pub fn window_minutes(&self) -> u32 {
        u32::try_from(
            self.deactivation_window
                .as_secs()
                .checked_div(60)
                .unwrap_or(0),
        )
        .unwrap_or(u32::MAX)
    }

    /// The contact erasure delay in whole minutes, a part minute rounded up,
    /// so a delay above `"0s"` never erases in the deletion itself.
    #[must_use]
    pub fn erasure_minutes(&self) -> u32 {
        let millis = self.contact_erasure_delay.as_millis();
        let minutes = millis.div_ceil(60_000);
        u32::try_from(minutes).unwrap_or(u32::MAX)
    }

    /// The `binding_issuer` of every client: the issuers whose deleted Users
    /// the contact erasure reaches.
    #[must_use]
    pub fn binding_issuers(&self) -> Vec<String> {
        self.client
            .iter()
            .map(|client| client.binding_issuer.clone())
            .collect()
    }

    /// The boot refusals of `[auth.scim]`, each naming its key; `resolved` is
    /// the resolved `[auth]` section, absent when it does not resolve.
    #[must_use]
    pub fn errors(&self, resolved: Option<&Resolved>) -> Vec<String> {
        let mut errors = Vec::new();
        let origin = self.public_base_url.as_ref().map(|url| url.expose().trim());
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
                "auth.scim.public_base_url (an absolute http(s) origin with no userinfo, query \
                 or fragment, e.g. 'https://cdr.example.com') is required: every meta.location \
                 names the resource by its URI (RFC 7643 §3.1)"
                    .to_owned(),
            );
        }
        if self.filter_max_results == 0 {
            errors.push("auth.scim.filter_max_results must be at least 1".to_owned());
        }
        if self.max_deactivations == 0 {
            errors.push("auth.scim.max_deactivations must be at least 1".to_owned());
        }
        if self.max_group_members == 0 {
            errors.push("auth.scim.max_group_members must be at least 1".to_owned());
        }
        if self.deactivation_window.as_millis() < 60_000 {
            errors.push("auth.scim.deactivation_window must be at least \"1min\"".to_owned());
        }
        if self.contact_erasure_delay > MAX_CONTACT_ERASURE_DELAY {
            errors.push(format!(
                "auth.scim.contact_erasure_delay must be at most \"{MAX_CONTACT_ERASURE_DELAY}\": \
                 a deleted User's contacts serve no attribution and are kept no longer than \
                 necessary (GDPR Art. 5(1)(e))"
            ));
        }
        if self.client.is_empty() {
            errors.push(
                "[auth.scim] needs at least one [[auth.scim.client]] table: the endpoint \
                 serves only declared provisioning clients"
                    .to_owned(),
            );
        }
        let mut clients = BTreeSet::new();
        let mut binding_issuers = BTreeSet::new();
        for client in &self.client {
            let name = format!(
                "the [[auth.scim.client]] table of {:?} and {:?}",
                client.issuer, client.client_id
            );
            for (key, value) in [
                ("issuer", client.issuer.expose()),
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
            if !clients.insert((client.issuer.expose(), client.client_id.as_str())) {
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
    match resolved.kinds.get(client.issuer.expose()) {
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
            issuer: SecretUrl::new(issuer),
            client_id: client_id.to_owned(),
            scope: "scim".to_owned(),
            binding_issuer: binding_issuer.to_owned(),
        }
    }

    fn enabled(clients: Vec<ScimClient>) -> ScimConfig {
        ScimConfig {
            public_base_url: Some(SecretUrl::new("https://cdr.example.com")),
            client: clients,
            ..ScimConfig::default()
        }
    }

    /// The group issuer is the table's `group_identifier_issuer` when it sets
    /// one, and otherwise the `binding_issuer` with `/groups` appended.
    #[test]
    fn a_client_whose_table_sets_no_group_issuer_derives_one() {
        let table = |identifier_issuer: &str, groups: &str| PartySection {
            identifier_issuer: identifier_issuer.to_owned(),
            group_identifier_issuer: groups.to_owned(),
            ..PartySection::default()
        };
        let login = client("https://as.example", "entra", "urn:example:login");
        assert_eq!(
            login.group_issuer(&[table("urn:example:login", "")]),
            "urn:example:login/groups"
        );
        assert_eq!(login.group_issuer(&[]), "urn:example:login/groups");
        assert_eq!(
            login.group_issuer(&[
                table("urn:example:other", "urn:example:other-groups"),
                table("urn:example:login", " urn:example:groups "),
            ]),
            "urn:example:groups",
            "a configured group issuer stays settable"
        );
        let url = client("https://as.example", "entra", "https://idp.example/login/");
        assert_eq!(url.group_issuer(&[]), "https://idp.example/login/groups");
    }

    #[test]
    fn off_by_default() {
        assert!(crate::config::FerroEhrConfig::default().auth.scim.is_none());
        assert_eq!(
            enabled(vec![client("https://as.example", "entra", "urn:a")]).errors(None),
            Vec::<String>::new()
        );
    }

    #[test]
    fn the_base_url_hangs_off_the_origin() {
        let config = ScimConfig {
            public_base_url: Some(SecretUrl::new("https://cdr.example.com/")),
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
            public_base_url: Some(SecretUrl::new("https://user:pw@cdr.example.com")),
            client: vec![ScimClient::default()],
            ..enabled(Vec::new())
        };
        let errors = config.errors(None);
        for needed in [
            "auth.scim.public_base_url",
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
                .any(|e| e.contains("at least one [[auth.scim.client]]"))
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

    #[test]
    fn a_contact_erasure_period_past_thirty_days_is_refused() {
        let config = ScimConfig {
            public_base_url: Some(SecretUrl::new("https://cdr.example.com")),
            contact_erasure_delay: Duration::from_millis(30 * 86_400_000 + 1),
            client: vec![client(
                "https://as.example",
                "provisioning",
                "urn:example:login",
            )],
            ..ScimConfig::default()
        };
        let refused = |config: &ScimConfig| {
            config
                .errors(None)
                .iter()
                .any(|error| error.contains("auth.scim.contact_erasure_delay"))
        };
        assert!(refused(&config), "{:?}", config.errors(None));
        let config = ScimConfig {
            contact_erasure_delay: Duration::from_days(30),
            ..config
        };
        assert!(!refused(&config), "{:?}", config.errors(None));
    }
}
